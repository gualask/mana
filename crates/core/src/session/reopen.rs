//! Playing a source to its end: a local file once, a live source opened
//! again whenever an opening is interrupted, as a TV tunes in again.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::stats::Shared;
use super::Program;
use crate::live::hls::{self, Live};
use crate::live::source::{self, Source};
use crate::live::{Interruption, QualityChoice};
use crate::log::log;
use crate::playback::demux::Demux;
use crate::playback::feeder::{self, Ended};
use crate::playback::ffmpeg::Media;
use crate::playback::reader::Reader;
use crate::playback::{self, OpenBackend, Settings};

/// A live source is opened again after a failure, or resolved again after
/// a refused token, at most this many times within REOPEN_WINDOW; then
/// playback ends. A frozen decoder takes 15 s to be noticed, so the window
/// must hold more than that many of them: one stuck every 20 s ends after
/// about two minutes, an occasional one is always recovered.
pub const MAX_REOPENINGS: usize = 6;
const REOPEN_WINDOW: Duration = Duration::from_secs(300);

/// What to do once an opening of a live source ended.
#[derive(Debug, PartialEq)]
enum Next {
    /// The source ended, or cannot be played.
    End(Result<(), String>),
    /// Open it again from the live edge; `budgeted` failures count against
    /// MAX_REOPENINGS.
    Reopen { reason: String, budgeted: bool },
    /// Resolve it again, then open it: its token was refused.
    Resolve(String),
}

/// The policy, in one place: how an opening ended, by live's account first
/// (its interruption stops playback too), then by playback's.
fn next(interruption: Option<Interruption>, ended: Result<Ended, playback::Error>) -> Next {
    match (interruption, ended) {
        (Some(Interruption::Refused(reason)), _) => Next::Resolve(reason),
        (Some(Interruption::Stuck(reason)), _) => Next::Reopen { reason, budgeted: false },
        (Some(interruption @ (Interruption::Unsupported(_) | Interruption::Failed(_))), _) => {
            Next::End(Err(interruption.to_string()))
        }
        (None, Ok(Ended::Done)) => Next::End(Ok(())),
        (None, Ok(Ended::Retune(idle))) => {
            Next::Reopen { reason: format!("{} s senza dati", idle.as_secs()), budgeted: false }
        }
        (None, Err(playback::Error::Unsupported(reason))) => Next::End(Err(reason)),
        (None, Err(playback::Error::Failed(reason))) => Next::Reopen { reason, budgeted: true },
    }
}

/// Reopenings and resolutions within the last REOPEN_WINDOW.
#[derive(Default)]
struct Budget {
    spent: VecDeque<Instant>,
}

impl Budget {
    /// Spends one; None once MAX_REOPENINGS are spent within the window,
    /// else how many are.
    fn spend(&mut self) -> Option<usize> {
        self.spent.retain(|at| at.elapsed() < REOPEN_WINDOW);
        if self.spent.len() >= MAX_REOPENINGS {
            return None;
        }
        self.spent.push_back(Instant::now());
        Some(self.spent.len())
    }
}

pub(super) fn run(
    program: &Program,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), String> {
    let loaded;
    let source = match program {
        Program::Live(source) => source,
        // A `.stream` file describes a live HLS stream.
        Program::Path(path) if path.extension().is_some_and(|e| e == source::EXTENSION) => {
            loaded = Source::load(path)?;
            &loaded
        }
        // Anything else is a local file, which can loop.
        Program::Path(path) => return play_file(path, settings, open, stop, &shared.playback),
    };
    log!("diretta {}", source.describe());
    play_source(source, settings, open, stop, shared, quality)
}

/// Opens the live `source` again and again, as `charge` allows, until it
/// ends or the viewing stops.
fn play_source(
    source: &Source,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), String> {
    let mut budget = Budget::default();
    let mut live = resolve(source, stop, shared, quality, &mut budget)?;
    while let Some(opened) = &live {
        let source_stop = hls::Stop::new(stop);
        let ended = play_live(opened, &source_stop, open, settings, stop, &shared.playback);
        // The downloaders of this opening end with it, also when it failed
        // to open.
        source_stop.set();
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match charge(next(source_stop.interruption(), ended), &mut budget) {
            Next::End(result) => return result,
            Next::Reopen { .. } => {}
            Next::Resolve(_) => live = resolve(source, stop, shared, quality, &mut budget)?,
        }
    }
    Ok(())
}

/// Resolves `source`: None once stopped meanwhile. A refused token, as when
/// the recipe's URL expired already, has it resolved again, within the
/// budget.
fn resolve(
    source: &Source,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
    budget: &mut Budget,
) -> Result<Option<Live>, String> {
    loop {
        let reason = match Live::resolve(source, &hls::Stop::resolving(stop), shared.live.clone(), quality) {
            Ok(live) => return Ok(live),
            Err(Interruption::Refused(reason)) => reason,
            Err(other) => return Err(other.to_string()),
        };
        if let Next::End(result) = charge(Next::Resolve(reason), budget) {
            return result.map(|()| None);
        }
    }
}

/// Logs a reopening or a resolution and charges it to `budget` when it
/// counts: a reopening after an error, or a resolution, in case the new
/// token is refused too. Past the budget, playback ends instead.
fn charge(outcome: Next, budget: &mut Budget) -> Next {
    match outcome {
        Next::End(_) => outcome,
        Next::Reopen { ref reason, budgeted: false } => {
            log!("diretta riaperta dal bordo live: {reason}");
            outcome
        }
        Next::Reopen { reason, budgeted: true } => match budget.spend() {
            Some(spent) => {
                log!("diretta riaperta dal bordo live dopo un errore ({spent}/{MAX_REOPENINGS}): {reason}");
                Next::Reopen { reason, budgeted: true }
            }
            None => Next::End(Err(reason)),
        },
        Next::Resolve(reason) => match budget.spend() {
            Some(spent) => {
                log!("fonte da risolvere di nuovo ({spent}/{MAX_REOPENINGS}): {reason}");
                Next::Resolve(reason)
            }
            None => Next::End(Err(format!("fonte rifiutata anche dopo {MAX_REOPENINGS} nuove risoluzioni: {reason}"))),
        },
    }
}

/// Plays a local file to its end, again and again when the settings loop it.
fn play_file(
    path: &Path,
    settings: &Settings,
    open: &OpenBackend,
    stop: &AtomicBool,
    stats: &Mutex<playback::Stats>,
) -> Result<(), String> {
    let reader = Reader::start(Demux::new(Media::open(path)?, None), settings.loop_playback).map_err(|e| e.to_string())?;
    let ended = feeder::play(&reader, false, open, settings, stop, stats);
    reader.finish();
    ended.map_err(|e| e.to_string())?;
    Ok(())
}

fn play_live(
    live: &Live,
    source_stop: &hls::Stop,
    open: &OpenBackend,
    settings: &Settings,
    stop: &AtomicBool,
    stats: &Mutex<playback::Stats>,
) -> Result<Ended, playback::Error> {
    let streams = live.open(source_stop)?;
    let main = Media::open_stream(Box::new(streams.main))?;
    let audio = streams.audio.map(|a| Media::open_stream(Box::new(a))).transpose()?;
    let reader = Reader::start(Demux::new(main, audio), false)?;
    let ended = feeder::play(&reader, true, open, settings, stop, stats);
    // The reader may be waiting on the downloads for data: they stop first.
    source_stop.set();
    reader.finish();
    ended
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_interruptions_come_first() {
        let refused = Some(Interruption::Refused("HTTP 403 per /x".into()));
        assert_eq!(next(refused, Ok(Ended::Done)), Next::Resolve("HTTP 403 per /x".into()));
        let stuck = Some(Interruption::Stuck("ferma".into()));
        let failing = Err(playback::Error::Failed("fermato".into()));
        assert_eq!(next(stuck, failing), Next::Reopen { reason: "ferma".into(), budgeted: false });
        let unsupported = Some(Interruption::Unsupported("segmenti con BYTERANGE".into()));
        assert_eq!(next(unsupported, Ok(Ended::Done)), Next::End(Err("non supportato: segmenti con BYTERANGE".into())));
    }

    #[test]
    fn playback_failures_reopen_unless_unsupported() {
        assert_eq!(next(None, Ok(Ended::Done)), Next::End(Ok(())));
        let starved = next(None, Ok(Ended::Retune(Duration::from_secs(12))));
        assert_eq!(starved, Next::Reopen { reason: "12 s senza dati".into(), budgeted: false });
        let frozen = next(None, Err(playback::Error::Failed("bloccato".into())));
        assert_eq!(frozen, Next::Reopen { reason: "bloccato".into(), budgeted: true });
        let codec = next(None, Err(playback::Error::Unsupported("codec video non supportato".into())));
        assert_eq!(codec, Next::End(Err("codec video non supportato".into())));
    }

    #[test]
    fn the_budget_runs_out_within_the_window() {
        let mut budget = Budget::default();
        for spent in 1..=MAX_REOPENINGS {
            assert_eq!(budget.spend(), Some(spent));
        }
        assert_eq!(budget.spend(), None);
    }
}
