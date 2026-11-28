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

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use super::continuous::{self, Body};
use super::downloader::{Downloader, Failure, Fetched};
use super::ladder::Ladder;
use super::playlist::{self, parse_master, text};
use super::ts;
use super::stream::{self, HlsStream, Sender, Stop};
use crate::live::playable::Playable;
use crate::live::{Cause, Interruption, QualityChoice, Stats};
use crate::log::log;
use crate::net::{self, Client};

/// Downloaded segments waiting for the demuxer, each of at most
/// segment::MAX_BYTES.
const QUEUED_SEGMENTS: usize = 3;
/// Bytes of them waiting past the first: a 4K channel's 10 s segments,
/// 31 MB at 25 Mbit/s, wait one at a time, 6 s ones two at a time, while
/// those of Rai 1 and Nove (3–6 MB) never reach it. hls.js stops loading
/// past 60 MB ahead (`maxBufferSize`), Media3 past 125 MiB of video
/// (`DefaultLoadControl.DEFAULT_VIDEO_BUFFER_SIZE`); a B1 has about
/// 440 MB free.
const QUEUED_BYTES: usize = 48 * 1024 * 1024;

/// The byte streams of a stream to play: video (with muxed audio, if any) and an
/// audio rendition of its own, when the variant has one.
pub struct Streams {
    pub main: HlsStream,
    pub audio: Option<HlsStream>,
}

/// A live stream with its variants: their media playlists can be opened
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
    /// playlist. When the stream's URL served a media playlist, the first
    /// opening starts from that answer, which needs no request of its own.
    Playlists { ladder: Arc<Mutex<Ladder>>, audio: Option<Url>, first: Mutex<Option<Fetched>> },
    /// The stream's URL, as the stream to play gives it: a redirect may lead to an
    /// address whose token expires. The answer that told it apart goes to
    /// the first opening, which needs no request of its own.
    Continuous { url: Url, first: Mutex<Option<Body>> },
}

/// How long opening a stream insists before giving up: `attempts` network
/// failures, or failures for `within` since the first attempt, whichever
/// comes first, so that a server that never answers does not hold the
/// screen for a read timeout per attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patience {
    pub attempts: u32,
    pub within: Duration,
}

impl Live {
    /// Tells what the stream's URL serves:
    /// a master playlist, whose variants and audio rendition it chooses,
    /// following the viewer's `choice` of quality, or a continuous stream.
    /// Blocks until then, retrying network errors within `patience` when
    /// given; returns None when stopped meanwhile. A refused token is an interruption of its own:
    /// the session may resolve the stream again. The stream's credentials
    /// go only to the origin of its URL.
    pub fn resolve(
        playable: &Playable,
        stop: &Stop,
        stats: Arc<Mutex<Stats>>,
        choice: &QualityChoice,
        patience: Option<Patience>,
    ) -> Result<Option<Self>, Interruption> {
        let named = [playable.url.clone()];
        let client = Client::for_source(playable.headers.clone(), &named, playable.root_certificate.as_deref())
            .map_err(|e| Interruption::Failed(Cause::Client(e)))?;
        let mut reader = Downloader::new(client.clone(), "stream");
        let master = &playable.url;
        let served = |d: &mut Downloader| served(d.client(), master, choice, &stats);
        let Some(media) = reader.retry(stop, patience, served).map_err(interruption)? else {
            return Ok(None);
        };
        Ok(Some(Self { client, media }))
    }

    /// Starts downloading the video variant and its audio rendition, from
    /// the live edge, or the continuous stream, until `stop`; fails only
    /// when a download thread cannot start.
    pub fn open(&self, stop: &Stop) -> std::io::Result<Streams> {
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

/// Why resolving a stream failed, as the session tells the failures apart.
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
) -> std::io::Result<HlsStream> {
    let (sender, stream) = stream::queue(QUEUED_SEGMENTS, QUEUED_BYTES, stop.clone());
    let mut downloader = Downloader::new(client.clone(), "hls");
    if let Some(ladder) = ladder {
        downloader = downloader.switching(ladder.clone());
    }
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("hls".into())
        .spawn(move || download(&mut downloader, &playlist, first, &sender, &thread_stop))?;
    Ok(stream)
}

/// Follows `playlist` with `downloader` to its end; a failure interrupts
/// the opening.
fn download(downloader: &mut Downloader, playlist: &Url, first: Option<Fetched>, sender: &Sender, stop: &Stop) {
    let interruption = match downloader.run(playlist, first, sender, stop) {
        Ok(()) => return,
        Err(Failure::Unsupported(error)) => Interruption::Unsupported(error),
        Err(Failure::Transient(error) | Failure::Refused(error)) => Interruption::Failed(error),
    };
    log!("hls: {interruption}");
    stop.interrupt(interruption);
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
        log!("ts: continuous MPEG-TS stream");
        let first = Mutex::new(Some(continuous::rejoin(head, body)));
        return Ok(Media::Continuous { url: master.clone(), first });
    }
    // Text would have no NUL: an endless answer in another format must not
    // be read up to the bound.
    if !playlist::is_playlist(&head) && head.contains(&0) {
        return Err(playlist::Error::Unsupported(playlist::Unsupported::OtherFormat).into());
    }
    let mut data = head;
    let rest = net::MAX_TEXT_BYTES.saturating_sub(data.len() as u64);
    data.extend(net::read_rest(master, body, rest)?);
    let variants = parse_master(&text(data)?, &served)?;
    // A media playlist: the first opening follows it from this answer.
    let first = variants.media.map(|playlist| Fetched { playlist, at });
    if variants.audio.is_some() {
        log!("hls: audio in a rendition of its own");
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
