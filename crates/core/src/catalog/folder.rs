//! The folders of the list: a source's catalogue, read only when the viewer
//! opens its folder, never at start. Its copy shows at once; the source is
//! asked again on a thread of its own when the copy comes from an earlier
//! start or is older than AGE, and the items that come replace it and are
//! saved. A source that fails leaves the copy, if any, and says why.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use super::{COPIES, Channel, Folder, Play, by_start, save};
use crate::log::log;
use crate::net::Client;
use crate::source::run::{Cause, Engine, Failure, MAX_ANSWER_BYTES, Outcome, Request};
use crate::source::{Item, Selection};

/// How old a catalogue may be when its folder opens: events start and end,
/// and their signals come shortly before.
pub const AGE: Duration = Duration::from_secs(2 * 60);
/// The end of the name of a catalogue's copy: its items, as JSON.
pub(super) const COPY: &str = ".catalogue.json";

/// What asking a folder's source brought.
#[derive(Debug)]
pub struct Update {
    /// The folder's name in the list.
    pub name: String,
    /// Its channels, to show instead of those shown, or why none came.
    pub outcome: Result<Vec<Channel>, Failure>,
}

pub struct Folders {
    /// Where the copies are, in the data directory.
    dir: PathBuf,
    /// AGE, but in the tests.
    age: Duration,
    /// When each folder's catalogue came, in this start.
    read: HashMap<String, Instant>,
    /// The folders whose source is being asked.
    asking: HashSet<String>,
    sender: Sender<Update>,
    updates: Receiver<Update>,
}

impl Folders {
    /// The folders whose copies `data_dir` keeps.
    pub fn new(data_dir: &Path) -> Self {
        let (sender, updates) = channel();
        Self { dir: data_dir.join(COPIES), age: AGE, read: HashMap::new(), asking: HashSet::new(), sender, updates }
    }

    /// The channels of the copy of `folder`'s catalogue, if any, to show at
    /// once; the source is asked again if the copy is old.
    pub fn open(&mut self, folder: &Folder) -> Option<Vec<Channel>> {
        self.refresh(folder);
        self.saved(folder)
    }

    /// The channels of the copy of `folder`'s catalogue, if it was ever
    /// read, without asking its source.
    pub fn saved(&self, folder: &Folder) -> Option<Vec<Channel>> {
        let text = std::fs::read_to_string(self.dir.join(copy_name(&folder.name))).ok()?;
        let fields: Vec<BTreeMap<String, String>> = serde_json::from_str(&text).ok()?;
        Some(channels_of(folder, fields.into_iter().map(|fields| Item { fields }).collect()))
    }

    /// Asks `folder`'s source for its catalogue again when the one shown
    /// came more than AGE ago, or from an earlier start, unless it is
    /// being asked already; returns whether it asked.
    pub fn refresh(&mut self, folder: &Folder) -> bool {
        let fresh = self.read.get(&folder.name).is_some_and(|read| read.elapsed() < self.age);
        if fresh || !self.asking.insert(folder.name.clone()) {
            return false;
        }
        ask(folder.clone(), self.dir.clone(), self.sender.clone());
        true
    }

    /// Whether the source of the folder `name` is being asked.
    pub fn asking(&self, name: &str) -> bool {
        self.asking.contains(name)
    }

    /// What the next answer of a source brought, once it came.
    pub fn poll(&mut self) -> Option<Update> {
        let update = self.updates.try_recv().ok()?;
        self.asking.remove(&update.name);
        // A failure is tried again only AGE later, not at every opening.
        self.read.insert(update.name.clone(), Instant::now());
        Some(update)
    }
}

/// Asks `folder`'s source for its catalogue on a thread, which saves it in
/// `dir` and sends the channels to `updates`.
fn ask(folder: Folder, dir: PathBuf, updates: Sender<Update>) {
    let failed = updates.clone();
    let name = folder.name.clone();
    let started = std::thread::Builder::new().name("catalogue".into()).spawn(move || {
        let outcome = read(&folder, &dir);
        let _ = updates.send(Update { name: folder.name, outcome });
    });
    if let Err(error) = started {
        let _ = failed.send(Update { name, outcome: Err(failure(error.to_string())) });
    }
}

/// The channels of `folder`'s catalogue, saved in `dir`.
fn read(folder: &Folder, dir: &Path) -> Result<Vec<Channel>, Failure> {
    let client = Client::new().map_err(failure)?;
    let fetch = |r: &Request| client.send(r.method, &r.url, &r.headers, r.body.as_deref(), MAX_ANSWER_BYTES);
    let items = Engine::new(&folder.source, &folder.device, fetch, || false).catalogue();
    let items = items.inspect_err(|failure| log!("catalogue of {} not read: {failure}", folder.name))?;
    let copy: Vec<_> = items.iter().map(|item| &item.fields).collect();
    save(dir, &copy_name(&folder.name), &serde_json::to_string(&copy).unwrap_or_default());
    Ok(channels_of(folder, items))
}

/// Why no request could be made, as the system says.
fn failure(detail: String) -> Failure {
    Failure { outcome: Outcome::NetworkError, at: "catalogue".into(), cause: Cause::Problem(detail) }
}

/// The file name of the copy of the catalogue of the folder `name`.
pub(super) fn copy_name(name: &str) -> String {
    format!("{name}{COPY}")
}

/// The items of a source's catalogue as channels of `folder`'s source:
/// those with a start time first, by time, then the others in the
/// catalogue's order.
fn channels_of(folder: &Folder, items: Vec<Item>) -> Vec<Channel> {
    let mut channels: Vec<Channel> = items
        .into_iter()
        .map(|item| Channel {
            name: item.fields.get("title").cloned(),
            logo: item.fields.get("logo").cloned(),
            group: item.fields.get("group").cloned(),
            starts_at: item.starts_at(),
            play: Ok(Play::Source(Selection { source: folder.source.clone(), item, device: folder.device.clone(), candidate: None })),
        })
        .collect();
    by_start(&mut channels);
    channels
}

#[cfg(test)]
mod tests;
