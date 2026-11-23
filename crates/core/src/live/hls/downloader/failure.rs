//! Why a download failed, and the consecutive failures `Downloader::retry`
//! counts and logs.

use std::time::Instant;

use super::super::download::Patience;
use super::super::playlist;
use crate::live::Cause;
use crate::log::log;
use crate::net;

/// After this many consecutive failures, only every tenth one is logged.
const LOGGED_FAILURES: u32 = 3;

/// Why one attempt at a download failed, as `Downloader::retry` tells them
/// apart.
pub(in crate::live::hls) enum Failure {
    /// Worth retrying: the network, the server, a damaged answer.
    Transient(Cause),
    /// The URL's token refused: only a new resolution gives new URLs.
    Refused(Cause),
    /// HLS features this client does not have, or an answer larger than
    /// it takes: retrying cannot help.
    Unsupported(Cause),
}

impl From<net::Error> for Failure {
    fn from(error: net::Error) -> Self {
        if error.is_refused() {
            Self::Refused(error.into())
        } else if error.is_oversized() {
            Self::Unsupported(error.into())
        } else {
            Self::Transient(error.into())
        }
    }
}

impl From<playlist::Error> for Failure {
    fn from(error: playlist::Error) -> Self {
        match error {
            playlist::Error::Invalid(_) => Self::Transient(error.into()),
            playlist::Error::Unsupported(_) => Self::Unsupported(error.into()),
        }
    }
}

impl Failure {
    pub(in crate::live::hls) fn cause(self) -> Cause {
        match self {
            Self::Transient(cause) | Self::Refused(cause) | Self::Unsupported(cause) => cause,
        }
    }
}

/// The consecutive failures of one download.
pub(super) struct Failures {
    count: u32,
    /// The first attempt.
    started: Instant,
    since: Option<Instant>,
    /// When to give up; None retries for as long as it takes.
    patience: Option<Patience>,
    label: &'static str,
}

impl Failures {
    pub(super) fn new(patience: Option<Patience>, label: &'static str) -> Self {
        Self { count: 0, started: Instant::now(), since: None, patience, label }
    }

    /// Counts one and logs it, only every tenth past LOGGED_FAILURES; an
    /// error once the patience given is over.
    pub(super) fn count(&mut self, error: Cause) -> Result<(), Failure> {
        self.count += 1;
        self.since.get_or_insert_with(Instant::now);
        if self.count <= LOGGED_FAILURES || self.count.is_multiple_of(10) {
            log!("{}: {error} (attempt {})", self.label, self.count);
        }
        match self.patience {
            Some(p) if self.count >= p.attempts || self.started.elapsed() >= p.within => Err(Failure::Transient(error)),
            _ => Ok(()),
        }
    }

    /// Logs how long the download failed, when it did.
    pub(super) fn recovered(&self) {
        if let Some(since) = self.since {
            log!("{}: reachable again after {} s", self.label, since.elapsed().as_secs());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn cause() -> Cause {
        Cause::Network(net::Error::with_status(&url::Url::parse("http://a.invalid/x").unwrap(), 503))
    }

    #[test]
    fn patience_ends_at_its_attempts_or_its_time_whichever_comes_first() {
        let mut failures = Failures::new(Some(Patience { attempts: 3, within: Duration::MAX }), "test");
        assert!(failures.count(cause()).is_ok() && failures.count(cause()).is_ok());
        assert!(failures.count(cause()).is_err(), "third failure");
        let mut failures = Failures::new(Some(Patience { attempts: 3, within: Duration::ZERO }), "test");
        assert!(failures.count(cause()).is_err(), "time over at the first failure");
        let mut failures = Failures::new(None, "test");
        assert!((0..20).all(|_| failures.count(cause()).is_ok()), "without patience, for as long as it takes");
    }
}
