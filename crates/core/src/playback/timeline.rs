//! The continuous playback timeline: source timestamps, which wrap, jump
//! at discontinuities and restart at each loop, mapped to positions that
//! only go on.

use crate::log::debug;

/// A timestamp this far from the timeline position is a discontinuity
/// (encoder restart, 33-bit wrap, skipped segments), not interleaving.
const JUMP_US: i64 = 3_000_000;
/// Timeline gap left after a discontinuity or a loop: one 25 fps frame.
pub const FRAME_GAP_US: i64 = 40_000;
/// Period of the 33-bit 90 kHz MPEG-TS clock, in µs.
const TS_WRAP_US: i64 = (1 << 33) * 1_000_000 / 90_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Track {
    Video = 0,
    Audio = 1,
}

/// Undoes the MPEG-TS clock wrap for one track: a step back of more than
/// half a period is the clock starting again from zero.
#[derive(Default)]
pub struct Unwrap {
    last: Option<i64>,
    offset: i64,
}

impl Unwrap {
    pub fn apply(&mut self, us: i64) -> i64 {
        if let Some(last) = self.last {
            let step = us + self.offset - last;
            if step < -TS_WRAP_US / 2 {
                self.offset += TS_WRAP_US;
            } else if step > TS_WRAP_US / 2 {
                self.offset -= TS_WRAP_US;
            }
        }
        let unwrapped = us + self.offset;
        self.last = Some(unwrapped);
        unwrapped
    }
}

/// Source timestamps to timeline positions (µs).
#[derive(Clone, Copy)]
struct Mapping {
    base_us: i64,
    offset_us: i64,
}

impl Mapping {
    fn apply(&self, source_us: i64) -> i64 {
        source_us - self.base_us + self.offset_us
    }
}

#[derive(Default)]
pub struct Timeline {
    /// The newest mapping, made by the first track to cross a
    /// discontinuity. Kept across loops, with a new offset: re-deriving it
    /// from whichever track comes first would shift audio against video.
    latest: Option<Mapping>,
    /// The mapping each track is on: after a discontinuity one track can
    /// still carry packets from before it while the other has crossed it.
    mapping: [Option<Mapping>; 2],
    /// Last source timestamp of each track, to tell its own jumps.
    last_source: [Option<i64>; 2],
    /// Discontinuity section of each track's last packet, and of `latest`.
    section: [Option<u64>; 2],
    latest_section: Option<u64>,
    /// The furthest position handed to the decoder.
    reached_us: i64,
}

/// A source timestamp placed on the timeline.
pub struct Placed {
    pub us: i64,
    /// Whether the track jumped to a new mapping right after the position
    /// reached: a discontinuity no other track crossed before.
    pub jumped: bool,
}

impl Timeline {
    /// Maps a source timestamp of `track` to the timeline (µs). A jump in
    /// that track's own timestamps moves it to the newest mapping when that
    /// places it near the position reached (the other track crossed the
    /// same discontinuity first); otherwise a new mapping continues
    /// playback right after the position reached. Until it jumps, each
    /// track keeps its own mapping, so packets still queued from before a
    /// discontinuity are not thrown far ahead. A steady offset between the
    /// tracks (separate HLS renditions starting apart) is not a jump: the
    /// decoder aligns them by timestamp.
    ///
    /// Crossing into another discontinuity section counts as a jump whatever
    /// its size, since timestamps there may go on, step a little or go back:
    /// the first track to cross makes the new mapping, the other adopts it.
    pub fn place(&mut self, track: Track, source_us: i64, section: u64) -> Placed {
        if self.latest.is_none() {
            self.latest_section = Some(section);
        }
        let latest = *self.latest.get_or_insert(Mapping { base_us: source_us, offset_us: 0 });
        let mapping = *self.mapping[track as usize].get_or_insert(latest);
        let previous = self.last_source[track as usize].replace(source_us);
        let crossed = self.section[track as usize].replace(section).is_some_and(|last| last != section);
        let jumped = crossed || previous.is_some_and(|p| (source_us - p).abs() > JUMP_US);
        if !jumped {
            return Placed { us: mapping.apply(source_us).max(0), jumped: false };
        }
        let other_crossed = if crossed {
            self.latest_section == Some(section)
        } else {
            (latest.apply(source_us) - self.reached_us).abs() <= JUMP_US
        };
        if other_crossed {
            self.mapping[track as usize] = Some(latest);
            return Placed { us: latest.apply(source_us).max(0), jumped: false };
        }
        debug!(
            "jump of {} ms in the {} timestamps (from {} to {} ms): timeline realigned",
            (latest.apply(source_us) - self.reached_us) / 1000,
            if track == Track::Video { "video" } else { "audio" },
            previous.unwrap_or(0) / 1000,
            source_us / 1000
        );
        let new = Mapping { base_us: source_us, offset_us: self.reached_us + FRAME_GAP_US };
        self.latest = Some(new);
        self.latest_section = Some(section);
        self.mapping[track as usize] = Some(new);
        Placed { us: new.offset_us, jumped: true }
    }

    /// Records that data up to `us` went to the decoder.
    pub fn reach(&mut self, us: i64) {
        self.reached_us = self.reached_us.max(us);
    }

    pub fn reached(&self) -> i64 {
        self.reached_us
    }

    /// After rewinding a looped file: both tracks continue on the same
    /// base, right after the position reached. Returns the new offset.
    pub fn continue_after_loop(&mut self) -> i64 {
        let offset_us = self.reached_us + FRAME_GAP_US;
        if let Some(latest) = self.latest.as_mut() {
            latest.offset_us = offset_us;
            self.mapping = [Some(*latest); 2];
        }
        offset_us
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Places a timestamp and records it as fed, as the feeder does.
    fn feed(timeline: &mut Timeline, track: Track, source_us: i64, section: u64) -> i64 {
        let placed = timeline.place(track, source_us, section);
        timeline.reach(placed.us);
        placed.us
    }

    #[test]
    fn the_clock_wrap_is_undone() {
        let mut unwrap = Unwrap::default();
        assert_eq!(unwrap.apply(TS_WRAP_US - 20_000), TS_WRAP_US - 20_000);
        assert_eq!(unwrap.apply(20_000), TS_WRAP_US + 20_000);
        assert_eq!(unwrap.apply(TS_WRAP_US - 10_000), TS_WRAP_US - 10_000); // a late packet from before
    }

    #[test]
    fn steady_offset_between_tracks_is_kept() {
        let mut timeline = Timeline::default();
        assert_eq!(feed(&mut timeline, Track::Video, 10_000_000, 0), 0);
        assert_eq!(feed(&mut timeline, Track::Audio, 10_150_000, 0), 150_000);
        assert_eq!(feed(&mut timeline, Track::Video, 10_040_000, 0), 40_000);
    }

    #[test]
    fn a_jump_continues_after_the_position_reached() {
        let mut timeline = Timeline::default();
        feed(&mut timeline, Track::Video, 0, 0);
        feed(&mut timeline, Track::Video, 1_000_000, 0);
        let placed = timeline.place(Track::Video, 90_000_000, 0);
        assert!(placed.jumped);
        assert_eq!(placed.us, 1_000_000 + FRAME_GAP_US);
    }

    #[test]
    fn the_second_track_across_a_discontinuity_adopts_the_new_mapping() {
        let mut timeline = Timeline::default();
        feed(&mut timeline, Track::Video, 5_000_000, 3);
        feed(&mut timeline, Track::Audio, 5_000_000, 3);
        feed(&mut timeline, Track::Video, 6_000_000, 3);
        // The video crosses into section 4, where timestamps restart.
        let video = timeline.place(Track::Video, 100_000, 4);
        assert!(video.jumped);
        assert_eq!(video.us, 1_000_000 + FRAME_GAP_US);
        timeline.reach(video.us);
        // Audio still before the discontinuity stays on its mapping...
        assert_eq!(timeline.place(Track::Audio, 5_960_000, 3).us, 960_000);
        // ...then crosses and lands next to the video, without a jump of its own.
        let audio = timeline.place(Track::Audio, 120_000, 4);
        assert!(!audio.jumped);
        assert_eq!(audio.us, video.us + 20_000);
    }

    #[test]
    fn a_loop_continues_both_tracks_after_the_end() {
        let mut timeline = Timeline::default();
        feed(&mut timeline, Track::Video, 0, 0);
        feed(&mut timeline, Track::Audio, 0, 0);
        feed(&mut timeline, Track::Video, 2_000_000, 0);
        let offset = timeline.continue_after_loop();
        assert_eq!(offset, 2_000_000 + FRAME_GAP_US);
        // Rewound: the jump back is absorbed by the new offset, for both.
        assert_eq!(timeline.place(Track::Video, 0, 0).us, offset);
        assert_eq!(timeline.place(Track::Audio, 0, 0).us, offset);
    }
}
