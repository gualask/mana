//! Why a download failed, and the consecutive failures `Downloader::retry`
//! counts and logs.

use std::time::Instant;

use super::super::playlist;
use crate::log::log;
use crate::net;

/// After this many consecutive failures, only every tenth one is logged.
const LOGGED_FAILURES: u32 = 3;

/// Why one attempt at a download failed, as `Downloader::retry` tells them
/// apart.
pub(in crate::live::hls) enum Failure {
    /// Worth retrying: the network, the server, a damaged answer.
    Transient(String),
    /// The URL's token refused: only a new resolution gives new URLs.
    Refused(String),
    /// HLS features this client does not have, or an answer larger than
    /// it takes: retrying cannot help.
    Unsupported(String),
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
            playlist::Error::Invalid(message) => Self::Transient(message),
            playlist::Error::Unsupported(message) => Self::Unsupported(message),
        }
    }
}

impl Failure {
    pub(in crate::live::hls) fn message(self) -> String {
        match self {
            Self::Transient(message) | Self::Refused(message) | Self::Unsupported(message) => message,
        }
    }
}

/// The consecutive failures of one download.
pub(super) struct Failures {
    count: u32,
    since: Option<Instant>,
    /// Failures before giving up; None retries for as long as it takes.
    attempts: Option<u32>,
    label: &'static str,
}

impl Failures {
    pub(super) fn new(attempts: Option<u32>, label: &'static str) -> Self {
        Self { count: 0, since: None, attempts, label }
    }

    /// Counts one and logs it, only every tenth past LOGGED_FAILURES; an
    /// error once there were as many as the attempts allowed.
    pub(super) fn count(&mut self, error: String) -> Result<(), Failure> {
        self.count += 1;
        self.since.get_or_insert_with(Instant::now);
        if self.count <= LOGGED_FAILURES || self.count.is_multiple_of(10) {
            log!("{}: {error} (tentativo {})", self.label, self.count);
        }
        match self.attempts {
            Some(max) if self.count >= max => Err(Failure::Transient(error)),
            _ => Ok(()),
        }
    }

    /// Logs how long the download failed, when it did.
    pub(super) fn recovered(&self) {
        if let Some(since) = self.since {
            log!("{}: di nuovo raggiungibile dopo {} s", self.label, since.elapsed().as_secs());
        }
    }
}
