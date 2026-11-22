//! Downloads a live HLS stream: master/media playlists, live reloads, the
//! variant switched to as the bandwidth changes (ladder/), AES-128
//! segments, fixed request headers, MPEG-TS, fMP4 (EXT-X-MAP) or
//! packed-audio segments and an audio rendition of its own (EXT-X-MEDIA);
//! or a continuous MPEG-TS stream served in place of a playlist
//! (continuous.rs). Each playlist or stream is downloaded on its own thread
//! (downloader/) and handed to a demuxer as a byte stream through a
//! bounded queue (stream.rs), which also paces the downloads. The stream is
//! cut into sections at each discontinuity and new initialization section,
//! where streams and formats may change: each section is demuxed afresh.

use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use url::Url;

use super::continuous::{self, Body};
use super::downloader::{Downloader, Failure, Fetched};
use super::ladder::Ladder;
use super::playlist::{self, parse_master, text};
use super::ts;
use super::stream::{Chunk, HlsStream, Stop};
use crate::live::recipe;
use crate::live::source::Source;
use crate::live::{Interruption, QualityChoice, Stats};
use crate::log::log;
use crate::net::{self, Client, loggable};

/// Downloaded segments waiting for the demuxer, each of at most
/// segment::MAX_BYTES.
const QUEUED_SEGMENTS: usize = 3;

/// The byte streams of a source: video (with muxed audio, if any) and an
/// audio rendition of its own, when the variant has one.
pub struct Streams {
    pub main: HlsStream,
    pub audio: Option<HlsStream>,
}

/// A live source with its variants: their media playlists can be opened
/// again, from the live edge, without the master playlist, whose token may
/// have expired meanwhile (RaiPlay's lasts 150 s, its media playlists' 24 h);
/// a new opening starts on the variant followed last. Or a continuous
/// stream, asked for again at each opening.
pub struct Live {
    client: Client,
    media: Media,
}

enum Media {
    /// The video variants, and their audio rendition when it has its own
    /// playlist. When the source's URL served a media playlist, the first
    /// opening starts from that answer, which needs no request of its own.
    Playlists { ladder: Arc<Mutex<Ladder>>, audio: Option<Url>, first: Mutex<Option<Fetched>> },
    /// The stream's URL, as the source gives it: a redirect may lead to an
    /// address whose token expires. The answer that told it apart goes to
    /// the first opening, which needs no request of its own.
    Continuous { url: Url, first: Mutex<Option<Body>> },
}

impl Live {
    /// Resolves the source's recipe, if any, and tells what its URL serves:
    /// a master playlist, whose variants and audio rendition it chooses,
    /// following the viewer's `choice` of quality, or a continuous stream.
    /// Blocks until then, retrying network errors; returns None when
    /// stopped meanwhile. A refused token is an interruption of its own:
    /// the session may resolve the source again.
    pub fn resolve(
        source: &Source,
        stop: &Stop,
        stats: Arc<Mutex<Stats>>,
        choice: &QualityChoice,
    ) -> Result<Option<Self>, Interruption> {
        let failed = |e: String| Interruption::Failed(e);
        let named = source.named_urls().map_err(failed)?;
        let client = Client::for_source(source.headers.clone(), &named, source.root_certificate.as_deref()).map_err(failed)?;
        let mut reader = Downloader::new(client.clone(), "fonte");
        let Some(master) = master_url(source, &mut reader, stop)? else { return Ok(None) };
        let served = |d: &mut Downloader| served(d.client(), &master, choice, &stats);
        let Some(media) = reader.retry(stop, None, served).map_err(interruption)? else {
            return Ok(None);
        };
        Ok(Some(Self { client, media }))
    }

    /// Starts downloading the video variant and its audio rendition, from
    /// the live edge, or the continuous stream, until `stop`.
    pub fn open(&self, stop: &Stop) -> Result<Streams, String> {
        match &self.media {
            Media::Playlists { ladder, audio, first } => {
                let video = ladder.lock().unwrap().current().clone();
                let first = first.lock().unwrap().take();
                let main = follow(&self.client, video, first, Some(ladder), stop)?;
                let audio = audio.clone().map(|url| follow(&self.client, url, None, None, stop)).transpose()?;
                Ok(Streams { main, audio })
            }
            Media::Continuous { url, first } => {
                let first = first.lock().unwrap().take();
                let main = continuous::follow(&self.client, url.clone(), first, stop)?;
                Ok(Streams { main, audio: None })
            }
        }
    }
}

/// The URL `source` gives, or its recipe ends on; None when stopped.
fn master_url(source: &Source, reader: &mut Downloader, stop: &Stop) -> Result<Option<Url>, Interruption> {
    if let Some(url) = &source.url {
        return Url::parse(url).map(Some).map_err(|e| Interruption::Failed(e.to_string()));
    }
    let master = run_recipe(reader, &source.resolve, stop).map_err(interruption)?;
    if let Some(master) = &master {
        log!("fonte risolta: {}", loggable(master));
    }
    Ok(master)
}

/// Why resolving a source failed, as the session tells the failures apart.
fn interruption(failure: Failure) -> Interruption {
    match failure {
        Failure::Refused(error) => Interruption::Refused(error),
        Failure::Unsupported(error) => Interruption::Unsupported(error),
        Failure::Transient(error) => Interruption::Failed(error),
    }
}

/// Starts downloading one media playlist, from its `first` download if
/// any, or the variants of `ladder` starting with it, and returns its byte
/// stream.
fn follow(
    client: &Client,
    playlist: Url,
    first: Option<Fetched>,
    ladder: Option<&Arc<Mutex<Ladder>>>,
    stop: &Stop,
) -> Result<HlsStream, String> {
    let (sender, queue) = sync_channel(QUEUED_SEGMENTS);
    let mut downloader = Downloader::new(client.clone(), "hls");
    if let Some(ladder) = ladder {
        downloader = downloader.switching(ladder.clone());
    }
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("hls".into())
        .spawn(move || download(&mut downloader, &playlist, first, &sender, &thread_stop))
        .map_err(|e| e.to_string())?;
    Ok(HlsStream::new(queue, stop.clone()))
}

/// Follows `playlist` with `downloader` to its end; a failure interrupts
/// the opening.
fn download(downloader: &mut Downloader, playlist: &Url, first: Option<Fetched>, sender: &SyncSender<Chunk>, stop: &Stop) {
    let interruption = match downloader.run(playlist, first, sender, stop) {
        Ok(()) => return,
        Err(Failure::Unsupported(error)) => Interruption::Unsupported(error),
        Err(Failure::Transient(error) | Failure::Refused(error)) => Interruption::Failed(error),
    };
    log!("hls: {interruption}");
    stop.interrupt(interruption);
}

/// The URL the recipe `steps` end on. Network errors are retried as for
/// playlists; a recipe that does not fit the answers is an error of its
/// own. None when stopped.
fn run_recipe(reader: &mut Downloader, steps: &[recipe::Step], stop: &Stop) -> Result<Option<Url>, Failure> {
    reader.retry(stop, None, |d| {
        let mut network = None;
        recipe::run(steps, |url| {
            d.client().get(url, playlist::MAX_BYTES).map_err(|e| {
                let message = e.to_string();
                network = Some(e);
                message
            })
        })
        .map_err(|e| match network.take() {
            Some(network) if network.is_refused() => Failure::Refused(e),
            Some(_) => Failure::Transient(e),
            None => Failure::Unsupported(e),
        })
    })
}

/// What `master` serves, told by its first bytes. A playlist gives the
/// media playlists to follow: the variants of a master playlist, to switch
/// between as `choice` asks, and the audio rendition their AUDIO group
/// points to, if that has its own URI; or `master` itself when it is a
/// media playlist, already read.
fn served(client: &Client, master: &Url, choice: &QualityChoice, stats: &Arc<Mutex<Stats>>) -> Result<Media, Failure> {
    let at = Instant::now();
    let (served, mut body) = client.open(master)?;
    let head = continuous::read_up_to(&mut body, ts::HEAD).map_err(|e| net::read_error(master, &e))?;
    if is_continuous(&head) {
        log!("ts: flusso MPEG-TS continuo");
        let first = Mutex::new(Some(continuous::rejoin(head, body)));
        return Ok(Media::Continuous { url: master.clone(), first });
    }
    // Text would have no NUL: an endless answer in another format must not
    // be read up to the bound.
    if !playlist::is_playlist(&head) && head.contains(&0) {
        return Err(Failure::Unsupported("formato diverso da HLS e MPEG-TS".into()));
    }
    let mut data = head;
    let rest = playlist::MAX_BYTES.saturating_sub(data.len() as u64);
    data.extend(net::read_rest(master, body, rest)?);
    let variants = parse_master(&text(data)?, &served)?;
    // A media playlist: the first opening follows it from this answer.
    let first = variants.media.map(|playlist| Fetched { playlist, at });
    if variants.audio.is_some() {
        log!("hls: audio in una traccia separata");
    }
    let ladder = Ladder::new(variants.video, choice.clone(), stats.clone());
    Ok(Media::Playlists { ladder: Arc::new(Mutex::new(ladder)), audio: variants.audio, first: Mutex::new(first) })
}

/// Whether an answer starting with `head` is a continuous stream: MPEG-TS,
/// and not a playlist whose text has a `G` (0x47) in the same places.
fn is_continuous(head: &[u8]) -> bool {
    !playlist::is_playlist(head) && ts::is_transport_stream(head)
}

#[cfg(test)]
mod tests;
