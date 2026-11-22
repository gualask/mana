//! Where following a live playlist stands from one reload to the next, and
//! the rules that follow from it: where to start, when to reload, when the
//! playlist is stuck.

use std::time::{Duration, Instant};

use url::Url;

use super::playlist::MediaPlaylist;
use crate::log::log;

/// A live playlist with nothing new for this many half target durations
/// (3.5 targets, as Media3 waits) is stuck, as is one whose numbering went
/// back (an encoder restarted): the opening asks to be opened again from
/// the live edge.
const STUCK_HALF_TARGETS: u32 = 7;

/// Where following one media playlist stands, from one reload to the next.
pub(super) struct Progress {
    /// The next segment to send, once the first reload chose where to start.
    pub(super) next: Option<u64>,
    /// Where the variant switched from left off, until the first reload of
    /// the new one finds its segment there.
    left: Option<Position>,
    /// The initialization section (fMP4) last sent ahead of the segments.
    pub(super) sent_init: Option<Url>,
    pub(super) sent_segment: bool,
    /// Another variant is followed: its first segment starts a section.
    pub(super) switched: bool,
    /// When a reload last brought a new segment.
    pub(super) progressed: Instant,
}

impl Progress {
    pub(super) fn new() -> Self {
        Self { next: None, left: None, sent_init: None, sent_segment: false, switched: false, progressed: Instant::now() }
    }

    /// The first segment of `playlist`, fetched at `fetched_at`, to send:
    /// the next one, unless the window moved past it; at the first reload
    /// the live start, or the beginning of an ended playlist; after a
    /// switch, the one where the other variant left off.
    pub(super) fn start(&mut self, playlist: &MediaPlaylist, fetched_at: Instant) -> u64 {
        let first = playlist.media_sequence;
        let mut from = match (self.next, self.left.take()) {
            (Some(next), _) => next,
            (None, Some(left)) => left.find(playlist, fetched_at),
            (None, None) if playlist.ended => first,
            (None, None) => playlist.live_start(),
        };
        if from < first {
            log!("hls: rimasto indietro di {} segmenti, salto alla finestra", first - from);
            from = first;
        }
        self.next = Some(from);
        from
    }

    /// Leaves `playlist`, fetched at `fetched_at`, for another variant,
    /// which goes on from the same point in time: the variants of a stream
    /// may number their segments each its own way (RFC 8216 §4.3.3.2).
    pub(super) fn switch(&mut self, playlist: &MediaPlaylist, fetched_at: Instant) {
        if let Some(next) = self.next.take() {
            self.left = Some(Position::of(playlist, next, fetched_at));
        }
        self.switched = true;
    }
}

/// A point in the time of a stream, the start of a segment, as variants
/// share it: its date, when the playlist has EXT-X-PROGRAM-DATE-TIME; else
/// its distance from the start of an ended playlist, or from the live edge
/// when the playlist was fetched.
struct Position {
    date: Option<f64>,
    from_start: f64,
    from_end: f64,
    ended: bool,
    fetched_at: Instant,
}

impl Position {
    /// Where segment `next` of `playlist` starts; the end of the playlist
    /// when `next` comes after it.
    fn of(playlist: &MediaPlaylist, next: u64, fetched_at: Instant) -> Self {
        let starts = starts(playlist);
        let at = starts.iter().find(|s| s.sequence >= next).unwrap_or(starts.last().expect("the end is a start"));
        let total = starts.last().map_or(0.0, |s| s.time);
        Self { date: at.date, from_start: at.time, from_end: total - at.time, ended: playlist.ended, fetched_at }
    }

    /// The segment of `playlist`, fetched at `fetched_at`, that starts
    /// nearest to this point: by date when both have one, else by distance
    /// from the start of two ended playlists, else from the live edge,
    /// which moved on meanwhile. Past its last segment, the one to come.
    fn find(&self, playlist: &MediaPlaylist, fetched_at: Instant) -> u64 {
        let starts = starts(playlist);
        let total = starts.last().map_or(0.0, |s| s.time);
        let distance: Box<dyn Fn(&Start) -> f64> = match self.date {
            Some(date) if starts.iter().all(|s| s.date.is_some()) => Box::new(move |s| (s.date.unwrap_or_default() - date).abs()),
            _ if self.ended && playlist.ended => Box::new(|s| (s.time - self.from_start).abs()),
            _ => {
                let since = fetched_at.saturating_duration_since(self.fetched_at).as_secs_f64();
                let time = total - (self.from_end + since);
                Box::new(move |s| (s.time - time).abs())
            }
        };
        let nearest = starts.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)));
        nearest.map_or(playlist.media_sequence, |s| s.sequence)
    }
}

/// Where a segment starts, from the start of its playlist.
struct Start {
    sequence: u64,
    time: f64,
    date: Option<f64>,
}

/// Where each segment of `playlist` starts, then where the next one will.
fn starts(playlist: &MediaPlaylist) -> Vec<Start> {
    let mut starts = Vec::with_capacity(playlist.segments.len() + 1);
    let (mut time, mut date, mut sequence) = (0.0, None, playlist.media_sequence);
    for segment in &playlist.segments {
        starts.push(Start { sequence: segment.sequence, time, date: segment.date });
        let duration = segment.duration.as_secs_f64();
        (time, date, sequence) = (time + duration, segment.date.map(|d| d + duration), segment.sequence + 1);
    }
    starts.push(Start { sequence, time, date });
    starts
}

/// Why `playlist` is stuck, when no reload brought a new segment for `idle`,
/// longer than STUCK_HALF_TARGETS half target durations.
pub(super) fn stuck(playlist: &MediaPlaylist, idle: Duration, next: Option<u64>) -> Option<String> {
    if idle <= (playlist.target_duration / 2).saturating_mul(STUCK_HALF_TARGETS) {
        return None;
    }
    let first = playlist.media_sequence;
    let last = playlist.segments.last().map_or(first, |s| s.sequence);
    Some(format!("playlist ferma da {} s (ultimo segmento {last}, atteso {})", idle.as_secs(), next.unwrap_or(first)))
}

/// RFC 8216 §6.3.4: reload after the last segment's duration, or half a
/// target duration when nothing was new. The target is an upper bound:
/// waiting that long after shorter segments drains the buffer at every
/// reload. Half a target is the lower one: a segment of zero seconds must
/// not have the playlist reloaded without pause.
pub(super) fn reload_wait(playlist: &MediaPlaylist, added: bool) -> Duration {
    let half = playlist.target_duration / 2;
    let last = playlist.segments.last().map_or(playlist.target_duration, |s| s.duration);
    if added { last.max(half) } else { half }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live playlist with target duration 2 s and `count` segments of
    /// `duration` s from sequence 10.
    fn playlist(count: u64, duration: f64) -> MediaPlaylist {
        numbered(10, count, duration, "")
    }

    /// `count` segments of `duration` s from sequence `first`, after the
    /// tags in `head`.
    fn numbered(first: u64, count: u64, duration: f64, head: &str) -> MediaPlaylist {
        let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n{head}");
        for i in 0..count {
            text += &format!("#EXTINF:{duration},\ns{i}.ts\n");
        }
        MediaPlaylist::parse(&text, &Url::parse("http://lab.invalid/media.m3u8").unwrap()).unwrap()
    }

    /// The segment of `to` where following `from` goes on after a switch at
    /// its segment `next`, `later` after `from` was fetched.
    fn switched(from: &MediaPlaylist, next: u64, to: &MediaPlaylist, later: Duration) -> u64 {
        let mut progress = Progress::new();
        let fetched = Instant::now();
        progress.next = Some(next);
        progress.switch(from, fetched);
        assert!(progress.switched && progress.next.is_none());
        progress.start(to, fetched + later)
    }

    #[test]
    fn following_starts_at_the_live_start_then_goes_on_and_catches_up_with_the_window() {
        let window = playlist(6, 2.0);
        let mut progress = Progress::new();
        let now = Instant::now();
        assert_eq!(progress.start(&window, now), window.live_start());
        progress.next = Some(13);
        assert_eq!(progress.start(&window, now), 13);
        // Fallen behind the window: it starts at its first segment.
        progress.next = Some(4);
        assert_eq!(progress.start(&window, now), 10);
        assert_eq!(progress.next, Some(10));
    }

    #[test]
    fn a_switch_goes_on_at_the_same_time_however_the_variants_number_their_segments() {
        let old = numbered(100, 6, 2.0, "");
        // Segment 103 starts 6 s from the live edge: so does 503, and 13.
        assert_eq!(switched(&old, 103, &numbered(500, 6, 2.0, ""), Duration::ZERO), 503);
        assert_eq!(switched(&old, 103, &numbered(10, 6, 2.0, ""), Duration::ZERO), 13);
        // Fetched 2 s later, the new window has a segment more.
        assert_eq!(switched(&old, 103, &numbered(11, 6, 2.0, ""), Duration::from_secs(2)), 13);
        // All sent: the next one to come.
        assert_eq!(switched(&old, 106, &numbered(500, 6, 2.0, ""), Duration::ZERO), 506);
    }

    #[test]
    fn a_switch_follows_the_program_date_time_and_the_start_of_ended_playlists() {
        // The new window is shorter and 3 s behind: only the dates tell.
        let old = numbered(100, 6, 2.0, "#EXT-X-PROGRAM-DATE-TIME:2026-11-21T10:00:00Z\n");
        let new = numbered(7, 4, 2.0, "#EXT-X-PROGRAM-DATE-TIME:2026-11-21T10:00:01Z\n");
        // 103 starts at 10:00:06; 9 at 10:00:05, 10 at 10:00:07: the first.
        assert_eq!(switched(&old, 103, &new, Duration::ZERO), 9);
        let ended = |first| numbered(first, 3, 2.0, "#EXT-X-ENDLIST\n");
        assert_eq!(switched(&ended(0), 1, &ended(500), Duration::from_secs(9)), 501);
    }

    #[test]
    fn a_playlist_is_stuck_past_three_and_a_half_target_durations() {
        let window = playlist(6, 2.0);
        assert_eq!(stuck(&window, Duration::from_secs(7), Some(16)), None);
        let reason = stuck(&window, Duration::from_secs(8), Some(16)).unwrap();
        assert_eq!(reason, "playlist ferma da 8 s (ultimo segmento 15, atteso 16)");
    }

    #[test]
    fn reloads_follow_the_last_segment_or_half_the_target() {
        let short = playlist(6, 1.5);
        assert_eq!(reload_wait(&short, true), Duration::from_secs_f64(1.5));
        assert_eq!(reload_wait(&short, false), Duration::from_secs(1));
        let empty = playlist(6, 0.0);
        assert_eq!(reload_wait(&empty, true), Duration::from_secs(1));
    }
}
