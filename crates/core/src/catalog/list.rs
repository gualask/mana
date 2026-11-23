//! Mana's channel list, in TOML: recipes defined once (`[recipes.NAME]`),
//! channels with a URL or a recipe and its parameters (`[[channels]]`), and
//! whole M3U lists included where they appear (`[[include]]`). A list
//! that is not TOML is refused as a whole, and so is one with the Italian
//! keys of the first versions; a broken channel stays in it, unavailable
//! with the reason, and fields this version does not know are ignored, so
//! that the list can grow without a new app.

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::{DeTable, DeValue};
use url::Url;

use super::Channel;
use crate::language::{Language, Localized, display_in_english};
use crate::live::recipe;
use crate::live::source::Source;
use crate::text;

/// The parameter that gives the installation's device identifier.
pub const DEVICE_PARAMETER: &str = "device";
/// The keys of the format, which its error messages name too.
mod key {
    pub const RECIPES: &str = "recipes";
    pub const STEPS: &str = "steps";
    pub const CHANNELS: &str = "channels";
    pub const INCLUDE: &str = "include";
    pub const M3U: &str = "m3u";
    pub const NAME: &str = "name";
    pub const LOGO: &str = "logo";
    pub const GROUP: &str = "group";
    pub const URL: &str = "url";
    pub const RECIPE: &str = "recipe";
    pub const HEADER: &str = "header";
    /// The root key of the lists written for the first versions, with
    /// Italian keys.
    pub const ITALIAN_CHANNELS: &str = "canali";
}
/// A channel's fields that are not parameters of its recipe.
const CHANNEL_FIELDS: [&str; 6] = [key::NAME, key::LOGO, key::GROUP, key::URL, key::RECIPE, key::HEADER];

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
    BothUrlAndRecipe,
    NeitherUrlNorRecipe,
    /// A recipe the list does not define.
    UnknownRecipe(String),
    /// A recipe defined in a way that cannot be used.
    BrokenRecipe { name: String, problem: RecipeProblem },
    /// A recipe whose steps do not fit the channel.
    Recipe { name: String, error: recipe::Error },
    /// A header that is not a text.
    HeaderNotText(String),
    HeaderNotTable,
}

/// What is wrong with a recipe as the list defines it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeProblem {
    NotTable,
    StepsNotList,
    NoSteps,
    StepNotText,
}

impl Localized for ChannelError {
    fn text(&self, language: Language) -> String {
        match self {
            Self::NoStream => text!(language, "channel-no-stream"),
            Self::NoName => text!(language, "channel-no-name", key = key::NAME),
            Self::NotText(key) => text!(language, "channel-not-text", key = key),
            Self::BadUrl(detail) => text!(language, "channel-bad-url", key = key::URL, detail = detail),
            Self::BothUrlAndRecipe => text!(language, "channel-url-and-recipe", url = key::URL, recipe = key::RECIPE),
            Self::NeitherUrlNorRecipe => text!(language, "channel-no-url-or-recipe", url = key::URL, recipe = key::RECIPE),
            Self::UnknownRecipe(name) => text!(language, "channel-unknown-recipe", name = name),
            Self::BrokenRecipe { name, problem } => text!(language, "channel-recipe", name = name, problem = problem.text(language)),
            Self::Recipe { name, error } => text!(language, "channel-recipe", name = name, problem = error.text(language)),
            Self::HeaderNotText(name) => text!(language, "channel-header-not-text", name = name),
            Self::HeaderNotTable => text!(language, "channel-header-not-table", key = key::HEADER),
        }
    }
}

impl Localized for RecipeProblem {
    fn text(&self, language: Language) -> String {
        match self {
            Self::NotTable => text!(language, "channel-recipe-not-table"),
            Self::StepsNotList => text!(language, "channel-recipe-steps-not-list", key = key::STEPS),
            Self::NoSteps => text!(language, "channel-recipe-no-steps", key = key::STEPS),
            Self::StepNotText => text!(language, "channel-recipe-step-not-text"),
        }
    }
}

display_in_english!(Error, ChannelError, RecipeProblem);

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
}

/// The recipes of the list by name: their steps as written, or why they
/// cannot be used.
type Recipes = BTreeMap<String, Result<Vec<String>, RecipeProblem>>;

/// The list in `text`; `device` gives the device identifier, asked for
/// only when a recipe uses it. An error when `text` is not TOML, or has
/// the Italian keys.
pub fn parse(text: &str, device: impl Fn() -> Option<String>) -> Result<List, Error> {
    let root = DeTable::parse(text).map_err(|e| not_toml(text, &e))?.into_inner();
    if root.contains_key(key::ITALIAN_CHANNELS) {
        return Err(Error::ItalianKeys);
    }
    let recipes = match root.get(key::RECIPES).map(Spanned::get_ref) {
        Some(DeValue::Table(table)) => table.iter().map(|(name, value)| (name.get_ref().to_string(), steps(value))).collect(),
        _ => Recipes::new(),
    };
    let mut skipped = Vec::new();
    let mut placed = Vec::new();
    for (section, value) in tables(&root, key::CHANNELS, &mut skipped) {
        placed.push((section, Entry::Channel(channel(value, &recipes, &device))));
    }
    for (section, table) in tables(&root, key::INCLUDE, &mut skipped) {
        match string(table, key::M3U).map(|m3u| m3u.map(|m3u| Url::parse(&m3u).map_err(|e| format!("{m3u}: {e}")))) {
            Ok(Some(Ok(url))) => placed.push((section, Entry::Include(url))),
            Ok(Some(Err(error))) => skipped.push(format!("inclusion: {error}")),
            Err(error) => skipped.push(format!("inclusion: {error}")),
            Ok(None) => skipped.push(format!("inclusion without `{}`", key::M3U)),
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

/// The steps of a recipe as written, from its `steps`.
fn steps(value: &Spanned<DeValue>) -> Result<Vec<String>, RecipeProblem> {
    let DeValue::Table(table) = value.get_ref() else { return Err(RecipeProblem::NotTable) };
    let steps = match table.get(key::STEPS).map(Spanned::get_ref) {
        Some(DeValue::Array(steps)) => steps,
        Some(_) => return Err(RecipeProblem::StepsNotList),
        None => return Err(RecipeProblem::NoSteps),
    };
    steps.iter().map(|step| step.get_ref().as_str().map(str::to_string).ok_or(RecipeProblem::StepNotText)).collect()
}

/// The channel of `table`, unavailable with the reason when it is broken.
fn channel(table: &DeTable, recipes: &Recipes, device: &impl Fn() -> Option<String>) -> Channel {
    let name = string(table, key::NAME);
    let play = match &name {
        Ok(Some(_)) => source(table, recipes, device),
        Ok(None) => Err(ChannelError::NoName),
        Err(error) => Err(error.clone()),
    };
    Channel {
        name: name.ok().flatten(),
        logo: string(table, key::LOGO).ok().flatten(),
        group: string(table, key::GROUP).ok().flatten(),
        play,
    }
}

/// What the channel of `table` plays: its URL, or its recipe with its
/// parameters, with its headers.
fn source(table: &DeTable, recipes: &Recipes, device: &impl Fn() -> Option<String>) -> Result<Source, ChannelError> {
    let headers = headers(table)?;
    match (string(table, key::URL)?, string(table, key::RECIPE)?) {
        (Some(url), None) => {
            let url = Url::parse(&url).map_err(|e| ChannelError::BadUrl(e.to_string()))?;
            Ok(Source::direct(url, headers))
        }
        (None, Some(name)) => {
            let lines = recipes.get(&name).ok_or_else(|| ChannelError::UnknownRecipe(name.clone()))?;
            let lines = lines.as_ref().map_err(|problem| ChannelError::BrokenRecipe { name: name.clone(), problem: problem.clone() })?;
            let parameter =
                |key: &str| parameter(table, key).or_else(|| if key == DEVICE_PARAMETER { device() } else { None });
            let steps = recipe::instantiate(lines, parameter).map_err(|error| ChannelError::Recipe { name, error })?;
            Ok(Source::recipe(steps, headers))
        }
        (Some(_), Some(_)) => Err(ChannelError::BothUrlAndRecipe),
        (None, None) => Err(ChannelError::NeitherUrlNorRecipe),
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

/// The recipe parameter `key` of the channel: a field of its own that is
/// a text, a number or a boolean, as written.
fn parameter(table: &DeTable, key: &str) -> Option<String> {
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
