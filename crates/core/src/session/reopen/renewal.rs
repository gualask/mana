//! A refused stream given anew. A stream the list gives cannot change: the
//! same one is tried again after a pause, a few times. One a source gives
//! is resolved again, within the budget; when the source gives the very
//! stream just refused, it is tried once more, as a server may authorize
//! the device when asked, then playback ends: asking again cannot change
//! it (Nuvio drops a re-resolved URL equal to the failed one too).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::policy::{Budget, MAX_REOPENINGS};
use crate::live::Cause;
use crate::live::playable::Playable;
use crate::log::log;
use crate::session::stats::Failure;

/// The pauses before each attempt at a stream of the list refused again,
/// growing as Media3 spaces its retries of a playlist (1 s more each, at
/// most 5 s): an IPTV server may still count the connection of the
/// channel just left. hls.js and FFmpeg do not retry a refusal at all.
#[cfg(not(test))]
pub const PAUSES: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(3), Duration::from_secs(5)];
#[cfg(test)]
pub const PAUSES: [Duration; 3] = [Duration::from_millis(10), Duration::from_millis(30), Duration::from_millis(50)];

/// How a refused stream is given anew.
pub(super) enum Renewal<'a> {
    /// A stream the list gives: the same, after a pause.
    Again { tries: usize },
    /// A stream a source gives: resolved again by `resolve`.
    Resolve { resolve: &'a mut dyn FnMut() -> Result<Playable, Failure>, repeated: bool },
}

impl<'a> Renewal<'a> {
    pub(super) fn again() -> Self {
        Self::Again { tries: 0 }
    }

    pub(super) fn resolve(resolve: &'a mut dyn FnMut() -> Result<Playable, Failure>) -> Self {
        Self::Resolve { resolve, repeated: false }
    }

    /// The stream to open after `refused` was refused for `cause`; None
    /// once `stop` is set meanwhile.
    pub(super) fn renew(
        &mut self,
        refused: &Playable,
        cause: Cause,
        budget: &mut Budget,
        stop: &AtomicBool,
    ) -> Result<Option<Playable>, Failure> {
        match self {
            Self::Again { tries } => retry(tries, refused, cause, stop),
            Self::Resolve { resolve, repeated } => resolve_again(*resolve, repeated, refused, cause, budget).map(Some),
        }
    }

    /// The stream played since the last refusal: the next one starts a new
    /// series.
    pub(super) fn played(&mut self) {
        match self {
            Self::Again { tries } => *tries = 0,
            Self::Resolve { repeated, .. } => *repeated = false,
        }
    }
}

/// `refused` again after the next pause, the `tries`-th: None once `stop`
/// is set meanwhile; past the pauses, playback ends.
fn retry(tries: &mut usize, refused: &Playable, cause: Cause, stop: &AtomicBool) -> Result<Option<Playable>, Failure> {
    let Some(&pause) = PAUSES.get(*tries) else { return Err(Failure::RefusedRetried(cause)) };
    *tries += 1;
    log!("stream refused, tried again in {} ms ({tries}/{}): {cause}", pause.as_millis(), PAUSES.len());
    Ok(wait(pause, stop).then(|| refused.clone()))
}

/// The stream `resolve` gives in place of `refused`, within `budget`; given
/// the same once more after it was `repeated`, playback ends.
fn resolve_again(
    resolve: &mut dyn FnMut() -> Result<Playable, Failure>,
    repeated: &mut bool,
    refused: &Playable,
    cause: Cause,
    budget: &mut Budget,
) -> Result<Playable, Failure> {
    let Some(spent) = budget.spend() else { return Err(Failure::RefusedAgain(cause)) };
    log!("stream to resolve again ({spent}/{MAX_REOPENINGS}): {cause}");
    let renewed = resolve()?;
    if renewed != *refused {
        *repeated = false;
    } else if std::mem::replace(repeated, true) {
        return Err(Failure::RefusedSame(cause));
    }
    Ok(renewed)
}

/// Waits `pause` unless `stop` is set first; whether it was not.
fn wait(pause: Duration, stop: &AtomicBool) -> bool {
    let until = Instant::now() + pause;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10).min(until - Instant::now()));
    }
    !stop.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net;
    use url::Url;

    fn stream(url: &str) -> Playable {
        Playable::direct(Url::parse(url).unwrap(), Vec::new())
    }

    fn refused() -> Cause {
        Cause::Network(net::Error::with_status(&Url::parse("https://a.invalid/x").unwrap(), 403))
    }

    #[test]
    fn a_stream_of_the_list_is_tried_again_a_few_times_unless_it_played() {
        let (mut budget, stop) = (Budget::default(), AtomicBool::new(false));
        let refused_stream = stream("https://a.invalid/x.m3u8");
        let mut renewal = Renewal::again();
        for _ in 0..PAUSES.len() {
            assert_eq!(renewal.renew(&refused_stream, refused(), &mut budget, &stop), Ok(Some(refused_stream.clone())));
        }
        let ended = renewal.renew(&refused_stream, refused(), &mut budget, &stop);
        assert_eq!(ended, Err(Failure::RefusedRetried(refused())));
        renewal.played();
        assert_eq!(renewal.renew(&refused_stream, refused(), &mut budget, &stop), Ok(Some(refused_stream.clone())));
        stop.store(true, Ordering::Relaxed);
        assert_eq!(renewal.renew(&refused_stream, refused(), &mut budget, &stop), Ok(None));
    }

    #[test]
    fn a_source_giving_the_stream_just_refused_has_it_tried_once_more() {
        let (mut budget, stop) = (Budget::default(), AtomicBool::new(false));
        let (old, new) = (stream("https://a.invalid/x.m3u8?t=1"), stream("https://a.invalid/x.m3u8?t=2"));
        let mut given = vec![old.clone(), old.clone(), new.clone(), new.clone()].into_iter();
        let mut resolve = || Ok(given.next().unwrap());
        let mut renewal = Renewal::resolve(&mut resolve);
        assert_eq!(renewal.renew(&old, refused(), &mut budget, &stop), Ok(Some(old.clone())));
        assert_eq!(renewal.renew(&old, refused(), &mut budget, &stop), Err(Failure::RefusedSame(refused())));
        // A new stream, then the same once more: a series of its own.
        assert_eq!(renewal.renew(&old, refused(), &mut budget, &stop), Ok(Some(new.clone())));
        assert_eq!(renewal.renew(&new, refused(), &mut budget, &stop), Ok(Some(new.clone())));
    }

    #[test]
    fn a_source_s_resolutions_spend_the_budget() {
        let (mut budget, stop) = (Budget::default(), AtomicBool::new(false));
        let mut count = 0;
        let mut resolve = || {
            count += 1;
            Ok(stream(&format!("https://a.invalid/x.m3u8?t={count}")))
        };
        let mut renewal = Renewal::resolve(&mut resolve);
        let refused_stream = stream("https://a.invalid/x.m3u8?t=0");
        for _ in 0..MAX_REOPENINGS {
            assert!(matches!(renewal.renew(&refused_stream, refused(), &mut budget, &stop), Ok(Some(_))));
        }
        assert_eq!(renewal.renew(&refused_stream, refused(), &mut budget, &stop), Err(Failure::RefusedAgain(refused())));
    }
}
