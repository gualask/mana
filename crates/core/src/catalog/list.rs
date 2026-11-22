//! Mana's channel list, in TOML: recipes defined once (`[ricette.NAME]`),
//! channels with a URL or a recipe and its parameters (`[[canali]]`), and
//! whole M3U lists included where they appear (`[[includi]]`). A list
//! that is not TOML is refused as a whole; a broken channel stays in it,
//! unavailable with the reason, and fields this version does not know are
//! ignored, so that the list can grow without a new app.

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::{DeTable, DeValue};
use url::Url;

use super::Channel;
use crate::live::recipe;
use crate::live::source::Source;

/// The parameter that gives the installation's device identifier.
pub const DEVICE_PARAMETER: &str = "dispositivo";
/// A channel's fields that are not parameters of its recipe.
const CHANNEL_FIELDS: [&str; 6] = ["nome", "logo", "gruppo", "url", "ricetta", "header"];
/// The name shown for a channel without one.
const UNNAMED: &str = "Canale senza nome";

/// The list as written, in the file's order.
#[derive(Debug, PartialEq)]
pub struct List {
    pub entries: Vec<Entry>,
    /// Why an inclusion, or a section, was left out.
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
type Recipes = BTreeMap<String, Result<Vec<String>, String>>;

/// The list in `text`; `device` gives the device identifier, asked for
/// only when a recipe uses it. An error when `text` is not TOML.
pub fn parse(text: &str, device: impl Fn() -> Option<String>) -> Result<List, String> {
    let root = DeTable::parse(text).map_err(|e| not_toml(text, &e))?.into_inner();
    let recipes = match root.get("ricette").map(Spanned::get_ref) {
        Some(DeValue::Table(table)) => table.iter().map(|(name, value)| (name.get_ref().to_string(), steps(value))).collect(),
        _ => Recipes::new(),
    };
    let mut skipped = Vec::new();
    let mut placed = Vec::new();
    for (section, value) in tables(&root, "canali", &mut skipped) {
        placed.push((section, Entry::Channel(channel(value, &recipes, &device))));
    }
    for (section, table) in tables(&root, "includi", &mut skipped) {
        match string(table, "m3u").map(|m3u| m3u.map(|m3u| Url::parse(&m3u).map_err(|e| format!("{m3u}: {e}")))) {
            Ok(Some(Ok(url))) => placed.push((section, Entry::Include(url))),
            Ok(Some(Err(error))) | Err(error) => skipped.push(format!("inclusione: {error}")),
            Ok(None) => skipped.push("inclusione senza `m3u`".into()),
        }
    }
    // Channels and inclusions are two arrays: their order is that of their
    // headers in the file.
    placed.sort_by_key(|(at, _)| *at);
    Ok(List { entries: placed.into_iter().map(|(_, entry)| entry).collect(), skipped })
}

/// Why `text` is not TOML, in one line: where, and what the parser says.
fn not_toml(text: &str, error: &toml::de::Error) -> String {
    let message = error.message().trim();
    let Some(span) = error.span() else { return message.to_string() };
    let line = text.as_bytes()[..span.start.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1;
    format!("riga {line}: {message}")
}

/// The tables of the array `key`, each with where it starts in the file;
/// what is not a table is skipped with the reason.
fn tables<'a, 'i>(root: &'a DeTable<'i>, key: &str, skipped: &mut Vec<String>) -> Vec<(usize, &'a DeTable<'i>)> {
    let Some(value) = root.get(key) else { return Vec::new() };
    let DeValue::Array(array) = value.get_ref() else {
        skipped.push(format!("`{key}` non è un elenco di tabelle"));
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|item| match item.get_ref() {
            DeValue::Table(table) => Some((item.span().start, table)),
            _ => {
                skipped.push(format!("voce di `{key}` che non è una tabella"));
                None
            }
        })
        .collect()
}

/// The steps of a recipe as written, from its `passi`.
fn steps(value: &Spanned<DeValue>) -> Result<Vec<String>, String> {
    let DeValue::Table(table) = value.get_ref() else { return Err("non è una tabella".into()) };
    let steps = match table.get("passi").map(Spanned::get_ref) {
        Some(DeValue::Array(steps)) => steps,
        Some(_) => return Err("`passi` non è un elenco".into()),
        None => return Err("senza `passi`".into()),
    };
    steps.iter().map(|step| step.get_ref().as_str().map(str::to_string).ok_or("un passo non è un testo".into())).collect()
}

/// The channel of `table`, unavailable with the reason when it is broken.
fn channel(table: &DeTable, recipes: &Recipes, device: &impl Fn() -> Option<String>) -> Channel {
    let name = string(table, "nome");
    let play = match &name {
        Ok(Some(_)) => source(table, recipes, device),
        Ok(None) => Err("canale senza `nome`".into()),
        Err(error) => Err(error.clone()),
    };
    Channel {
        name: name.ok().flatten().unwrap_or_else(|| UNNAMED.into()),
        logo: string(table, "logo").ok().flatten(),
        group: string(table, "gruppo").ok().flatten(),
        play,
    }
}

/// What the channel of `table` plays: its URL, or its recipe with its
/// parameters, with its headers.
fn source(table: &DeTable, recipes: &Recipes, device: &impl Fn() -> Option<String>) -> Result<Source, String> {
    let headers = headers(table)?;
    match (string(table, "url")?, string(table, "ricetta")?) {
        (Some(url), None) => {
            Url::parse(&url).map_err(|e| format!("`url` non valido: {e}"))?;
            Ok(Source::direct(url, headers))
        }
        (None, Some(name)) => {
            let lines = recipes.get(&name).ok_or_else(|| format!("ricetta {name} inesistente"))?;
            let lines = lines.as_ref().map_err(|e| format!("ricetta {name}: {e}"))?;
            let parameter =
                |key: &str| parameter(table, key).or_else(|| if key == DEVICE_PARAMETER { device() } else { None });
            let steps = recipe::instantiate(lines, parameter).map_err(|e| format!("ricetta {name}: {e}"))?;
            Ok(Source::recipe(steps, headers))
        }
        (Some(_), Some(_)) => Err("canale con sia `url` sia `ricetta`".into()),
        (None, None) => Err("canale senza `url` né `ricetta`".into()),
    }
}

/// The text of the field `key`, if there is one; an error when it is not
/// a text.
fn string(table: &DeTable, key: &str) -> Result<Option<String>, String> {
    match table.get(key).map(Spanned::get_ref) {
        None => Ok(None),
        Some(DeValue::String(text)) => Ok(Some(text.to_string())),
        Some(_) => Err(format!("`{key}` non è un testo")),
    }
}

/// The channel's `header` table.
fn headers(table: &DeTable) -> Result<Vec<(String, String)>, String> {
    match table.get("header").map(Spanned::get_ref) {
        None => Ok(Vec::new()),
        Some(DeValue::Table(headers)) => headers
            .iter()
            .map(|(name, value)| match value.get_ref().as_str() {
                Some(value) => Ok((name.get_ref().to_string(), value.to_string())),
                None => Err(format!("header {} non è un testo", name.get_ref())),
            })
            .collect(),
        Some(_) => Err("`header` non è una tabella".into()),
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
