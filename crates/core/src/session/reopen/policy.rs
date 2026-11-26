//! When a live stream is opened again: how an opening ended, by live's
//! account then by playback's, and the budget of reopenings and
//! resolutions past which playback ends.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::live::{Cause, Interruption, Stall};
use crate::log::log;
use crate::playback::{self, feeder::Ended};
use crate::session::stats::Failure;

/// A live stream is opened again after a failure, or resolved again after
/// a refused token, at most this many times within REOPEN_WINDOW; then
/// playback ends. A frozen decoder takes 15 s to be noticed, so the window
/// must hold more than that many of them: one stuck every 20 s ends after
/// about two minutes, an occasional one is always recovered.
pub const MAX_REOPENINGS: usize = 6;
const REOPEN_WINDOW: Duration = Duration::from_secs(300);
/// What to do once an opening of a live stream ended.
#[derive(Debug, PartialEq)]
pub(super) enum Next {
    /// The stream ended, or cannot be played.
    End(Result<(), Failure>),
    /// Open it again from the live edge.
    Reopen(Reopening),
    /// Resolve it again, then open it: its token was refused.
    Resolve(Cause),
}

/// Why a live stream is opened again.
#[derive(Debug, PartialEq)]
pub(super) enum Reopening {
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
pub(super) fn next(interruption: Option<Interruption>, ended: Result<Ended, playback::Error>) -> Next {
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
pub(super) struct Budget {
    spent: VecDeque<Instant>,
}

impl Budget {
    /// Spends one; None once MAX_REOPENINGS are spent within the window,
    /// else how many are.
    pub(super) fn spend(&mut self) -> Option<usize> {
        self.spent.retain(|at| at.elapsed() < REOPEN_WINDOW);
        if self.spent.len() >= MAX_REOPENINGS {
            return None;
        }
        self.spent.push_back(Instant::now());
        Some(self.spent.len())
    }
}

/// Logs a reopening and charges it to `budget` when it counts, after an
/// error; past the budget, playback ends instead. A resolution is passed
/// on: the renewal charges it (renewal.rs).
pub(super) fn charge(outcome: Next, budget: &mut Budget) -> Next {
    match outcome {
        Next::End(_) | Next::Resolve(_) => outcome,
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
    }
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
        let error = playback::Error::DecoderWontResume;
        for _ in 0..MAX_REOPENINGS {
            assert!(matches!(charge(Next::Reopen(Reopening::Failed(error.clone())), &mut budget), Next::Reopen(_)));
        }
        assert_eq!(charge(Next::Resolve(refused()), &mut budget), Next::Resolve(refused()));
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
