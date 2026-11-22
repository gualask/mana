//! Which variant of a master playlist to follow: the one the viewer chose,
//! or one chosen on the bandwidth measured while downloading its segments,
//! as hls.js does: two moving averages of the download rate, the lower one
//! taken as the estimate; down to a lighter variant when the one followed
//! needs more than nearly all of it, up only to one that fits in 70% of it
//! (also Media3's share).
//!
//! Going down is immediate, against a stall. Going up waits after going
//! down, and longer each time going up proves wrong, as dash.js holds back
//! a quality it often had to leave (`SwitchHistoryRule`): a network
//! swinging around what a variant needs does not swing the picture with it.
//! The long waits are for going up from a picture already good to look at;
//! below it, going up is tried again at least once a minute.
//!
//! Before any measure the ladder starts on a picture of European SD, as
//! hls.js starts on what a default estimate allows (`abrEwmaDefaultEstimate`)
//! and Media3 on an initial estimate: on a slow network the first segments
//! come sooner, and on a fast one the first measure goes up at once.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use url::Url;

use super::playlist::Variant;
use crate::live::{Quality, QualityChoice, Stats};
use crate::log::log;
use average::Average;

mod average;

/// Half-lives of the two averages, in seconds of download (hls.js
/// `abrEwmaFastLive`, `abrEwmaSlowLive`): the fast one follows a network
/// that slows down, the slow one keeps a short burst from counting much.
const FAST_HALF_LIFE: f64 = 3.0;
const SLOW_HALF_LIFE: f64 = 9.0;
/// Share of the estimate a variant may need to be kept (hls.js
/// `abrBandWidthFactor`).
const KEEP_SHARE: f64 = 0.95;
/// Share of the estimate a heavier variant may need to be chosen (hls.js
/// `abrBandWidthUpFactor`, Media3 `bandwidthFraction`).
const UP_SHARE: f64 = 0.7;
/// A download counts as taking at least this long (hls.js `minDelayMs`): a
/// tiny segment from a cache does not make the network look endless.
const MIN_DOWNLOAD: Duration = Duration::from_millis(50);
/// How long after going down before going up again, at first (Media3 wants
/// 10 s of buffer to go up, Shaka 8 s between switches).
const UP_HOLD: Duration = Duration::from_secs(10);
/// Going up proved wrong when it went down again within this time: the
/// next hold lasts at least as long, then doubles at each wrong one up to
/// MAX_UP_HOLD. A variant kept this long brings the hold back to UP_HOLD.
const UP_PROVEN: Duration = Duration::from_secs(60);
const MAX_UP_HOLD: Duration = Duration::from_secs(240);
/// A picture this many lines high is good to look at on a TV: 540p, and the
/// 576 lines of European SD. Below it, the hold lasts at most
/// MAX_UP_HOLD_BELOW_GOOD.
const GOOD_LINES: u32 = 540;
const MAX_UP_HOLD_BELOW_GOOD: Duration = UP_PROVEN;
/// The highest picture to start on, before any measure; for variants
/// without a resolution, the most bandwidth, about what a 576p variant
/// needs (RaiPlay's takes 2.1 Mbit/s).
const START_LINES: u32 = 576;
const START_BANDWIDTH: u64 = 2_500_000;

/// The variants to switch between, the one followed and the bandwidth
/// measured; kept across the openings of a source.
pub(super) struct Ladder {
    /// Lightest first.
    variants: Vec<Variant>,
    current: usize,
    fast: Average,
    slow: Average,
    went_up: Option<Instant>,
    went_down: Option<Instant>,
    /// How long after going down before going up again.
    up_hold: Duration,
    choice: QualityChoice,
}

impl Ladder {
    /// Starts on the variant the viewer chose, else on the one `start`
    /// gives.
    pub(super) fn new(variants: Vec<Variant>, choice: QualityChoice, stats: Arc<Mutex<Stats>>) -> Self {
        assert!(!variants.is_empty(), "a master playlist has at least one variant");
        if variants.len() > 1 {
            let list: Vec<String> = variants.iter().map(describe).collect();
            log!("hls: varianti {}", list.join(", "));
        }
        stats.lock().unwrap().heights = heights(&variants);
        let mut ladder = Self {
            current: start(&variants),
            variants,
            fast: Average::new(FAST_HALF_LIFE),
            slow: Average::new(SLOW_HALF_LIFE),
            went_up: None,
            went_down: None,
            up_hold: UP_HOLD,
            choice,
        };
        ladder.current = ladder.chosen().unwrap_or(ladder.current);
        ladder
    }

    pub(super) fn current(&self) -> &Url {
        &self.variants[self.current].url
    }

    /// Counts a segment of `bytes` downloaded in `took`.
    pub(super) fn measure(&mut self, bytes: usize, took: Duration) {
        let seconds = took.max(MIN_DOWNLOAD).as_secs_f64();
        let rate = bytes as f64 * 8.0 / seconds;
        self.fast.add(seconds, rate);
        self.slow.add(seconds, rate);
    }

    /// The variant to follow from the next segment on, at `now`, when it is
    /// another one: the one the viewer chose; else lighter when the one
    /// followed needs more than KEEP_SHARE of the estimate, heavier when
    /// one fits in UP_SHARE of it and the hold since going down is over.
    pub(super) fn choose(&mut self, now: Instant) -> Option<Url> {
        if let Some(chosen) = self.chosen() {
            return (chosen != self.current).then(|| self.switch(chosen, "scelta"));
        }
        let estimate = self.estimate()?;
        if self.up_kept(now, UP_PROVEN) {
            self.up_hold = UP_HOLD;
        }
        let held = self.went_down.is_some_and(|down| now.duration_since(down) < self.hold_from(self.current));
        let fits = |share: f64| self.variants.iter().rposition(|v| v.bandwidth as f64 <= estimate * share);
        let next = match fits(UP_SHARE) {
            Some(up) if up > self.current && !held => up,
            _ if self.variants[self.current].bandwidth as f64 > estimate * KEEP_SHARE => fits(KEEP_SHARE).unwrap_or(0),
            _ => return None,
        };
        if next == self.current {
            return None;
        }
        let estimated = format!("banda stimata {} kbit/s", (estimate / 1000.0).round());
        if next > self.current {
            self.went_up = Some(now);
            return Some(self.switch(next, &estimated));
        }
        // Gone up and down again within UP_PROVEN: going up was wrong.
        if self.up_since_down() && !self.up_kept(now, UP_PROVEN) {
            self.up_hold = (self.up_hold * 2).max(UP_PROVEN).min(MAX_UP_HOLD);
        }
        self.went_down = Some(now);
        let hold = self.hold_from(next);
        Some(self.switch(next, &format!("{estimated}; risalita non prima di {} s", hold.as_secs())))
    }

    /// Follows variant `next` from now on, logging `why`; its URL.
    fn switch(&mut self, next: usize, why: &str) -> Url {
        log!("hls: variante {} al posto di {}, {why}", describe(&self.variants[next]), describe(&self.variants[self.current]));
        self.current = next;
        self.variants[next].url.clone()
    }


    /// The variant the viewer chose, if any.
    fn chosen(&self) -> Option<usize> {
        let Quality::Lines(lines) = self.choice.get() else { return None };
        if self.variants.iter().all(|v| v.height().is_none()) {
            return None;
        }
        Some(heaviest_fitting(&self.variants, |v| v.height().is_some_and(|h| h <= lines)))
    }

    /// How long after going down to variant `index` before going up from
    /// it: up to MAX_UP_HOLD_BELOW_GOOD below a good picture (unknown
    /// heights count as good).
    fn hold_from(&self, index: usize) -> Duration {
        match self.variants[index].height() {
            Some(lines) if lines < GOOD_LINES => self.up_hold.min(MAX_UP_HOLD_BELOW_GOOD),
            _ => self.up_hold,
        }
    }

    /// Whether the last change went up.
    fn up_since_down(&self) -> bool {
        self.went_up.is_some_and(|up| self.went_down.is_none_or(|down| down < up))
    }

    /// Whether the last change went up, at least `kept` before `now`.
    fn up_kept(&self, now: Instant, kept: Duration) -> bool {
        self.up_since_down() && self.went_up.is_some_and(|up| now.duration_since(up) >= kept)
    }

    /// Bits per second, once a segment was measured.
    fn estimate(&self) -> Option<f64> {
        Some(self.fast.value()?.min(self.slow.value()?))
    }
}

/// The variant to start on, before any measure: the best at most
/// START_LINES high, or needing at most START_BANDWIDTH when its height is
/// not known; else the lightest.
fn start(variants: &[Variant]) -> usize {
    heaviest_fitting(variants, |v| v.height().map_or(v.bandwidth <= START_BANDWIDTH, |lines| lines <= START_LINES))
}

/// The heaviest of `variants`, lightest first, that `fits`; else the
/// lightest.
fn heaviest_fitting(variants: &[Variant], fits: impl Fn(&Variant) -> bool) -> usize {
    variants.iter().rposition(fits).unwrap_or(0)
}

/// The heights of `variants`, lowest first, each once.
fn heights(variants: &[Variant]) -> Vec<u32> {
    let mut heights: Vec<u32> = variants.iter().filter_map(Variant::height).collect();
    heights.sort_unstable();
    heights.dedup();
    heights
}

fn describe(variant: &Variant) -> String {
    let kbps = variant.bandwidth / 1000;
    match &variant.resolution {
        Some(resolution) => format!("{resolution} {kbps} kbit/s"),
        None => format!("{kbps} kbit/s"),
    }
}

#[cfg(test)]
mod tests;
