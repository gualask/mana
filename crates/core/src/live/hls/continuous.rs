//! A live stream served as one MPEG-TS answer that never ends, instead of
//! a playlist, as IPTV servers often do: read as it arrives, and asked for
//! again when the answer breaks off. Each answer is a section of its own:
//! the demuxer starts afresh on it, as at a discontinuity.

use std::io::{Cursor, Read};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};

use url::Url;

use super::downloader::{Downloader, Failure};
use super::stream::{Chunk, HlsStream, Stop};
use super::ts;
use crate::live::Interruption;
use crate::log::log;
use crate::net::{self, Client};

/// Bytes handed to the demuxer at a time: a fraction of a second of a
/// channel, so that the queue below is bounded in time, not only in count.
const CHUNK: usize = 64 * 1024;
/// Chunks waiting for the demuxer, 8 MB: the start margin even at
/// 20 Mbit/s.
const QUEUED_CHUNKS: usize = 128;
/// How much of each answer is gathered before the demuxer gets any: a
/// margin against the network slowing down, as an HLS live stream has in
/// the segments it starts behind the live edge. Media3 starts with 2.5 s;
/// with the 1.5 s the feeder sends ahead this is 4.5 s, near the 6 s of
/// the lab's HLS channels.
const START_MARGIN: Duration = Duration::from_secs(3);
/// Bytes dropped at most while waiting for the program tables: about 8 s
/// of a 4 Mbit/s channel, longer than a segment of an HLS restream.
const MAX_BEFORE_TABLES: usize = 4 * 1024 * 1024;

/// An answer being read.
pub(super) type Body = Box<dyn Read + Send>;

/// Up to `count` bytes of `body`, fewer only at its end.
pub(super) fn read_up_to(body: &mut impl Read, count: usize) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(count);
    body.take(count as u64).read_to_end(&mut data)?;
    Ok(data)
}

/// An answer whose `head` was read already, with the rest as it goes on.
pub(super) fn rejoin(head: Vec<u8>, body: Body) -> Body {
    Box::new(Cursor::new(head).chain(body))
}

/// Starts reading the stream at `url` on its own thread and returns its
/// byte stream: `first` is the answer that told it apart, if not read yet.
pub(super) fn follow(
    client: &Client,
    url: Url,
    first: Option<Body>,
    stop: &Stop,
) -> Result<HlsStream, String> {
    let (sender, queue) = sync_channel(QUEUED_CHUNKS);
    let mut downloader = Downloader::new(client.clone(), "ts");
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("ts".into())
        .spawn(move || {
            let Err(failure) = read(&mut downloader, &url, first, &sender, &thread_stop) else { return };
            let interruption = match failure {
                Failure::Unsupported(error) => Interruption::Unsupported(error),
                Failure::Transient(error) | Failure::Refused(error) => Interruption::Failed(error),
            };
            log!("ts: {interruption}");
            thread_stop.interrupt(interruption);
        })
        .map_err(|e| e.to_string())?;
    Ok(HlsStream::new(queue, stop.clone()))
}

/// Reads one answer after another, each as a section, until stopped or
/// the demuxer is gone.
fn read(
    downloader: &mut Downloader,
    url: &Url,
    mut first: Option<Body>,
    sender: &SyncSender<Chunk>,
    stop: &Stop,
) -> Result<(), Failure> {
    for section in 0.. {
        let answer = match first.take() {
            Some(body) => Some(body),
            None => connect(downloader, url, stop)?,
        };
        let Some(body) = answer else { return Ok(()) };
        if section > 0 {
            log!("ts: di nuovo connesso, sezione {section}");
        }
        if sender.send(Chunk::Section(section)).is_err() {
            return Ok(());
        }
        let Some(reason) = pass_on(body, url, sender, stop) else { return Ok(()) };
        log!("ts: {reason}, nuova richiesta");
    }
    Ok(())
}

/// A new answer, asked for until one comes, as playlists are: None once
/// stopped, or interrupted by a refused token. An answer with nothing in
/// it counts as a failure, so that a server closing at once is asked
/// again only after a delay.
fn connect(downloader: &mut Downloader, url: &Url, stop: &Stop) -> Result<Option<Body>, Failure> {
    downloader.retry(stop, None, |d| {
        let (_, mut body) = d.client().open(url)?;
        let head = read_up_to(&mut body, 1).map_err(|e| net::read_error(url, &e))?;
        if head.is_empty() {
            return Err(Failure::Transient(format!("{}: risposta vuota", net::loggable_path(url))));
        }
        Ok(rejoin(head, body))
    })
}

/// Hands `body` to the demuxer chunk by chunk, from its program tables on
/// and after its start margin: why the answer broke off, or None once
/// stopped or the demuxer is gone.
fn pass_on(mut body: Body, url: &Url, sender: &SyncSender<Chunk>, stop: &Stop) -> Option<String> {
    let (mut tables, mut margin) = (TablesFirst::default(), Margin::default());
    while !stop.is_set() {
        let (mut chunk, broken) = match read_up_to(&mut body, CHUNK) {
            Ok(chunk) if chunk.len() == CHUNK => (chunk, None),
            Ok(chunk) => (chunk, Some("risposta chiusa dal server".to_string())),
            // What came before the error is lost with the rest of the
            // chunk: the next section starts afresh anyway.
            Err(error) => (Vec::new(), Some(net::read_error(url, &error).into())),
        };
        tables.trim(&mut chunk);
        let ready = margin.pass(chunk, broken.is_some());
        if !ready.into_iter().all(|chunk| sender.send(Chunk::Bytes(chunk)).is_ok()) {
            return None;
        }
        if broken.is_some() {
            return broken;
        }
    }
    None
}

/// Drops what an answer starts with before its first program association
/// table, as Media3's TsExtractor ignores packets until it has the tables:
/// the demuxer's short analysis would otherwise miss a stream whose tables
/// come late, as a restream of HLS segments sends them once per segment,
/// and the decoder would start between keyframes.
#[derive(Default)]
struct TablesFirst {
    dropped: usize,
    /// Once the tables were found, or given up.
    done: bool,
}

impl TablesFirst {
    fn trim(&mut self, chunk: &mut Vec<u8>) {
        if self.done {
            return;
        }
        let found = ts::tables_start(chunk);
        self.done = found.is_some() || self.dropped + chunk.len() > MAX_BEFORE_TABLES;
        let cut = match found {
            Some(at) => at,
            None if self.done => 0,
            None => chunk.len(),
        };
        chunk.drain(..cut);
        self.dropped += cut;
        if self.done && self.dropped > 0 {
            let outcome = if found.is_some() { "fino alle tabelle del programma" } else { "senza trovare le tabelle del programma" };
            log!("ts: scartati {} KB {outcome}", self.dropped / 1024);
        }
    }
}

/// Holds the start of an answer until START_MARGIN has passed since its
/// first useful bytes, as Media3 waits for a buffer before playing, or
/// until it holds as much as the queue does: a server sending in bursts
/// must not fill the memory meanwhile.
#[derive(Default)]
struct Margin {
    since: Option<Instant>,
    held: Vec<Vec<u8>>,
    held_bytes: usize,
    gathered: bool,
}

impl Margin {
    /// What to hand on now, given the next `chunk`: nothing while holding,
    /// everything held once the margin is gathered or the answer is at its
    /// `last` chunk.
    fn pass(&mut self, chunk: Vec<u8>, last: bool) -> Vec<Vec<u8>> {
        if !chunk.is_empty() {
            self.since.get_or_insert_with(Instant::now);
            self.held_bytes += chunk.len();
            self.held.push(chunk);
        }
        let full = self.held_bytes >= QUEUED_CHUNKS * CHUNK;
        let ready = self.gathered || last || full || self.since.is_some_and(|since| since.elapsed() >= START_MARGIN);
        if ready && !self.gathered {
            self.gathered = true;
            log!("ts: margine raccolto, {} KB", self.held_bytes / 1024);
        }
        if ready { std::mem::take(&mut self.held) } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests;
