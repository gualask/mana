//! The copies of the list in the data directory: the list and the URL it
//! came from, each M3U list and source catalogue it includes, and the URL
//! chosen; and the download that writes them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use url::Url;

use super::{Failure, Job, Outcome, Update, list_url};
use crate::catalog::device::Device;
use crate::catalog::list::{self, Entry, List};
use crate::catalog::{Channel, ChannelError, Play, m3u};
use crate::language::Language;
use crate::log::log;
use crate::net::{self, Client, loggable};
use crate::source::run::{Engine, MAX_ANSWER_BYTES, Request};
use crate::source::{Item, Selection, date};

/// The folder of the copies, in the data directory.
const COPIES: &str = "list";
/// The list's copy, and the URL it came from.
const LIST_COPY: &str = "list.toml";
const LIST_URL: &str = "list.url";
/// The URL chosen, kept across power-offs.
const CHOSEN_URL: &str = "chosen.url";
/// The extension of the copies of the included M3U lists.
const M3U_COPY: &str = ".m3u";
/// The copy of a source's catalogue: its items, as JSON.
const CATALOGUE_COPY: &str = ".catalogue.json";
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
                let kept = self.save(CHOSEN_URL, job.url.as_str()) && kept;
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
        let channels = self.assemble(list, |url| self.copy_of(url), |name, _| self.catalogue_copy(name));
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
        let kept = self.save(LIST_COPY, &text) && self.save(LIST_URL, url.as_str());
        let included: Vec<String> = list.entries.iter().filter_map(copy_of_entry).collect();
        let channels = self.assemble(
            list,
            |included| self.download_included(&client, included),
            |name, selection| self.download_catalogue(&client, name, selection),
        );
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
                self.save(&copy_name(url), &text);
                Some(text)
            }
            outcome => {
                let reason = outcome.err().map_or_else(|| "no channels".into(), |e| e.to_string());
                log!("included list {} not downloaded: {reason}", loggable(url));
                self.copy_of(url).inspect(|_| log!("included list {} from the saved copy", loggable(url)))
            }
        }
    }

    /// The items of the catalogue of the source `name`, saved; else those
    /// of its saved copy, if any.
    fn download_catalogue(&self, client: &Client, name: &str, selection: &Selection) -> Option<Vec<Item>> {
        let fetch = |r: &Request| client.send(r.method, &r.url, &r.headers, r.body.as_deref(), MAX_ANSWER_BYTES);
        match Engine::new(&selection.source, &selection.device, fetch, || false).catalogue() {
            Ok(items) => {
                let copy: Vec<_> = items.iter().map(|item| &item.fields).collect();
                self.save(&catalogue_name(name), &serde_json::to_string(&copy).unwrap_or_default());
                Some(items)
            }
            Err(failure) => {
                log!("catalogue of {name} not read: {failure}");
                self.catalogue_copy(name).inspect(|_| log!("catalogue of {name} from the saved copy"))
            }
        }
    }

    /// The saved items of the catalogue of the source `name`.
    fn catalogue_copy(&self, name: &str) -> Option<Vec<Item>> {
        let text = std::fs::read_to_string(self.dir.join(catalogue_name(name))).ok()?;
        let fields: Vec<std::collections::BTreeMap<String, String>> = serde_json::from_str(&text).ok()?;
        Some(fields.into_iter().map(|fields| Item { fields }).collect())
    }

    fn parse(&self, text: &str) -> Result<List, list::Error> {
        let device = self.device.clone();
        list::parse(text, move || device.id().map(str::to_string))
    }

    /// The list's channels, each inclusion replaced by the channels of the
    /// M3U `included` gives for it, or of the items `catalogue` gives for a
    /// source; one that gives none is left out. Logs what was left out and
    /// the channels that cannot be played.
    fn assemble(
        &self,
        list: List,
        included: impl Fn(&Url) -> Option<String>,
        catalogue: impl Fn(&str, &Selection) -> Option<Vec<Item>>,
    ) -> Vec<Channel> {
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
                Entry::Catalogue(name, selection) => match catalogue(&name, &selection) {
                    Some(items) => channels_of(&selection, items),
                    None => {
                        skipped.push(format!("catalogue of {name} skipped"));
                        Vec::new()
                    }
                },
            })
            .collect();
        log_left_out(&skipped, &channels);
        channels
    }

    /// The saved copy of the M3U list at `url`.
    fn copy_of(&self, url: &Url) -> Option<String> {
        std::fs::read_to_string(self.dir.join(copy_name(url))).ok()
    }

    /// Writes the copy `name` whole, through a temporary file: a power-off
    /// meanwhile leaves the previous one. Returns whether it was written.
    fn save(&self, name: &str, text: &str) -> bool {
        let (path, temporary) = (self.dir.join(name), self.dir.join(format!("{name}.new")));
        let saved = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&temporary, text))
            .and_then(|()| std::fs::rename(&temporary, &path));
        if let Err(error) = &saved {
            log!("copy {} not saved: {error}", path.display());
        }
        saved.is_ok()
    }

    /// Removes the copies of M3U lists and catalogues no longer included.
    fn forget_others(&self, included: &[String]) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if (name.ends_with(M3U_COPY) || name.ends_with(CATALOGUE_COPY)) && !included.contains(&name) {
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
        Entry::Catalogue(name, _) => Some(catalogue_name(name)),
        Entry::Channel(_) => None,
    }
}

/// The file name of the copy of the catalogue of the source `name`.
fn catalogue_name(name: &str) -> String {
    format!("{name}{CATALOGUE_COPY}")
}

/// The items of a source's catalogue as channels of `selection`'s source:
/// those with a start time first, by time, then the others in the
/// catalogue's order.
fn channels_of(selection: &Selection, items: Vec<Item>) -> Vec<Channel> {
    let mut channels: Vec<Channel> = items
        .into_iter()
        .map(|item| Channel {
            name: item.fields.get("title").cloned(),
            logo: item.fields.get("logo").cloned(),
            group: item.fields.get("group").cloned(),
            starts_at: item.fields.get("starts_at").and_then(|at| date::instant(at)),
            play: Ok(Play::Source(Selection { item, ..selection.clone() })),
        })
        .collect();
    channels.sort_by_key(|channel| (channel.starts_at.is_none(), channel.starts_at));
    channels
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
