//! The stable channel list's life: the copy saved at the last update,
//! shown at once, then the list downloaded again in the background from
//! its URL, which replaces it only when it is TOML. Each M3U list it
//! includes is downloaded too, and kept: one that cannot be downloaded
//! uses its own copy, else it is left out. The URL is the one the settings
//! give, else the one last chosen (`Updater::change`) whose list was TOML;
//! `Updater::refresh` downloads it again on demand.
//! Downloads run one after another, in the order asked, so the last URL
//! asked is the one on disk. Copies live in the data directory the app
//! gives; when that cannot be written, the list works in memory for the
//! session.

mod copies;

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};

use url::Url;

use super::{Channel, list};
use crate::language::{Language, Localized, display_in_english};
use crate::log::log;
use crate::net;
use crate::text;
use copies::Copies;

/// The longest list URL taken.
pub const MAX_URL: usize = net::MAX_LINE_VALUE;

/// The URL `text` gives for a channel list: `http` or `https`, without
/// control characters, of at most MAX_URL bytes.
pub fn list_url(text: &str) -> Result<Url, UrlError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(UrlError::Empty);
    }
    if text.len() > MAX_URL {
        return Err(UrlError::TooLong(text.len()));
    }
    if text.chars().any(char::is_control) {
        return Err(UrlError::Control);
    }
    let url = Url::parse(text).map_err(|e| UrlError::Invalid(e.to_string()))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        scheme => Err(UrlError::Scheme(scheme.to_string())),
    }
}

/// Why a text is not a list's URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    /// Its length, over MAX_URL.
    TooLong(usize),
    Control,
    /// What the url crate says.
    Invalid(String),
    /// A scheme other than http and https.
    Scheme(String),
}

impl Localized for UrlError {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Empty => text!(language, "list-url-empty"),
            Self::TooLong(length) => text!(language, "list-url-too-long", length = length, max = MAX_URL),
            Self::Control => text!(language, "list-url-control"),
            Self::Invalid(detail) => text!(language, "list-url-invalid", detail = detail),
            Self::Scheme(scheme) => text!(language, "list-url-scheme", scheme = scheme),
        }
    }
}

/// Why a download of the list was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Network(net::Error),
    /// The HTTP client could not be made, as TLS says.
    Client(String),
    NotUtf8,
    List(list::Error),
    /// The thread that downloads the lists did not start.
    NotStarted,
}

impl Localized for Failure {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Network(error) => error.text(language),
            Self::Client(detail) => detail.clone(),
            Self::NotUtf8 => text!(language, "list-not-utf8"),
            Self::List(error) => error.text(language),
            Self::NotStarted => text!(language, "list-update-not-started"),
        }
    }
}

display_in_english!(UrlError, Failure);

/// A download asked, to know its outcome among the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket(u64);

/// What a download of the list brought.
#[derive(Debug)]
pub struct Update {
    pub ticket: Ticket,
    pub url: Url,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub enum Outcome {
    /// The list, to show instead of the one shown; `kept` when it will
    /// still be there after a power-off.
    Channels { channels: Vec<Channel>, kept: bool },
    /// Why it was not taken: the list shown, if any, stays.
    Failed(Failure),
}

/// A download asked: of the URL the app started with, or of one chosen.
struct Job {
    ticket: Ticket,
    url: Url,
    chosen: bool,
}

pub struct Updater {
    /// The downloads asked, to the thread that runs them in order; none
    /// when the thread could not start.
    jobs: Option<Sender<Job>>,
    updates: Receiver<Update>,
    /// For the failures of downloads that never ran.
    failures: Sender<Update>,
    /// The URL of the list in use, or being downloaded at start.
    url: Option<Url>,
    /// Downloads asked whose outcome has not been polled yet.
    pending: usize,
    /// The ticket of the next download asked.
    next: u64,
}

impl Updater {
    /// The channels of the copy saved for the list's URL (any copy, without
    /// a URL), if any, and the updater that downloads the list in the
    /// background. The URL is `configured`, as the settings give it, else
    /// the one last chosen. `dir` keeps the copies, the URL chosen and the
    /// device identifier.
    pub fn start(dir: &Path, configured: Option<Url>) -> (Option<Vec<Channel>>, Self) {
        let copies = Copies::new(dir);
        let url = configured.or_else(|| copies.chosen());
        let saved = copies.saved(url.as_ref());
        let (failures, updates) = channel();
        let jobs = run_in_order(copies, failures.clone());
        let mut updater = Self { jobs, updates, failures, url: None, pending: 0, next: 0 };
        if let Some(url) = url {
            updater.url = Some(url.clone());
            updater.ask(url, false);
        }
        (saved, updater)
    }

    /// Downloads the list at `url` after those asked before; when it is
    /// TOML it replaces the list, and its URL is the one chosen from then on.
    pub fn change(&mut self, url: Url) -> Ticket {
        self.ask(url, true)
    }

    /// Downloads the list in use again, after those asked before, without
    /// changing the URL chosen; none when there is no URL.
    pub fn refresh(&mut self) -> Option<Ticket> {
        let url = self.url.clone()?;
        Some(self.ask(url, false))
    }

    fn ask(&mut self, url: Url, chosen: bool) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        self.pending += 1;
        let job = Job { ticket, url, chosen };
        let refused = match &self.jobs {
            Some(jobs) => jobs.send(job).err().map(|e| e.0),
            None => Some(job),
        };
        if let Some(Job { ticket, url, .. }) = refused {
            let outcome = Outcome::Failed(Failure::NotStarted);
            drop(self.failures.send(Update { ticket, url, outcome }));
        }
        ticket
    }

    /// The outcome of the next download asked, once it ended.
    pub fn poll(&mut self) -> Option<Update> {
        let update = self.updates.try_recv().ok()?;
        self.pending -= 1;
        if let Outcome::Channels { .. } = &update.outcome {
            self.url = Some(update.url.clone());
        }
        Some(update)
    }

    /// The URL of the list shown, or of the one asked at start until it
    /// comes; none when the settings give none and none was chosen.
    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    /// Whether a download asked has not ended yet.
    pub fn downloading(&self) -> bool {
        self.pending > 0
    }
}

/// Starts the thread that runs the downloads sent to it one after another,
/// each outcome to `updates`, until the updater is dropped; none when the
/// thread cannot start.
fn run_in_order(copies: Copies, updates: Sender<Update>) -> Option<Sender<Job>> {
    let (jobs, queue) = channel::<Job>();
    let spawned = std::thread::Builder::new().name("list".into()).spawn(move || {
        for job in queue {
            if updates.send(copies.run(&job)).is_err() {
                return;
            }
        }
    });
    spawned.inspect_err(|e| log!("list: thread not started: {e}")).ok().map(|_| jobs)
}

#[cfg(test)]
pub(super) mod tests;
