//! Feeds packets to the decoder at most a fixed lead ahead of the frame it
//! is displaying. Pacing on the displayed position rather than on our own
//! clock absorbs any clock rate difference. While data is missing the
//! decoder is paused before it runs dry: left playing, its clock runs on
//! through the gap and it then races through the data that comes after.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::demux::Formats;
use super::silence::Silence;
use super::timeline::{Timeline, Track, Unwrap};
use super::{Backend, BackendError, Error, OpenBackend, Phase, Settings, Stats};
use crate::log::{debug, log};

mod opening;
mod presentation;

pub use opening::{Ended, play};

const RETRY_SLEEP: Duration = Duration::from_millis(5);
const MAX_WAIT_SLICE: Duration = Duration::from_millis(20);
/// A decoder that refuses data for this long is treated as stuck.
const STUCK_AFTER: Duration = Duration::from_secs(3);
/// Displayed position unchanged this long, while data waits, means the
/// decoder is stuck.
const FROZEN_AFTER: Duration = Duration::from_secs(15);
/// At the end of a file, how long to let queued data play out.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const REPORT_EVERY: Duration = Duration::from_secs(1);
/// Audio missing for longer than this many frames is filled with silence.
const MAX_AUDIO_GAP_FRAMES: i64 = 4;

struct Feeder<'a> {
    backend: Box<dyn Backend>,
    settings: &'a Settings,
    stop: &'a AtomicBool,
    stats: &'a Mutex<Stats>,
    /// Wall clock of the first fed packet; paces feeding until the decoder
    /// reports a displayed frame.
    started: Option<Instant>,
    /// Last displayed position (µs) and when it last changed.
    presented: Option<(i64, Instant)>,
    stalled: bool,
    /// Undoes the MPEG-TS clock wrap, per track.
    clocks: [Unwrap; 2],
    timeline: Timeline,
    last_report: Instant,
    /// A silent frame in the format of the stream's audio.
    silence: Option<Silence>,
    /// Timeline position where the audio fed so far ends.
    audio_end_us: Option<i64>,
    /// Where the current stretch of silence started.
    silent_since_us: Option<i64>,
    /// When data last went to the decoder.
    last_push: Option<Instant>,
    /// Since when the decoder is paused for lack of data.
    paused: Option<Instant>,
    /// Data ran short and the decoder refused to pause: it stood still.
    ran_dry: bool,
}

impl<'a> Feeder<'a> {
    /// Loads a decoder for `formats`, with a timeline starting afresh.
    fn start(
        open: &OpenBackend,
        settings: &'a Settings,
        stop: &'a AtomicBool,
        stats: &'a Mutex<Stats>,
        formats: Formats,
    ) -> Result<Self, Error> {
        show_formats(stats, formats);
        let mut backend = open().map_err(Error::NoDecoder)?;
        let load_started = Instant::now();
        backend.load(&formats)?;
        log!("decoder loaded in {} ms", load_started.elapsed().as_millis());
        let feeder = Feeder {
            backend,
            settings,
            stop,
            stats,
            started: None,
            presented: None,
            stalled: false,
            clocks: Default::default(),
            timeline: Timeline::default(),
            last_report: Instant::now(),
            silence: None,
            audio_end_us: None,
            silent_since_us: None,
            last_push: None,
            paused: None,
            ran_dry: false,
        };
        feeder.set_phase(Phase::Playing);
        Ok(feeder)
    }

    /// Feeds one access unit with its source timestamps, then, with an
    /// audio PTS offset in `silence`, the silence missing up to it. Access
    /// units arrive in decode order: paced on DTS, stamped with PTS; one
    /// without either is skipped. Returns false when playback must stop.
    fn feed_video(
        &mut self,
        data: &[u8],
        section: u64,
        dts: Option<i64>,
        pts: Option<i64>,
        silence: Option<i64>,
    ) -> Result<bool, Error> {
        let Some(schedule) = dts.or(pts) else { return Ok(true) };
        let schedule = self.place(Track::Video, schedule, section);
        let pts_ns = pts.map_or(schedule, |pts| self.place(Track::Video, pts, section)).max(0) * 1000;
        self.timeline.reach(schedule);
        if !self.push(schedule, pts_ns, |b, ns| b.feed_video(data, ns))? {
            return Ok(false);
        }
        match silence {
            Some(pts_offset) => self.fill_silence(self.timeline.reached(), pts_offset),
            None => Ok(true),
        }
    }

    /// Feeds one ADTS (or other) audio frame, after silence for any gap
    /// before it: the decoder beeps when its audio runs out.
    fn feed_audio(&mut self, data: &[u8], section: u64, source_us: i64, pts_offset: i64) -> Result<bool, Error> {
        let at = self.place(Track::Audio, source_us, section);
        if !self.silence.as_ref().is_some_and(|s| s.matches(data)) {
            self.silence = Silence::matching(data);
        }
        if !self.fill_silence(at, pts_offset)? {
            return Ok(false);
        }
        if let Some(since) = self.silent_since_us.take() {
            debug!("audio back after {} ms of silence", (at - since) / 1000);
        }
        self.timeline.reach(at);
        let pts_ns = (at + pts_offset).max(0) * 1000;
        if !self.push(at, pts_ns, |b, ns| b.feed_audio(data, ns))? {
            return Ok(false);
        }
        self.audio_end_us = Some(at + self.silence.as_ref().map_or(0, Silence::duration_us));
        Ok(true)
    }

    /// Feeds silent frames from the end of the audio fed so far up to
    /// `until_us`, when more than a few frames are missing.
    fn fill_silence(&mut self, until_us: i64, pts_offset: i64) -> Result<bool, Error> {
        let (Some(silence), Some(mut at)) = (self.silence.as_ref(), self.audio_end_us) else { return Ok(true) };
        let (frame, step) = (silence.frame().to_vec(), silence.duration_us());
        if until_us - at <= MAX_AUDIO_GAP_FRAMES * step {
            return Ok(true);
        }
        if self.silent_since_us.is_none() {
            debug!("audio missing for {} ms: silence in its place", at / 1000);
            self.silent_since_us = Some(at);
        }
        while at + step <= until_us {
            if !self.push(at, (at + pts_offset).max(0) * 1000, |b, ns| b.feed_audio(&frame, ns))? {
                return Ok(false);
            }
            at += step;
            self.audio_end_us = Some(at);
        }
        Ok(true)
    }

    /// At the end of a file: lets the queued data play out.
    fn drain(&mut self) {
        if let Err(error) = self.resume() {
            log!("{error}");
            return;
        }
        let draining = Instant::now();
        while self.displayed().is_some_and(|us| us < self.timeline.reached())
            && draining.elapsed() < DRAIN_TIMEOUT
            && !self.stop.load(Ordering::Relaxed)
        {
            std::thread::sleep(MAX_WAIT_SLICE);
        }
    }

    /// Places a source timestamp on the timeline, unwrapped.
    fn place(&mut self, track: Track, source_us: i64, section: u64) -> i64 {
        let source_us = self.clocks[track as usize].apply(source_us);
        self.timeline.place(track, source_us, section).us
    }

    /// Waits for room ahead of the displayed frame, then hands the data to
    /// the decoder, retrying while its buffer is full.
    fn push(
        &mut self,
        schedule: i64,
        pts_ns: i64,
        play: impl Fn(&mut dyn Backend, i64) -> Result<bool, BackendError>,
    ) -> Result<bool, Error> {
        if std::mem::take(&mut self.ran_dry) {
            self.restart_frozen_timer();
        }
        if !self.wait_for_room(schedule)? {
            return Ok(false);
        }
        let refused_since = Instant::now();
        while !play(self.backend.as_mut(), pts_ns)? {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            if refused_since.elapsed() > STUCK_AFTER {
                return Err(Error::DecoderRefuses(refused_since.elapsed()));
            }
            std::thread::sleep(RETRY_SLEEP);
        }
        self.last_push = Some(Instant::now());
        self.publish();
        Ok(true)
    }

    /// Sleeps until `schedule` is at most the lead ahead of the displayed
    /// frame, or of the wall clock before the first frame is displayed. A
    /// decoder paused for lack of data resumes once it has the lead.
    fn wait_for_room(&mut self, schedule: i64) -> Result<bool, Error> {
        let lead = self.settings.lead_ms * 1000;
        let started = *self.started.get_or_insert_with(Instant::now);
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let position = self.displayed().unwrap_or_else(|| started.elapsed().as_micros() as i64);
            if schedule <= position + lead {
                return Ok(true);
            }
            if self.paused.is_some() {
                self.resume()?;
                continue;
            }
            if self.presented.is_some_and(|(_, changed)| changed.elapsed() > FROZEN_AFTER) {
                return Err(Error::DecoderFrozen(FROZEN_AFTER));
            }
            std::thread::sleep(MAX_WAIT_SLICE);
        }
    }

    fn publish(&mut self) {
        let mut stats = self.stats.lock().unwrap();
        stats.presented_ms = self.backend.presented_ns().map(|ns| ns / 1_000_000);
        if self.last_report.elapsed() >= REPORT_EVERY {
            self.last_report = Instant::now();
            debug!(
                "position: sent {} ms, presented {:?} ms, buffer low {}",
                self.timeline.reached() / 1000,
                stats.presented_ms,
                self.backend.buffer_low()
            );
        }
    }

    fn set_phase(&self, phase: Phase) {
        self.stats.lock().unwrap().phase = phase;
    }
}

/// Shows `formats` as those playing.
fn show_formats(stats: &Mutex<Stats>, formats: Formats) {
    stats.lock().unwrap().formats = Some(formats);
}

#[cfg(test)]
mod tests;
