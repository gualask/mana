//! Playing a stream to its end: a local file once, a live stream opened
//! again whenever an opening is interrupted, as a TV tunes in again.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::Program;
use super::stats::{Failure, Shared};
use crate::live::hls::{self, Live};
use crate::live::playable::{self, Playable};
use crate::live::{Cause, Interruption, QualityChoice, Stall};
use crate::log::log;
use crate::playback::demux::Demux;
use crate::playback::feeder::{self, Ended};
use crate::playback::ffmpeg::Media;
use crate::playback::reader::Reader;
use crate::playback::{self, OpenBackend, Settings};

/// A live stream is opened again after a failure, or resolved again after
/// a refused token, at most this many times within REOPEN_WINDOW; then
/// playback ends. A frozen decoder takes 15 s to be noticed, so the window
/// must hold more than that many of them: one stuck every 20 s ends after
/// about two minutes, an occasional one is always recovered.
pub const MAX_REOPENINGS: usize = 6;
const REOPEN_WINDOW: Duration = Duration::from_secs(300);

/// What to do once an opening of a live stream ended.
#[derive(Debug, PartialEq)]
enum Next {
    /// The stream ended, or cannot be played.
    End(Result<(), Failure>),
    /// Open it again from the live edge.
    Reopen(Reopening),
    /// Resolve it again, then open it: its token was refused.
    Resolve(Cause),
}

/// Why a live stream is opened again.
#[derive(Debug, PartialEq)]
enum Reopening {
    Stuck(Stall),
    /// So long without data.
    Starved(Duration),
    /// Playback failed: these count against MAX_REOPENINGS.
    Failed(playback::Error),
}

impl std::fmt::Display for Reopening {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Stuck(stall) => write!(f, "{stall}"),
            Self::Starved(idle) => write!(f, "{} s without data", idle.as_secs()),
            Self::Failed(error) => write!(f, "{error}"),
        }
    }
}

/// The policy, in one place: how an opening ended, by live's account first
/// (its interruption stops playback too), then by playback's.
fn next(interruption: Option<Interruption>, ended: Result<Ended, playback::Error>) -> Next {
    match (interruption, ended) {
        (Some(Interruption::Refused(cause)), _) => Next::Resolve(cause),
        (Some(Interruption::Stuck(stall)), _) => Next::Reopen(Reopening::Stuck(stall)),
        (Some(interruption @ (Interruption::Unsupported(_) | Interruption::Failed(_))), _) => {
            Next::End(Err(Failure::Live(interruption)))
        }
        (None, Ok(Ended::Done)) => Next::End(Ok(())),
        (None, Ok(Ended::Retune(idle))) => Next::Reopen(Reopening::Starved(idle)),
        (None, Err(error)) if error.is_unsupported() => Next::End(Err(error.into())),
        (None, Err(error)) => Next::Reopen(Reopening::Failed(error)),
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
) -> Result<(), Failure> {
    let loaded;
    let playable = match program {
        Program::Live(playable) => playable,
        // A `.stream` file describes a live HLS stream.
        Program::Path(path) if path.extension().is_some_and(|e| e == playable::EXTENSION) => {
            loaded = Playable::load(path).map_err(Failure::Playable)?;
            &loaded
        }
        // Anything else is a local file, which can loop.
        Program::Path(path) => return Ok(play_file(path, settings, open, stop, &shared.playback)?),
    };
    log!("live {}", playable.describe());
    play_stream(playable, settings, open, stop, shared, quality)
}

/// Opens the live `playable` again and again, as `charge` allows, until it
/// ends or the viewing stops.
fn play_stream(
    playable: &Playable,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) -> Result<(), Failure> {
    let mut budget = Budget::default();
    let mut live = resolve(playable, stop, shared, quality, &mut budget)?;
    while let Some(opened) = &live {
        let stream_stop = hls::Stop::new(stop);
        let ended = play_live(opened, &stream_stop, open, settings, stop, &shared.playback);
        // The downloaders of this opening end with it, also when it failed
        // to open.
        stream_stop.set();
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match charge(next(stream_stop.interruption(), ended), &mut budget) {
            Next::End(result) => return result,
            Next::Reopen(_) => {}
            Next::Resolve(_) => live = resolve(playable, stop, shared, quality, &mut budget)?,
        }
    }
    Ok(())
}

/// Resolves `playable`: None once stopped meanwhile. A refused token, as when
/// the recipe's URL expired already, has it resolved again, within the
/// budget.
fn resolve(
    playable: &Playable,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
    budget: &mut Budget,
) -> Result<Option<Live>, Failure> {
    loop {
        let cause = match Live::resolve(playable, &hls::Stop::resolving(stop), shared.live.clone(), quality) {
            Ok(live) => return Ok(live),
            Err(Interruption::Refused(cause)) => cause,
            Err(other) => return Err(Failure::Live(other)),
        };
        if let Next::End(result) = charge(Next::Resolve(cause), budget) {
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
        Next::Reopen(Reopening::Failed(error)) => match budget.spend() {
            Some(spent) => {
                log!("live reopened from the live edge after an error ({spent}/{MAX_REOPENINGS}): {error}");
                Next::Reopen(Reopening::Failed(error))
            }
            None => Next::End(Err(error.into())),
        },
        Next::Reopen(ref reason) => {
            log!("live reopened from the live edge: {reason}");
            outcome
        }
        Next::Resolve(cause) => match budget.spend() {
            Some(spent) => {
                log!("stream to resolve again ({spent}/{MAX_REOPENINGS}): {cause}");
                Next::Resolve(cause)
            }
            None => Next::End(Err(Failure::RefusedAgain(cause))),
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
) -> Result<(), playback::Error> {
    let media = Media::open(path).map_err(playback::Error::Demuxer)?;
    let reader = Reader::start(Demux::new(media, None), settings.loop_playback)?;
    let ended = feeder::play(&reader, false, open, settings, stop, stats);
    reader.finish();
    ended?;
    Ok(())
}

fn play_live(
    live: &Live,
    stream_stop: &hls::Stop,
    open: &OpenBackend,
    settings: &Settings,
    stop: &AtomicBool,
    stats: &Mutex<playback::Stats>,
) -> Result<Ended, playback::Error> {
    let streams = live.open(stream_stop).map_err(|e| playback::Error::Thread(e.to_string()))?;
    let main = Media::open_stream(Box::new(streams.main)).map_err(playback::Error::Demuxer)?;
    let audio = streams.audio.map(|a| Media::open_stream(Box::new(a))).transpose().map_err(playback::Error::Demuxer)?;
    let reader = Reader::start(Demux::new(main, audio), false)?;
    let ended = feeder::play(&reader, true, open, settings, stop, stats);
    // The reader may be waiting on the downloads for data: they stop first.
    stream_stop.set();
    reader.finish();
    ended
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::{Language, Localized};
    use crate::net;
    use url::Url;

    /// A cause of a refused token, as a refused answer gives it.
    fn refused() -> Cause {
        Cause::Network(net::Error::empty(&Url::parse("https://a.invalid/x").unwrap()))
    }

    #[test]
    fn live_interruptions_come_first() {
        assert_eq!(next(Some(Interruption::Refused(refused())), Ok(Ended::Done)), Next::Resolve(refused()));
        let stall = Stall { idle: Duration::from_secs(8), last: 15, expected: 16 };
        let failing = Err(playback::Error::DemuxerGone);
        assert_eq!(next(Some(Interruption::Stuck(stall.clone())), failing), Next::Reopen(Reopening::Stuck(stall)));
        let unsupported = Interruption::Unsupported(Cause::KeyLength);
        let ended = next(Some(unsupported.clone()), Ok(Ended::Done));
        assert_eq!(ended, Next::End(Err(Failure::Live(unsupported))));
        let Next::End(Err(failure)) = ended else { unreachable!() };
        assert_eq!(failure.text(Language::Italian), "non supportato: chiave AES-128 non di 16 byte");
    }

    #[test]
    fn playback_failures_reopen_unless_unsupported() {
        assert_eq!(next(None, Ok(Ended::Done)), Next::End(Ok(())));
        let starved = next(None, Ok(Ended::Retune(Duration::from_secs(12))));
        assert_eq!(starved, Next::Reopen(Reopening::Starved(Duration::from_secs(12))));
        assert_eq!(Reopening::Starved(Duration::from_secs(12)).to_string(), "12 s without data");
        let frozen = next(None, Err(playback::Error::DecoderWontResume));
        assert_eq!(frozen, Next::Reopen(Reopening::Failed(playback::Error::DecoderWontResume)));
        let codec = playback::Error::VideoCodec(Some("mpeg2video 720×576".into()));
        assert_eq!(next(None, Err(codec.clone())), Next::End(Err(Failure::Playback(codec))));
    }

    #[test]
    fn past_the_budget_a_failure_ends_playback() {
        let mut budget = Budget::default();
        for _ in 0..MAX_REOPENINGS {
            assert!(matches!(charge(Next::Resolve(refused()), &mut budget), Next::Resolve(_)));
        }
        assert_eq!(charge(Next::Resolve(refused()), &mut budget), Next::End(Err(Failure::RefusedAgain(refused()))));
        let error = playback::Error::DecoderWontResume;
        assert_eq!(charge(Next::Reopen(Reopening::Failed(error.clone())), &mut budget), Next::End(Err(error.into())));
        let starved = Next::Reopen(Reopening::Starved(Duration::from_secs(12)));
        assert_eq!(charge(starved, &mut budget), Next::Reopen(Reopening::Starved(Duration::from_secs(12))));
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
