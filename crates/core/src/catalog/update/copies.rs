//! The copies of the list in the data directory: the list and the URL it
//! came from, each M3U list it includes, and the URL chosen; and the
//! download that writes them. The copies of the folders' catalogues are
//! there too ([folder](crate::catalog::folder)): a download removes those
//! of the folders no longer in the list.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use url::Url;

use super::{Failure, Job, Outcome, Update, list_url};
use crate::catalog::device::Device;
use crate::catalog::list::{self, Entry, List};
use crate::catalog::{COPIES, Channel, ChannelError, Folder, Play, folder, m3u, save};
use crate::language::Language;
use crate::log::log;
use crate::net::{self, Client, loggable};
/// The list's copy, and the URL it came from.
const LIST_COPY: &str = "list.toml";
const LIST_URL: &str = "list.url";
/// The URL chosen, kept across power-offs.
const CHOSEN_URL: &str = "chosen.url";
/// The extension of the copies of the included M3U lists.
const M3U_COPY: &str = ".m3u";
/// Unavailable channels named in the log, before only counting them.
const LOGGED_UNAVAILABLE: usize = 10;

/// The copies in their folder, and the device identifier for the sources.
pub(super) struct Copies {
    dir: PathBuf,
    device: Arc<Device>,
}

impl Copies {
    /// The copies kept in `dir`, the data directory, which also keeps the
    /// device identifier.
    pub(super) fn new(dir: &Path) -> Self {
        Self { dir: dir.join(COPIES), device: Arc::new(Device::new(dir.to_path_buf())) }
    }

    /// The download `job` asks for; the URL chosen is saved, after the
    /// copies, only when the list is TOML.
    pub(super) fn run(&self, job: &Job) -> Update {
        let outcome = match self.download(&job.url) {
            Outcome::Channels { channels, kept } if job.chosen => {
                let kept = save(&self.dir, CHOSEN_URL, job.url.as_str()) && kept;
                log!("list chosen: {}", loggable(&job.url));
                Outcome::Channels { channels, kept }
            }
            outcome => outcome,
        };
        Update { ticket: job.ticket, url: job.url.clone(), outcome }
    }

    /// The URL chosen last, if any.
    pub(super) fn chosen(&self) -> Option<Url> {
        let text = std::fs::read_to_string(self.dir.join(CHOSEN_URL)).ok()?;
        list_url(&text).inspect_err(|e| log!("list chosen not valid: {e}")).ok()
    }

    /// The channels of the list saved, when it came from `url` or no URL is
    /// asked for, with its inclusions from their copies.
    pub(super) fn saved(&self, url: Option<&Url>) -> Option<Vec<Channel>> {
        let from = std::fs::read_to_string(self.dir.join(LIST_URL)).ok()?;
        if url.is_some_and(|url| url.as_str() != from.trim()) {
            return None;
        }
        let text = std::fs::read_to_string(self.dir.join(LIST_COPY)).ok()?;
        let list = self.parse(&text).inspect_err(|e| log!("saved list not valid: {e}")).ok()?;
        let channels = self.assemble(list, |url| self.copy_of(url));
        log!("saved list: {} channels", channels.len());
        Some(channels)
    }

    /// The list at `url`, with its inclusions, saved when it is TOML.
    fn download(&self, url: &Url) -> Outcome {
        let (client, text, list) = match self.fetch_list(url) {
            Ok(fetched) => fetched,
            Err(reason) => {
                log!("list from {} not updated: {reason}", loggable(url));
                return Outcome::Failed(reason);
            }
        };
        let kept = save(&self.dir, LIST_COPY, &text) && save(&self.dir, LIST_URL, url.as_str());
        let included: Vec<String> = list.entries.iter().filter_map(copy_of_entry).collect();
        let channels = self.assemble(list, |included| self.download_included(&client, included));
        self.forget_others(&included);
        log!("list from {} updated: {} channels", loggable(url), channels.len());
        Outcome::Channels { channels, kept }
    }

    /// The client, the text at `url` and its list, when the text is TOML.
    fn fetch_list(&self, url: &Url) -> Result<(Client, String, List), Failure> {
        let client = Client::new().map_err(Failure::Client)?;
        let text = fetch(&client, url)?;
        let list = self.parse(&text).map_err(Failure::List)?;
        Ok((client, text, list))
    }

    /// The M3U list at `url`, saved, when it has channels; else its saved
    /// copy, if any.
    fn download_included(&self, client: &Client, url: &Url) -> Option<String> {
        match fetch(client, url) {
            Ok(text) if !m3u::parse(&text).is_empty() => {
                save(&self.dir, &copy_name(url), &text);
                Some(text)
            }
            outcome => {
                let reason = outcome.err().map_or_else(|| "no channels".into(), |e| e.to_string());
                log!("included list {} not downloaded: {reason}", loggable(url));
                self.copy_of(url).inspect(|_| log!("included list {} from the saved copy", loggable(url)))
            }
        }
    }

    fn parse(&self, text: &str) -> Result<List, list::Error> {
        let device = self.device.clone();
        list::parse(text, move || device.id().map(str::to_string))
    }

    /// The list's channels, each M3U inclusion replaced by the channels of
    /// the text `included` gives for it, left out when it gives none, and
    /// each source's catalogue by its folder. Logs what was left out and
    /// the channels that cannot be played.
    fn assemble(&self, list: List, included: impl Fn(&Url) -> Option<String>) -> Vec<Channel> {
        let mut skipped = list.skipped;
        let channels: Vec<Channel> = list
            .entries
            .into_iter()
            .flat_map(|entry| match entry {
                Entry::Channel(channel) => vec![channel],
                Entry::Include(url) => included(&url).map(|text| m3u::parse(&text)).unwrap_or_else(|| {
                    skipped.push(format!("included list {} skipped", loggable(&url)));
                    Vec::new()
                }),
                Entry::Folder(folder) => vec![folder_channel(folder)],
            })
            .collect();
        log_left_out(&skipped, &channels);
        channels
    }

    /// The saved copy of the M3U list at `url`.
    fn copy_of(&self, url: &Url) -> Option<String> {
        std::fs::read_to_string(self.dir.join(copy_name(url))).ok()
    }

    /// Removes the copies of M3U lists and catalogues no longer included.
    fn forget_others(&self, included: &[String]) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if (name.ends_with(M3U_COPY) || name.ends_with(folder::COPY)) && !included.contains(&name) {
                drop(std::fs::remove_file(&path));
            }
        }
    }
}

/// Logs what the list left out, and the channels that cannot be played.
fn log_left_out(skipped: &[String], channels: &[Channel]) {
    skipped.iter().for_each(|reason| log!("list: {reason}"));
    let unavailable: Vec<(&Channel, &ChannelError)> =
        channels.iter().filter_map(|c| c.play.as_ref().err().map(|error| (c, error))).collect();
    for (channel, error) in unavailable.iter().take(LOGGED_UNAVAILABLE) {
        log!("list: {} unavailable: {error}", channel.title(Language::English));
    }
    if unavailable.len() > LOGGED_UNAVAILABLE {
        log!("list: {} more channels unavailable", unavailable.len() - LOGGED_UNAVAILABLE);
    }
}

/// The name of the copy an inclusion keeps.
fn copy_of_entry(entry: &Entry) -> Option<String> {
    match entry {
        Entry::Include(url) => Some(copy_name(url)),
        Entry::Folder(folder) => Some(folder::copy_name(&folder.name)),
        Entry::Channel(_) => None,
    }
}

/// The row of `folder`: the source's name, opened to show its catalogue.
fn folder_channel(folder: Folder) -> Channel {
    let name = Some(folder.source.name.clone());
    Channel { name, logo: None, group: None, starts_at: None, play: Ok(Play::Folder(folder)) }
}

/// The file name of the copy of the M3U list at `url`: a hash of the URL
/// (FNV-1a), which may be too long for a name.
fn copy_name(url: &Url) -> String {
    let hash = url.as_str().bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}{M3U_COPY}")
}

/// The text at `url`, of at most MAX_TEXT_BYTES.
fn fetch(client: &Client, url: &Url) -> Result<String, Failure> {
    let body = client.get(url, net::MAX_TEXT_BYTES).map_err(Failure::Network)?;
    String::from_utf8(body).map_err(|_| Failure::NotUtf8)
}
