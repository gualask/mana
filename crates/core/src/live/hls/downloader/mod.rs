//! Following one playlist on its own thread: reloads, the segments not sent
//! yet with their sections and initialization sections, the switches to
//! another variant as the bandwidth changes, the stuck playlist that asks
//! for the opening to be opened again, and the retries every download of a
//! source goes through.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use super::download::Patience;
use super::ladder::Ladder;
use super::playlist::{MediaPlaylist, Segment, text};
use super::progress::{Progress, reload_wait, stuck};
use super::segment::{self, Segments};
use super::stream::{Chunk, POLL, Sender, Stop};
use crate::live::Interruption;
use crate::log::{debug, log};
use crate::net::{self, Client};
use failure::Failures;
pub(super) use failure::Failure;

mod failure;

const RETRY_DELAY: Duration = Duration::from_secs(1);
/// Attempts on one segment before it is skipped; playlists are retried for
/// as long as playback lasts, as a TV keeps trying a lost channel.
const SEGMENT_PATIENCE: Patience = Patience { attempts: 3, within: Duration::MAX };

/// A media playlist and when its download started.
pub(super) struct Fetched {
    pub(super) playlist: MediaPlaylist,
    pub(super) at: Instant,
}

pub(super) struct Downloader {
    client: Client,
    segments: Segments,
    /// What the log calls its failures: `hls`, `ts` or the source.
    label: &'static str,
    /// The variants to switch between, when following a master playlist's
    /// video.
    ladder: Option<Arc<Mutex<Ladder>>>,
}

impl Downloader {
    pub(super) fn new(client: Client, label: &'static str) -> Self {
        Self { client, segments: Segments::default(), label, ladder: None }
    }

    /// Follows the variant `ladder` chooses, measuring its segments.
    pub(super) fn switching(self, ladder: Arc<Mutex<Ladder>>) -> Self {
        Self { ladder: Some(ladder), ..self }
    }

    /// Follows `playlist_url`, starting from its `first` download when
    /// there is one, or, switching, the variant the ladder chooses: the new
    /// variant's playlist goes on from the same point in time (Progress).
    pub(super) fn run(
        &mut self,
        playlist_url: &Url,
        mut first: Option<Fetched>,
        sender: &Sender,
        stop: &Stop,
    ) -> Result<(), Failure> {
        let mut progress = Progress::new();
        let mut playlist_url = playlist_url.clone();
        loop {
            let Some(fetched) = self.next_playlist(&playlist_url, &mut first, stop)? else { return Ok(()) };
            let Fetched { playlist, at: fetched_at } = &fetched;
            let Some(sent) = self.send_new(playlist, *fetched_at, &mut progress, sender, stop)? else { return Ok(()) };
            if sent.added {
                progress.progressed = Instant::now();
            }
            if let Some(variant) = sent.switch {
                playlist_url = variant;
                continue;
            }
            if playlist.ended {
                return Ok(());
            }
            if !sent.added
                && stop.retunes
                && let Some(reason) = stuck(playlist, progress.progressed.elapsed(), progress.next)
            {
                stop.interrupt(Interruption::Stuck(reason));
                return Ok(());
            }
            if !wait_unless_stopped(stop, *fetched_at, reload_wait(playlist, sent.added)) {
                return Ok(());
            }
        }
    }

    /// The playlist at `url` to follow: its `first` download while there is
    /// one, else a new one; None once stopped.
    fn next_playlist(&mut self, url: &Url, first: &mut Option<Fetched>, stop: &Stop) -> Result<Option<Fetched>, Failure> {
        if let Some(fetched) = first.take() {
            return Ok(Some(fetched));
        }
        let at = Instant::now();
        Ok(self.retry(stop, None, |d| d.media_playlist(url))?.map(|playlist| Fetched { playlist, at }))
    }

    /// Sends the segments of `playlist`, fetched at `fetched_at`, not sent
    /// yet, up to a switch to another variant; None once the demuxer is
    /// gone or the opening stopped.
    fn send_new(
        &mut self,
        playlist: &MediaPlaylist,
        fetched_at: Instant,
        progress: &mut Progress,
        sender: &Sender,
        stop: &Stop,
    ) -> Result<Option<Batch>, Failure> {
        let from = progress.start(playlist, fetched_at);
        let mut batch = Batch { added: false, switch: None };
        for segment in playlist.segments.iter().filter(|s| s.sequence >= from) {
            if let Some(variant) = self.ladder.as_ref().and_then(|ladder| ladder.lock().unwrap().choose(Instant::now())) {
                progress.switch(playlist, fetched_at);
                batch.switch = Some(variant);
                break;
            }
            match self.send(segment, progress, sender, stop)? {
                Sent::Segment => batch.added = true,
                Sent::Skipped => {}
                Sent::Gone => return Ok(None),
            }
        }
        Ok(Some(batch))
    }

    /// Sends one segment, preceded by the start of a section and by its
    /// initialization section when they change. The first segment of
    /// another variant starts a section too, its streams demuxed afresh,
    /// with the same discontinuity number: its timestamps go on.
    fn send(&mut self, segment: &Segment, progress: &mut Progress, sender: &Sender, stop: &Stop) -> Result<Sent, Failure> {
        if !self.open_section(segment, progress, sender, stop)? {
            return Ok(Sent::Gone);
        }
        let started = Instant::now();
        let data = match self.retry(stop, Some(SEGMENT_PATIENCE), |d| d.fetch(segment)) {
            Ok(Some(data)) => data,
            Ok(None) => return Ok(Sent::Gone),
            // Even one this client cannot read: the next may be fine.
            Err(error) => {
                log!("hls: segment {} skipped: {}", segment.sequence, error.cause());
                progress.next = Some(segment.sequence + 1);
                return Ok(Sent::Skipped);
            }
        };
        // The opening waits for the whole first segment: what a slow start
        // spends, on a slow network, goes there.
        if !progress.sent_segment {
            log!("{}: first segment, {} KB in {} ms", self.label, data.len() / 1024, started.elapsed().as_millis());
        }
        if sender.send(Chunk::Bytes(data)).is_err() {
            return Ok(Sent::Gone); // the demuxer is gone
        }
        self.count_sent(segment, progress);
        Ok(Sent::Segment)
    }

    /// Starts a section ahead of `segment` when it starts one, with its
    /// initialization section when it changes or the section is new. False
    /// once the demuxer is gone or the opening stopped.
    fn open_section(&mut self, segment: &Segment, progress: &mut Progress, sender: &Sender, stop: &Stop) -> Result<bool, Failure> {
        let new_init = segment.init.is_some() && segment.init != progress.sent_init;
        let switched = std::mem::take(&mut progress.switched);
        let new_section = progress.sent_segment && (segment.discontinuity || new_init || switched);
        if (new_section || !progress.sent_segment) && sender.send(Chunk::Section(segment.discontinuity_sequence)).is_err() {
            return Ok(false);
        }
        // A new section is demuxed afresh: it needs the initialization
        // section again, even an unchanged one.
        let Some(init) = segment.init.as_ref().filter(|_| new_init || new_section) else { return Ok(true) };
        if !self.send_init(init, new_init && progress.sent_init.is_some(), segment, sender, stop)? {
            return Ok(false);
        }
        progress.sent_init = Some(init.clone());
        Ok(true)
    }

    /// Records `segment` as sent: following goes on after it.
    fn count_sent(&self, segment: &Segment, progress: &mut Progress) {
        progress.sent_segment = true;
        if segment.discontinuity {
            debug!("hls: discontinuity at segment {}", segment.sequence);
        }
        progress.next = Some(segment.sequence + 1);
    }

    /// Sends the initialization section `init` ahead of `segment`; `changed`
    /// when it replaces another. False once the demuxer is gone or the
    /// opening stopped.
    fn send_init(&mut self, init: &Url, changed: bool, segment: &Segment, sender: &Sender, stop: &Stop) -> Result<bool, Failure> {
        let Some(data) = self.retry(stop, None, |d| Ok(d.client.get(init, segment::MAX_BYTES)?))? else { return Ok(false) };
        if changed {
            log!("hls: new initialization section at segment {}", segment.sequence);
        }
        Ok(sender.send(Chunk::Bytes(data)).is_ok())
    }

    /// Retries `attempt` on failure, up to `attempts` times if given; returns
    /// None when stopped or interrupted.
    pub(super) fn retry<T>(
        &mut self,
        stop: &Stop,
        patience: Option<Patience>,
        mut attempt: impl FnMut(&mut Self) -> Result<T, Failure>,
    ) -> Result<Option<T>, Failure> {
        let mut failures = Failures::new(patience, self.label);
        loop {
            if stop.is_set() {
                return Ok(None);
            }
            match attempt(self) {
                Ok(value) => {
                    failures.recovered();
                    return Ok(Some(value));
                }
                Err(Failure::Unsupported(error)) => return Err(Failure::Unsupported(error)),
                // A token refused (expired, as RaiPlay's after 24 h): only a
                // new resolution of the source gives new URLs.
                Err(Failure::Refused(error)) if stop.retunes => {
                    stop.interrupt(Interruption::Refused(error));
                    return Ok(None);
                }
                // While resolving, the session decides whether to resolve
                // again.
                Err(Failure::Refused(error)) => return Err(Failure::Refused(error)),
                Err(failure) => {
                    failures.count(failure.cause())?;
                    std::thread::sleep(RETRY_DELAY);
                }
            }
        }
    }

    pub(super) fn client(&self) -> &Client {
        &self.client
    }
}

impl Downloader {
    fn media_playlist(&mut self, url: &Url) -> Result<MediaPlaylist, Failure> {
        let (served, body) = self.client.get_with_final_url(url, net::MAX_TEXT_BYTES)?;
        Ok(MediaPlaylist::parse(&text(body)?, &served)?)
    }

    /// Downloads one segment, measuring the bandwidth for the ladder.
    fn fetch(&mut self, segment: &Segment) -> Result<Vec<u8>, Failure> {
        let started = Instant::now();
        let data = self.segments.fetch(&self.client, segment)?;
        if let Some(ladder) = &self.ladder {
            ladder.lock().unwrap().measure(data.len(), started.elapsed());
        }
        Ok(data)
    }
}

/// What one pass over a playlist did.
struct Batch {
    /// Whether a segment was sent.
    added: bool,
    /// The variant to go on with, chosen before the next segment.
    switch: Option<Url>,
}

/// How sending one segment ended.
enum Sent {
    Segment,
    /// It could not be read, and the next one follows.
    Skipped,
    /// The demuxer is gone or the opening stopped.
    Gone,
}

/// Waits until `wait` has passed since `since`; false when stopped first.
fn wait_unless_stopped(stop: &Stop, since: Instant, wait: Duration) -> bool {
    while since.elapsed() < wait {
        if stop.is_set() {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}
