//! The window's state: the levels of the list (levels.rs) and what the
//! keys do with them (keys.rs); what the threads bring, the list, a
//! folder's events, an event's signals; what plays, through the core's
//! viewing session; and what the status says.

use std::path::Path;
use std::time::Duration;

use mana_core::catalog::Channel;
use mana_core::catalog::folder::Folders;
use mana_core::catalog::update::{self, Outcome, Updater};
use mana_core::language::{Language, Localized};
use mana_core::session::{Candidates, Program, Session, State as Viewing, chosen_at_once};
use mana_core::source::Selection;
use mana_core::text;
use rune::{NativeSurface, Tx};
use url::Url;

mod keys;
mod levels;

pub use levels::{Action, Kind, Level};
use levels::{rows, signal_row};

use crate::awake::Awake;
use crate::backend;

/// How often to look at what the threads brought. The video's pictures wake
/// the window themselves: since anomalyco/rune#1141 was fixed, every picture
/// pushed at 50 fps is drawn without the window asking for frames.
const IDLE: Duration = Duration::from_millis(100);

/// What the status says, kept as it happened: it reads in the language of
/// each frame.
pub enum Note {
    None,
    ListDownloading,
    ListUnavailable(update::Failure),
    ListNotSet,
    FolderLoading,
    FolderFailed(mana_core::source::run::Failure),
    SignalsAsking,
    SignalsNone(mana_core::source::run::Failure),
    Unplayable(String, mana_core::catalog::ChannelError),
    NotStarted(mana_core::session::Failure),
}

pub struct App {
    pub surface: NativeSurface,
    pub language: Language,
    pub levels: Vec<Level>,
    pub list_open: bool,
    /// Changes with every change of rows: Rune reads the keys of virtual
    /// rows again only then.
    pub version: usize,
    revealed: Option<String>,
    updater: Updater,
    folders: Folders,
    asking: Option<(String, Selection, Candidates)>,
    pub session: Session,
    pub note: Note,
    /// The display kept awake while a video plays.
    awake: Option<Awake>,
}

impl App {
    pub fn new(data: &Path, list: Option<Url>, language: Language) -> Self {
        let surface = NativeSurface::new();
        let (saved, updater) = Updater::start(data, list);
        let rows = saved.as_deref().map(|channels| rows(channels, "l", language)).unwrap_or_default();
        let note = match () {
            _ if updater.url().is_none() => Note::ListNotSet,
            _ if rows.is_empty() => Note::ListDownloading,
            _ => Note::None,
        };
        Self {
            session: Session::new(backend::open(surface.clone())),
            surface,
            language,
            levels: vec![Level { kind: Kind::List, title: String::new(), rows, focus: 0 }],
            list_open: true,
            version: 0,
            revealed: None,
            updater,
            folders: Folders::new(data),
            asking: None,
            note,
            awake: None,
        }
    }

    pub fn top(&self) -> &Level {
        self.levels.last().expect("the list is always there")
    }

    fn top_mut(&mut self) -> &mut Level {
        self.version += 1;
        self.levels.last_mut().expect("the list is always there")
    }

    /// Takes what the threads brought and keeps the focused row in view;
    /// whether the window must be drawn again.
    pub fn poll(&mut self, tx: &mut Tx) -> bool {
        let changed = self.take_list() | self.take_folders() | self.take_signals();
        let playing = self.session.is_playing();
        if playing != self.awake.is_some() {
            self.awake = if playing { Awake::hold() } else { None };
        }
        tx.wake_after(IDLE);
        let wanted = self.list_open.then(|| self.top().rows.get(self.top().focus).map(|r| r.key.clone())).flatten();
        if self.revealed == wanted {
            return changed || self.session.is_playing();
        }
        if let Some(key) = &wanted {
            tx.scroll_into_view(key.clone());
        }
        self.revealed = wanted;
        true
    }

    fn take_list(&mut self) -> bool {
        let mut changed = false;
        while let Some(update) = self.updater.poll() {
            changed = true;
            match update.outcome {
                Outcome::Channels { channels, .. } => self.show_list(&channels),
                Outcome::Failed(failure) => self.note = Note::ListUnavailable(failure),
            }
        }
        changed
    }

    /// The list's channels, downloaded: the focus stays within them.
    fn show_list(&mut self, channels: &[Channel]) {
        let list = &mut self.levels[0];
        list.rows = rows(channels, "l", self.language);
        list.focus = list.focus.min(list.rows.len().saturating_sub(1));
        self.version += 1;
        if matches!(self.note, Note::ListDownloading | Note::ListUnavailable(_)) {
            self.note = Note::None;
        }
    }

    fn take_folders(&mut self) -> bool {
        let mut changed = false;
        while let Some(update) = self.folders.poll() {
            changed = true;
            // The answer of a folder no longer open is only saved.
            if !self.levels.iter().any(|l| l.kind == Kind::Folder(update.name.clone())) {
                continue;
            }
            match update.outcome {
                Ok(channels) => self.show_folder(&update.name, &channels),
                Err(failure) => self.note = Note::FolderFailed(failure),
            }
            self.version += 1;
        }
        changed
    }

    /// The events of the open folder `name`: the focus stays on the same
    /// event.
    fn show_folder(&mut self, name: &str, channels: &[Channel]) {
        let language = self.language;
        let Some(level) = self.levels.iter_mut().find(|l| l.kind == Kind::Folder(name.to_string())) else { return };
        let focused = level.rows.get(level.focus).map(|r| r.title.clone());
        level.rows = rows(channels, name, language);
        level.focus = focused.and_then(|title| level.rows.iter().position(|r| r.title == title)).unwrap_or(0);
        if matches!(self.note, Note::FolderLoading | Note::FolderFailed(_)) {
            self.note = Note::None;
        }
    }

    fn take_signals(&mut self) -> bool {
        let Some(answer) = self.asking.as_ref().and_then(|(_, _, asking)| asking.poll()) else { return false };
        let (name, selection, _) = self.asking.take().expect("asked");
        match answer {
            Ok(candidates) if chosen_at_once(&candidates).is_some() => {
                let chosen = Selection { candidate: chosen_at_once(&candidates).cloned(), ..selection };
                self.play(name, Program::Source(chosen));
            }
            Ok(candidates) => {
                let rows = candidates.into_iter().enumerate().map(|(i, c)| signal_row(i, c, &selection)).collect();
                let title = format!("{} · {name}", text!(self.language, "signals-title"));
                self.levels.push(Level { kind: Kind::Signals(name), title, rows, focus: 0 });
                self.version += 1;
                self.note = Note::None;
            }
            Err(failure) => self.note = Note::SignalsNone(failure),
        }
        true
    }

    /// The status line: the note, else the viewing's state.
    pub fn status(&self) -> Option<String> {
        let language = self.language;
        Some(match &self.note {
            Note::ListDownloading => text!(language, "home-list-downloading"),
            Note::ListUnavailable(reason) => text!(language, "home-list-unavailable", reason = reason.text(language)),
            Note::ListNotSet => text!(language, "desktop-list-not-set"),
            Note::FolderLoading => text!(language, "folder-loading"),
            Note::FolderFailed(failure) => text!(language, "folder-unavailable", reason = failure.text(language)),
            Note::SignalsAsking => text!(language, "signals-asking"),
            Note::SignalsNone(failure) => failure.text(language),
            Note::Unplayable(name, error) => format!("{name}: {}", error.text(language)),
            Note::NotStarted(failure) => failure.text(language),
            Note::None => {
                let stats = self.session.stats()?;
                match &stats.state {
                    Viewing::Starting => text!(language, "state-starting"),
                    Viewing::Playing => text!(language, "state-playing"),
                    Viewing::Ending => text!(language, "state-ending"),
                    Viewing::Stopped => text!(language, "state-stopped"),
                    Viewing::Failed(failure) => text!(language, "state-failed", failure = failure.text(language)),
                }
            }
        })
    }

    /// What plays, by name.
    pub fn playing(&self) -> Option<String> {
        self.session.stats().map(|stats| stats.name)
    }
}
