//! Mana's channel list, in TOML: sources defined once (`[sources.NAME]`,
//! docs/configuration.md), channels with a URL or a source and the fields
//! its flows read (`[[channels]]`), and
//! whole M3U lists included where they appear (`[[include]]`). A list
//! that is not TOML is refused as a whole, and so is one with the Italian
//! keys of the first versions; a broken channel stays in it, unavailable
//! with the reason, and fields this version does not know are ignored, so
//! that the list can grow without a new app.

use std::collections::BTreeMap;
use std::sync::Arc;

use toml::Spanned;
use toml::de::{DeTable, DeValue};
use url::Url;

use super::{Channel, Play};
use crate::language::{Language, Localized, display_in_english};
use crate::live::playable::Playable;
use crate::source::{self, Item, Selection, Source};
use crate::text;

/// The keys of the format, which its error messages name too.
mod key {
    pub const SOURCES: &str = "sources";
    pub const CHANNELS: &str = "channels";
    pub const INCLUDE: &str = "include";
    pub const M3U: &str = "m3u";
    pub const NAME: &str = "name";
    pub const LOGO: &str = "logo";
    pub const GROUP: &str = "group";
    pub const URL: &str = "url";
    pub const SOURCE: &str = "source";
    pub const HEADER: &str = "header";
    /// The root key of the lists written for the first versions, with
    /// Italian keys.
    pub const ITALIAN_CHANNELS: &str = "canali";
}
/// A channel's fields that are not fields of the item its source reads.
const CHANNEL_FIELDS: [&str; 6] = [key::NAME, key::LOGO, key::GROUP, key::URL, key::SOURCE, key::HEADER];
/// The item's field that holds the channel's name.
const TITLE: &str = "title";

/// Why a list was refused as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not TOML: the line, when the parser gives one, and what it says.
    NotToml { line: Option<usize>, message: String },
    /// The Italian keys of the first versions, `canali` at the root.
    ItalianKeys,
}

impl Localized for Error {
    fn text(&self, language: Language) -> String {
        match self {
            Self::NotToml { line: Some(line), message } => text!(language, "list-not-toml-at-line", line = line, message = message),
            Self::NotToml { line: None, message } => text!(language, "list-not-toml", message = message),
            Self::ItalianKeys => {
                text!(language, "list-italian-keys", channels = key::CHANNELS, italian = key::ITALIAN_CHANNELS)
            }
        }
    }
}

/// Why a channel cannot be played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelError {
    /// The list gives no stream for it, as an M3U entry without a URL or a
    /// made-up channel.
    NoStream,
    NoName,
    /// A field that is not a text.
    NotText(String),
    /// A `url` that does not parse, as the url crate says.
    BadUrl(String),
    BothUrlAndSource,
    NeitherUrlNorSource,
    /// A source the list does not define.
    UnknownSource(String),
    /// A source that does not keep the contract, as its checks say.
    BrokenSource { name: String, error: source::Error },
    /// Headers on a channel with a source: they are the source's to give.
    HeaderWithSource,
    /// A header that is not a text.
    HeaderNotText(String),
    HeaderNotTable,
}

impl Localized for ChannelError {
    fn text(&self, language: Language) -> String {
        match self {
            Self::NoStream => text!(language, "channel-no-stream"),
            Self::NoName => text!(language, "channel-no-name", key = key::NAME),
            Self::NotText(key) => text!(language, "channel-not-text", key = key),
            Self::BadUrl(detail) => text!(language, "channel-bad-url", key = key::URL, detail = detail),
            Self::BothUrlAndSource => text!(language, "channel-url-and-source", url = key::URL, source = key::SOURCE),
            Self::NeitherUrlNorSource => text!(language, "channel-no-url-or-source", url = key::URL, source = key::SOURCE),
            Self::UnknownSource(name) => text!(language, "channel-unknown-source", name = name),
            Self::BrokenSource { name, error } => text!(language, "channel-source", name = name, problem = error.text(language)),
            Self::HeaderWithSource => text!(language, "channel-header-with-source", key = key::HEADER),
            Self::HeaderNotText(name) => text!(language, "channel-header-not-text", name = name),
            Self::HeaderNotTable => text!(language, "channel-header-not-table", key = key::HEADER),
        }
    }
}

display_in_english!(Error, ChannelError);

/// The list as written, in the file's order.
#[derive(Debug, PartialEq)]
pub struct List {
    pub entries: Vec<Entry>,
    /// Why an inclusion, or a section, was left out, for the log.
    pub skipped: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum Entry {
    Channel(Channel),
    /// The URL of an M3U list whose channels go here.
    Include(Url),
    /// A source whose catalogue's items go here, as channels: its name in
    /// the list, and the source with the device.
    Catalogue(String, Selection),
}

/// The sources of the list by name, checked, or why they cannot be used.
type Sources = BTreeMap<String, Result<Arc<Source>, source::Error>>;

/// The list in `text`; `device` gives the device identifier, asked for
/// only when a channel has a source. An error when `text` is not TOML, or
/// has the Italian keys.
pub fn parse(text: &str, device: impl Fn() -> Option<String>) -> Result<List, Error> {
    let root = DeTable::parse(text).map_err(|e| not_toml(text, &e))?.into_inner();
    if root.contains_key(key::ITALIAN_CHANNELS) {
        return Err(Error::ItalianKeys);
    }
    let mut skipped = Vec::new();
    let sources = sources(text, &mut skipped);
    let mut placed = Vec::new();
    for (section, value) in tables(&root, key::CHANNELS, &mut skipped) {
        placed.push((section, Entry::Channel(channel(value, &sources, &device))));
    }
    for (section, table) in tables(&root, key::INCLUDE, &mut skipped) {
        match included(table, &sources, &device) {
            Ok(entry) => placed.push((section, entry)),
            Err(reason) => skipped.push(format!("inclusion: {reason}")),
        }
    }
    // Channels and inclusions are two arrays: their order is that of their
    // headers in the file.
    placed.sort_by_key(|(at, _)| *at);
    Ok(List { entries: placed.into_iter().map(|(_, entry)| entry).collect(), skipped })
}

/// Why `text` is not TOML: where, and what the parser says.
fn not_toml(text: &str, error: &toml::de::Error) -> Error {
    let message = error.message().trim().to_string();
    let line = error.span().map(|span| text.as_bytes()[..span.start.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1);
    Error::NotToml { line, message }
}

/// The tables of the array `key`, each with where it starts in the file;
/// what is not a table is skipped with the reason.
fn tables<'a, 'i>(root: &'a DeTable<'i>, key: &str, skipped: &mut Vec<String>) -> Vec<(usize, &'a DeTable<'i>)> {
    let Some(value) = root.get(key) else { return Vec::new() };
    let DeValue::Array(array) = value.get_ref() else {
        skipped.push(format!("`{key}` is not a list of tables"));
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|item| match item.get_ref() {
            DeValue::Table(table) => Some((item.span().start, table)),
            _ => {
                skipped.push(format!("an entry of `{key}` is not a table"));
                None
            }
        })
        .collect()
}

/// What an `[[include]]` puts in its place: an M3U list, or the catalogue
/// of a source of the list; why it puts nothing, for the log.
fn included(table: &DeTable, sources: &Sources, device: &impl Fn() -> Option<String>) -> Result<Entry, String> {
    let (m3u, source) = (string(table, key::M3U).map_err(|e| e.to_string())?, string(table, key::SOURCE).map_err(|e| e.to_string())?);
    match (m3u, source) {
        (Some(m3u), None) => Url::parse(&m3u).map(Entry::Include).map_err(|e| format!("{m3u}: {e}")),
        (None, Some(name)) => match sources.get(&name) {
            Some(Ok(source)) if source.catalogue.is_some() => {
                let selection = Selection {
                    source: source.clone(),
                    item: Item { fields: BTreeMap::new() },
                    device: device().unwrap_or_default(),
                    candidate: None,
                };
                Ok(Entry::Catalogue(name, selection))
            }
            Some(Ok(_)) => Err(format!("source {name} has no catalogue")),
            Some(Err(error)) => Err(format!("source {name}: {error}")),
            None => Err(format!("source {name} does not exist")),
        },
        _ => Err(format!("one of `{}` and `{}` is needed", key::M3U, key::SOURCE)),
    }
}

/// The list's `[sources.NAME]`, each read in the data model a source file
/// has and checked. A broken source makes its channels unavailable, with
/// the reason; `sources` that is not a table of tables is skipped.
fn sources(text: &str, skipped: &mut Vec<String>) -> Sources {
    let Ok(root) = toml::from_str::<serde_json::Value>(text) else { return Sources::new() };
    let Some(sources) = root.get(key::SOURCES) else { return Sources::new() };
    let Some(sources) = sources.as_object() else {
        skipped.push(format!("`{}` is not a table", key::SOURCES));
        return Sources::new();
    };
    sources.iter().map(|(name, value)| (name.clone(), Source::from_list(name, value).map(Arc::new))).collect()
}

/// The channel of `table`, unavailable with the reason when it is broken.
fn channel(table: &DeTable, sources: &Sources, device: &impl Fn() -> Option<String>) -> Channel {
    let name = string(table, key::NAME);
    let play = match &name {
        Ok(Some(name)) => play(table, name, sources, device),
        Ok(None) => Err(ChannelError::NoName),
        Err(error) => Err(error.clone()),
    };
    Channel {
        name: name.ok().flatten(),
        logo: string(table, key::LOGO).ok().flatten(),
        group: string(table, key::GROUP).ok().flatten(),
        starts_at: None,
        play,
    }
}

/// What the channel of `table`, named `name`, plays: its URL with its
/// headers, or the item of its source, made of its name as `title` and its
/// other fields.
fn play(table: &DeTable, name: &str, sources: &Sources, device: &impl Fn() -> Option<String>) -> Result<Play, ChannelError> {
    match (string(table, key::URL)?, string(table, key::SOURCE)?) {
        (Some(url), None) => {
            let headers = headers(table)?;
            let url = Url::parse(&url).map_err(|e| ChannelError::BadUrl(e.to_string()))?;
            Ok(Play::Stream(Playable::direct(url, headers)))
        }
        (None, Some(source)) => {
            if table.contains_key(key::HEADER) {
                return Err(ChannelError::HeaderWithSource);
            }
            let found = sources.get(&source).ok_or_else(|| ChannelError::UnknownSource(source.clone()))?;
            let found = found.as_ref().map_err(|error| ChannelError::BrokenSource { name: source, error: error.clone() })?;
            let mut fields: BTreeMap<String, String> =
                table.keys().filter_map(|key| Some((key.get_ref().to_string(), field(table, key.get_ref())?))).collect();
            fields.insert(TITLE.into(), name.into());
            let item = Item { fields };
            Ok(Play::Source(Selection { source: found.clone(), item, device: device().unwrap_or_default(), candidate: None }))
        }
        (Some(_), Some(_)) => Err(ChannelError::BothUrlAndSource),
        (None, None) => Err(ChannelError::NeitherUrlNorSource),
    }
}

/// The text of the field `key`, if there is one; an error when it is not
/// a text.
fn string(table: &DeTable, key: &str) -> Result<Option<String>, ChannelError> {
    match table.get(key).map(Spanned::get_ref) {
        None => Ok(None),
        Some(DeValue::String(text)) => Ok(Some(text.to_string())),
        Some(_) => Err(ChannelError::NotText(key.to_string())),
    }
}

/// The channel's `header` table.
fn headers(table: &DeTable) -> Result<Vec<(String, String)>, ChannelError> {
    match table.get(key::HEADER).map(Spanned::get_ref) {
        None => Ok(Vec::new()),
        Some(DeValue::Table(headers)) => headers
            .iter()
            .map(|(name, value)| match value.get_ref().as_str() {
                Some(value) => Ok((name.get_ref().to_string(), value.to_string())),
                None => Err(ChannelError::HeaderNotText(name.get_ref().to_string())),
            })
            .collect(),
        Some(_) => Err(ChannelError::HeaderNotTable),
    }
}

/// The item field `key` of the channel: a field of its own that is a
/// text, a number or a boolean, as written.
fn field(table: &DeTable, key: &str) -> Option<String> {
    if CHANNEL_FIELDS.contains(&key) {
        return None;
    }
    match table.get(key)?.get_ref() {
        DeValue::String(text) => Some(text.to_string()),
        DeValue::Integer(number) => Some(number.as_str().to_string()),
        DeValue::Float(number) => Some(number.as_str().to_string()),
        DeValue::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
