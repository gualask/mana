//! The levels the list goes through, as on the TV: the channels, a
//! folder's events, an event's signals; each a list of rows.

use mana_core::catalog::{Channel, Play};
use mana_core::clock;
use mana_core::language::Language;
use mana_core::source::Selection;
use mana_core::source::run::Candidate;
use mana_core::text;

/// What OK does on a row.
pub enum Action {
    Channel(Channel),
    Signal(Selection),
}

pub struct Row {
    pub key: String,
    pub title: String,
    /// The event's time in local time, if it is one.
    pub time: Option<String>,
    pub detail: String,
    pub action: Action,
}

#[derive(PartialEq)]
pub enum Kind {
    List,
    /// The events of the folder with this name in the list.
    Folder(String),
    /// The signals of the event with this name.
    Signals(String),
}

pub struct Level {
    pub kind: Kind,
    pub title: String,
    pub rows: Vec<Row>,
    pub focus: usize,
}

/// The rows of `channels`, keyed with `prefix`.
pub(super) fn rows(channels: &[Channel], prefix: &str, language: Language) -> Vec<Row> {
    let now = clock::now();
    let row = |(i, channel): (usize, &Channel)| {
        let folder = matches!(channel.play, Ok(Play::Folder(_)));
        let detail = if folder { text!(language, "desktop-folder") } else { channel.group.clone().unwrap_or_default() };
        Row {
            key: format!("{prefix}-{i}"),
            title: channel.title(language).to_string(),
            time: channel.starts_at.map(|at| clock::time(at, now)),
            detail,
            action: Action::Channel(channel.clone()),
        }
    };
    channels.iter().enumerate().map(row).collect()
}

pub(super) fn signal_row(i: usize, candidate: Candidate, selection: &Selection) -> Row {
    let title = candidate.label(i);
    let detail = candidate.fields.get("type").cloned().unwrap_or_default();
    let chosen = Selection { candidate: Some(candidate), ..selection.clone() };
    Row { key: format!("s-{i}"), title, time: None, detail, action: Action::Signal(chosen) }
}
