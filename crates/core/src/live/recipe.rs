//! A provisional resolution recipe, from the `resolve = …` lines of a
//! `.stream` file or the `steps` of a recipe in the channel list: the steps
//! that turn a provider's API into the URL to play, run whenever the source
//! is opened or its token expires. Four operations only, on a current
//! value; the executable profile contract (docs/configuration.md,
//! "Composable recipes") will replace this syntax.
//!
//! - `get [URL]`: downloads URL, or the current value, as JSON or text;
//! - `json PATH`: takes a field of the current JSON, `a.b.0` style;
//! - `query NAME=VALUE`: sets a query parameter of the current URL;
//! - `text TEMPLATE`: the text of TEMPLATE, where `{json:PATH}` is a field
//!   of the current JSON, inserted as it is.
//!
//! A step written with `{name}` takes the parameter of that name, given by
//! the channel, before it runs (`instantiate`).

use serde_json::Value;
use url::Url;

use crate::language::{Language, Localized, display_in_english};
use crate::net;
use crate::text;

/// Steps in one recipe: a bound against loops of mistakes, not a feature.
const MAX_STEPS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Get(Option<String>),
    Json(String),
    Query(String, String),
    Text(String),
}

impl Step {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (operation, argument) = text.trim().split_once(' ').map_or((text.trim(), ""), |(o, a)| (o, a.trim()));
        let invalid = || Error::InvalidStep(text.to_string());
        match operation {
            "get" => Ok(Self::Get((!argument.is_empty()).then(|| argument.to_string()))),
            "json" if !argument.is_empty() => Ok(Self::Json(argument.to_string())),
            "query" => {
                let (name, value) = argument.split_once('=').ok_or_else(invalid)?;
                Ok(Self::Query(name.trim().to_string(), value.trim().to_string()))
            }
            "text" if !argument.is_empty() => Ok(Self::Text(argument.to_string())),
            _ => Err(invalid()),
        }
    }
}

/// Why a recipe could not be written out or run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A step that is not one of the operations, as written.
    InvalidStep(String),
    /// A `{name}` the channel gives no parameter for.
    MissingParameter(String),
    /// More than MAX_STEPS.
    TooManySteps(usize),
    /// The step numbered from 1, as written, and what went wrong in it.
    Step { number: usize, step: String, problem: Problem },
    /// The result is not a URL, as the url crate says.
    NotUrl(String),
    /// The last step gave no text.
    NoUrlAtEnd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    NoUrl,
    /// A URL that does not parse, as the url crate says.
    Url(String),
    Network(net::Error),
    NeitherJsonNorText,
    MissingField(String),
    FieldNotText(String),
    /// `{json:` without its `}`.
    Unclosed,
    NotJson,
    UnexpectedValue,
}

impl Error {
    /// The download that failed, when that is what stopped the recipe.
    pub fn network(&self) -> Option<&net::Error> {
        match self {
            Self::Step { problem: Problem::Network(error), .. } => Some(error),
            _ => None,
        }
    }
}

impl Localized for Error {
    fn text(&self, language: Language) -> String {
        match self {
            Self::InvalidStep(step) => text!(language, "recipe-invalid-step", step = step),
            Self::MissingParameter(name) => text!(language, "recipe-missing-parameter", name = name),
            Self::TooManySteps(count) => text!(language, "recipe-too-many-steps", count = count, max = MAX_STEPS),
            Self::Step { number, step, problem } => {
                text!(language, "recipe-step", number = number, step = step, problem = problem.text(language))
            }
            Self::NotUrl(detail) => text!(language, "recipe-not-url", detail = detail),
            Self::NoUrlAtEnd => text!(language, "recipe-no-url-at-end"),
        }
    }
}

impl Localized for Problem {
    fn text(&self, language: Language) -> String {
        match self {
            Self::NoUrl => text!(language, "recipe-no-url"),
            Self::Url(detail) => detail.clone(),
            Self::Network(error) => error.text(language),
            Self::NeitherJsonNorText => text!(language, "recipe-neither-json-nor-text"),
            Self::MissingField(path) => text!(language, "recipe-missing-field", path = path),
            Self::FieldNotText(path) => text!(language, "recipe-field-not-text", path = path),
            Self::Unclosed => text!(language, "recipe-unclosed"),
            Self::NotJson => text!(language, "recipe-not-json"),
            Self::UnexpectedValue => text!(language, "recipe-unexpected-value"),
        }
    }
}

display_in_english!(Error, Problem);

/// The steps written as `lines`, each `{name}` replaced by `parameter(name)`
/// first: a parameter it does not give, or a step that is not one of the
/// operations above, is an error. `{json:…}` is left to `text`, and
/// braces around anything but a name are text.
pub fn instantiate(lines: &[String], parameter: impl Fn(&str) -> Option<String>) -> Result<Vec<Step>, Error> {
    lines.iter().map(|line| Step::parse(&substitute(line, &parameter)?)).collect()
}

/// `line` with its `{name}` placeholders replaced.
fn substitute(line: &str, parameter: &impl Fn(&str) -> Option<String>) -> Result<String, Error> {
    let mut text = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|c| open + c) else { break };
        let name = &rest[open + 1..close];
        text.push_str(&rest[..open]);
        if is_name(name) {
            text.push_str(&parameter(name).ok_or_else(|| Error::MissingParameter(name.to_string()))?);
        } else {
            text.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    Ok(text)
}

/// Whether `text` names a parameter: letters, digits, `_` and `-`.
fn is_name(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

enum Current {
    Text(String),
    Json(Value),
}

/// Runs `steps` with `fetch` for the downloads; returns the URL they end on.
pub fn run(steps: &[Step], mut fetch: impl FnMut(&Url) -> Result<Vec<u8>, net::Error>) -> Result<Url, Error> {
    if steps.len() > MAX_STEPS {
        return Err(Error::TooManySteps(steps.len()));
    }
    let mut current = None;
    for (number, step) in steps.iter().enumerate() {
        let applied = apply(step, current.take(), &mut fetch);
        current = Some(applied.map_err(|problem| Error::Step { number: number + 1, step: format!("{step:?}"), problem })?);
    }
    match current {
        Some(Current::Text(url)) => Url::parse(&url).map_err(|e| Error::NotUrl(e.to_string())),
        _ => Err(Error::NoUrlAtEnd),
    }
}

/// The value `step` makes of the `current` one.
fn apply(
    step: &Step,
    current: Option<Current>,
    fetch: &mut impl FnMut(&Url) -> Result<Vec<u8>, net::Error>,
) -> Result<Current, Problem> {
    match (step, current) {
        (Step::Get(url), current) => get(url.as_deref(), current, fetch),
        (Step::Json(path), Some(Current::Json(json))) => Ok(match field(&json, path)? {
            Value::String(text) => Current::Text(text.clone()),
            other => Current::Json(other.clone()),
        }),
        (Step::Text(template), current) => {
            let json = match &current {
                Some(Current::Json(json)) => Some(json),
                _ => None,
            };
            Ok(Current::Text(fill(template, json)?))
        }
        (Step::Query(name, value), Some(Current::Text(url))) => with_query(&url, name, value).map(Current::Text),
        (_, _) => Err(Problem::UnexpectedValue),
    }
}

/// What the URL written in the step, else the current text, answers: JSON
/// when it reads as JSON, else text.
fn get(
    url: Option<&str>,
    current: Option<Current>,
    fetch: &mut impl FnMut(&Url) -> Result<Vec<u8>, net::Error>,
) -> Result<Current, Problem> {
    let url = match (url, current) {
        (Some(url), _) => url.to_string(),
        (None, Some(Current::Text(url))) => url,
        _ => return Err(Problem::NoUrl),
    };
    let url = Url::parse(&url).map_err(|e| Problem::Url(e.to_string()))?;
    let body = fetch(&url).map_err(Problem::Network)?;
    match serde_json::from_slice(&body) {
        Ok(json) => Ok(Current::Json(json)),
        Err(_) => String::from_utf8(body).map(Current::Text).map_err(|_| Problem::NeitherJsonNorText),
    }
}

/// `url` with its query parameter `name` set to `value`, the others kept.
fn with_query(url: &str, name: &str, value: &str) -> Result<String, Problem> {
    let mut url = Url::parse(url).map_err(|e| Problem::Url(e.to_string()))?;
    let others: Vec<(String, String)> =
        url.query_pairs().filter(|(key, _)| key != name).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    url.query_pairs_mut().clear().extend_pairs(others).append_pair(name, value);
    Ok(url.into())
}

/// The field at `path`, `a.b.0` style, of `json`.
fn field<'a>(json: &'a Value, path: &str) -> Result<&'a Value, Problem> {
    path.split('.')
        .try_fold(json, |value, key| match key.parse::<usize>() {
            Ok(index) => value.get(index),
            Err(_) => value.get(key),
        })
        .ok_or_else(|| Problem::MissingField(path.to_string()))
}

/// `template` with each `{json:PATH}` replaced by that field of `json`, as
/// it is: a text without its quotes, a number or a boolean as written.
fn fill(template: &str, json: Option<&Value>) -> Result<String, Problem> {
    let mut text = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find("{json:") {
        let close = rest[open..].find('}').map(|c| open + c).ok_or(Problem::Unclosed)?;
        let path = &rest[open + "{json:".len()..close];
        let json = json.ok_or(Problem::NotJson)?;
        text.push_str(&rest[..open]);
        match field(json, path)? {
            Value::String(value) => text.push_str(value),
            value @ (Value::Number(_) | Value::Bool(_)) => text.push_str(&value.to_string()),
            _ => return Err(Problem::FieldNotText(path.to_string())),
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    Ok(text)
}

#[cfg(test)]
mod tests;
