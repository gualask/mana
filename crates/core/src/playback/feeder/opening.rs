//! Plays one opening of a source: what the reader hands over goes to the
//! feeder, a section that needs another decoder loads one, and a live
//! source starved too long asks to be opened again.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{Feeder, MAX_WAIT_SLICE, show_formats};
use crate::log::log;
use crate::playback::demux::{Formats, Packet};
use crate::playback::reader::{Demuxed, Reader};
use crate::playback::{Error, OpenBackend, Phase, Settings, Stats};

/// Nothing to feed for this long: a live source is opened again from the
/// live edge, as a TV tunes in again after losing the signal, rather than
/// going on as far behind the live edge as the gap lasted.
const RETUNE_AFTER: Duration = Duration::from_secs(10);

/// How playing one opening of a source ended.
pub enum Ended {
    /// Stopped, or the end of the source.
    Done,
    /// A live source starved for RETUNE_AFTER, now with data again.
    Retune(Duration),
}

/// Plays what `reader` reads until the end, a stop or, live, a retune.
pub fn play(
    reader: &Reader,
    live: bool,
    open: &OpenBackend,
    settings: &Settings,
    stop: &AtomicBool,
    stats: &Mutex<Stats>,
) -> Result<Ended, Error> {
    let (first, description) = &reader.formats;
    log!("source: {description}");
    let mut playing = Playing::start(Load { open, settings, stop, stats }, *first, reader.without_audio)?;
    let mut freeze_test = FreezeTest::new(settings);
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Ended::Done);
        }
        let Some(next) = reader.next(MAX_WAIT_SLICE)? else {
            playing.feeder.pause_if_running_dry();
            continue;
        };
        freeze_test.run(&mut playing.feeder);
        if let Some(idle) = starved(&playing.feeder, live) {
            return Ok(Ended::Retune(idle));
        }
        let ended = match next {
            Demuxed::Packet(packet) => playing.packet(packet)?,
            Demuxed::Section { formats, without_audio } => {
                playing = playing.section(formats, without_audio)?;
                None
            }
            Demuxed::Looped => playing.looped(),
            Demuxed::Failed(error) => return Err(Error::Demuxer(error)),
        };
        if let Some(ended) = ended {
            return Ok(ended);
        }
    }
}

/// How long a live source has had nothing to feed, once past RETUNE_AFTER.
fn starved(feeder: &Feeder, live: bool) -> Option<Duration> {
    feeder.last_push.map(|last| last.elapsed()).filter(|idle| live && *idle > RETUNE_AFTER)
}

/// What loading a decoder needs.
struct Load<'a> {
    open: &'a OpenBackend,
    settings: &'a Settings,
    stop: &'a AtomicBool,
    stats: &'a Mutex<Stats>,
}

/// The decoder of the opening, the formats it is loaded for and whether
/// the section lacks the audio it has: loaded again for a section that
/// needs another one.
struct Playing<'a> {
    load: Load<'a>,
    feeder: Feeder<'a>,
    formats: Formats,
    without_audio: bool,
    audio_offset: i64,
}

impl<'a> Playing<'a> {
    fn start(load: Load<'a>, formats: Formats, without_audio: bool) -> Result<Self, Error> {
        let feeder = Feeder::start(load.open, load.settings, load.stop, load.stats, formats)?;
        let audio_offset = load.settings.audio_offset_ms * 1000;
        Ok(Self { load, feeder, formats, without_audio, audio_offset })
    }

    /// Feeds `packet`; how the opening ended, when it did.
    fn packet(&mut self, packet: Packet) -> Result<Option<Ended>, Error> {
        let audio = self.formats.audio.is_some();
        let fed = match packet {
            Packet::Video { data, pts_us, dts_us, section, .. } => {
                // The decoder has an audio track: keep it fed alongside the video.
                let silence = (audio && self.without_audio).then_some(self.audio_offset);
                self.feeder.feed_video(&data, section, dts_us, pts_us, silence)?
            }
            Packet::Audio { data, pts_us: Some(pts), section } if audio => {
                self.feeder.feed_audio(&data, section, pts, self.audio_offset)?
            }
            Packet::Audio { .. } => true,
            Packet::Eof => {
                self.feeder.set_phase(Phase::Draining);
                self.feeder.drain();
                false
            }
        };
        Ok((!fed).then_some(Ended::Done))
    }

    /// A new section, with its formats when known: a decoder loaded again,
    /// as for a new channel, on a new timeline, when the section needs
    /// another one; else only the screens learn its formats, as a
    /// resolution that changes.
    fn section(mut self, formats: Option<(Formats, String)>, without_audio: bool) -> Result<Self, Error> {
        self.without_audio = without_audio;
        let Some((next, description)) = formats else { return Ok(self) };
        if !self.formats.need_reload_for(&next) {
            show_formats(self.load.stats, next);
            return Ok(self);
        }
        let reloading = Instant::now();
        let Self { load, feeder, formats, .. } = self;
        // The TV plays one pipeline at a time: this one goes before the
        // next one loads.
        drop(feeder);
        let playing = Self::start(load, Formats { audio: next.audio.or(formats.audio), ..next }, without_audio)?;
        log!("decoder reloaded for {description} in {} ms", reloading.elapsed().as_millis());
        Ok(playing)
    }

    /// A local file started again from its beginning.
    fn looped(&mut self) -> Option<Ended> {
        let offset_us = self.feeder.timeline.continue_after_loop();
        log!("restarted from the beginning, offset {} ms", offset_us / 1000);
        None
    }
}

/// The freeze test (`freeze_test_s`): the decoder paused once, that long
/// after the opening.
struct FreezeTest {
    due: Option<Instant>,
}

impl FreezeTest {
    fn new(settings: &Settings) -> Self {
        let due = (settings.freeze_test_s > 0).then(|| Instant::now() + Duration::from_secs(settings.freeze_test_s));
        Self { due }
    }

    fn run(&mut self, feeder: &mut Feeder) {
        if self.due.is_none_or(|due| Instant::now() < due) {
            return;
        }
        self.due = None;
        log!("test: decoder paused, {}", if feeder.backend.pause() { "accepted" } else { "refused" });
    }
}
