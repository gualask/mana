//! A source: how a provider's channels or events are listed (`catalogue`),
//! what streams an item offers (`sources`) and how a stream is resolved to
//! the media to play (`resolve`), written as data with the steps of a fixed
//! vocabulary (operation.rs). The contract is in docs/configuration.md.
//!
//! This module reads a source file and checks it before it can be used
//! (validate.rs); nothing runs its steps yet. A file is read into the data
//! model TOML and JSON share, so both give the same `Source`.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::language::{Language, Localized, display_in_english};
use crate::text;

mod operation;
mod validate;

pub use operation::{Alphabet, Field, Headers, Kind, Mapping, Method, Mode, Operation};

/// A source file, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// Named header sets, used by the steps and the streams by name.
    pub headers: BTreeMap<String, BTreeMap<String, String>>,
    pub catalogue: Option<Catalogue>,
    /// From an item to the candidates it offers; without it, the item is
    /// its only candidate.
    pub sources: Option<Flow>,
    pub resolve: Resolve,
    /// Resolvers chosen by the routes of `resolve`.
    pub resolvers: BTreeMap<String, Flow>,
    /// Steps shared by several flows, run with `call`.
    pub recipes: BTreeMap<String, Recipe>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Catalogue {
    /// Items written in the file.
    Items(Vec<Item>),
    Steps(Flow),
}

/// A channel or an event: `id` and `title`, then the fields the next flows
/// read, all text.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Resolve {
    Steps(Flow),
    /// Tried in order on the candidate; the first that matches chooses.
    Routes(Vec<Route>),
}

/// Ordered steps; the last one gives the flow's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub id: String,
    pub operation: Operation,
}

/// A flow run with `call`, given the inputs it names.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub inputs: Vec<String>,
    pub flow: Flow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub when: When,
    pub to: Use,
}

/// What a candidate must have for a route to choose it: its `type` field,
/// and the host and the path of its `url`, compared exactly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct When {
    pub kind: Option<String>,
    pub host: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Use {
    /// The candidate's `url` is already the media.
    Stream(Option<Headers>),
    Resolver(String),
}

impl Source {
    /// The source a TOML file describes, if it keeps the contract.
    pub fn from_toml(text: &str) -> Result<Self, Error> {
        let value: Value = toml::from_str(text).map_err(|error: toml::de::Error| {
            let line = error.span().map(|span| text.as_bytes()[..span.start.min(text.len())].iter().filter(|&&b| b == b'\n').count() + 1);
            Error::Format { line, message: error.message().trim().to_string() }
        })?;
        let source = read(&value)?;
        validate::check(&source)?;
        Ok(source)
    }
}

fn read(value: &Value) -> Result<Source, Error> {
    let top = ["id", "name", "headers", "catalogue", "sources", "resolve", "resolvers", "recipes"];
    let fields = Fields::new("source", value, &top)?;
    let id = fields.required_text("id")?;
    if !is_name(&id) {
        return Err(fields.invalid("id", "letters, digits, `-` and `_` only"));
    }
    let mut headers = BTreeMap::new();
    for (name, set) in fields.table("headers")?.into_iter().flatten() {
        let at = format!("headers.{name}");
        let set = set.as_object().ok_or_else(|| Error::invalid(&at, name, "not a table"))?;
        let values = set.iter().map(|(key, value)| match value {
            Value::String(text) => Ok((key.clone(), text.clone())),
            _ => Err(Error::invalid(&at, key, "not a text")),
        });
        headers.insert(name.clone(), values.collect::<Result<_, _>>()?);
    }
    let catalogue = fields.get("catalogue").map(|value| catalogue(value)).transpose()?;
    let sources = fields.get("sources").map(|value| flow("sources", value, &[])).transpose()?;
    let resolve = match fields.get("resolve") {
        Some(value) => resolve(value)?,
        None => return Err(Error::MissingField { at: "source".into(), field: "resolve".into() }),
    };
    let mut resolvers = BTreeMap::new();
    for (name, value) in fields.table("resolvers")?.into_iter().flatten() {
        resolvers.insert(name.clone(), flow(&format!("resolvers.{name}"), value, &[])?);
    }
    let mut recipes = BTreeMap::new();
    for (name, value) in fields.table("recipes")?.into_iter().flatten() {
        let at = format!("recipes.{name}");
        let inputs = Fields::new(&at, value, &["inputs", "steps"])?.texts("inputs")?;
        recipes.insert(name.clone(), Recipe { inputs, flow: flow(&at, value, &["inputs"])? });
    }
    let source = Source { id, name: fields.required_text("name")?, headers, catalogue, sources, resolve, resolvers, recipes };
    Ok(source)
}

fn catalogue(value: &Value) -> Result<Catalogue, Error> {
    let fields = Fields::new("catalogue", value, &["items", "steps"])?;
    match (fields.get("items"), fields.get("steps")) {
        (Some(items), None) => {
            let items = items.as_array().ok_or_else(|| fields.invalid("items", "not a list"))?;
            items.iter().enumerate().map(|(n, value)| item(&format!("catalogue.items.{n}"), value)).collect::<Result<_, _>>().map(Catalogue::Items)
        }
        (None, Some(_)) => flow("catalogue", value, &["items"]).map(Catalogue::Steps),
        _ => Err(Error::OneOf { at: "catalogue".into(), choices: "`items`, `steps`".into() }),
    }
}

fn item(at: &str, value: &Value) -> Result<Item, Error> {
    let table = value.as_object().ok_or_else(|| Error::invalid(at, "", "not a table"))?;
    let mut fields = BTreeMap::new();
    for (key, value) in table {
        let Value::String(text) = value else { return Err(Error::invalid(at, key, "not a text")) };
        fields.insert(key.clone(), text.clone());
    }
    for required in ["id", "title"] {
        if !fields.contains_key(required) {
            return Err(Error::MissingField { at: at.into(), field: required.into() });
        }
    }
    Ok(Item { fields })
}

fn resolve(value: &Value) -> Result<Resolve, Error> {
    let fields = Fields::new("resolve", value, &["steps", "routes"])?;
    match (fields.get("steps"), fields.get("routes")) {
        (Some(_), None) => flow("resolve", value, &["routes"]).map(Resolve::Steps),
        (None, Some(routes)) => {
            let routes = routes.as_array().ok_or_else(|| fields.invalid("routes", "not a list"))?;
            routes.iter().enumerate().map(|(n, value)| route(&format!("resolve.routes.{n}"), value)).collect::<Result<_, _>>().map(Resolve::Routes)
        }
        _ => Err(Error::OneOf { at: "resolve".into(), choices: "`steps`, `routes`".into() }),
    }
}

fn route(at: &str, value: &Value) -> Result<Route, Error> {
    let fields = Fields::new(at, value, &["when", "use", "headers"])?;
    let when_at = format!("{at}.when");
    let when = match fields.get("when") {
        Some(value) => {
            let when = Fields::new(&when_at, value, &["type", "host", "path"])?;
            When { kind: when.text("type")?, host: when.text("host")?, path: when.text("path")? }
        }
        None => When::default(),
    };
    let to = match fields.required_text("use")?.as_str() {
        "stream" => Use::Stream(fields.headers("headers")?),
        name => {
            if fields.get("headers").is_some() {
                return Err(fields.invalid("headers", "only a route with `use = \"stream\"` gives headers"));
            }
            Use::Resolver(name.to_string())
        }
    };
    Ok(Route { when, to })
}

/// The `steps` of the flow at `at`; `others` are the other keys its table
/// may have.
fn flow(at: &str, value: &Value, others: &[&str]) -> Result<Flow, Error> {
    let fields = Fields::new(at, value, &[&["steps"], others].concat())?;
    let steps = fields.get("steps").ok_or_else(|| Error::MissingField { at: at.into(), field: "steps".into() })?;
    let steps = steps.as_array().ok_or_else(|| fields.invalid("steps", "not a list"))?;
    if steps.is_empty() {
        return Err(fields.invalid("steps", "no steps"));
    }
    let steps = steps.iter().enumerate().map(|(n, value)| {
        let id = value.get("id").and_then(Value::as_str);
        step(&id.map_or_else(|| format!("{at}.{n}"), |id| format!("{at}.{id}")), value)
    });
    Ok(Flow { steps: steps.collect::<Result<_, _>>()? })
}

fn step(at: &str, value: &Value) -> Result<Step, Error> {
    let table = value.as_object().ok_or_else(|| Error::invalid(at, "", "not a table"))?;
    let text = |key: &str| match table.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(Error::invalid(at, key, "not a text")),
        None => Err(Error::MissingField { at: at.into(), field: key.into() }),
    };
    let id = text("id")?;
    if !is_name(&id) || id == "input" || id == "item" {
        return Err(Error::invalid(at, "id", "letters, digits, `-` and `_` only, and neither `input` nor `item`"));
    }
    Ok(Step { operation: Operation::read(at, &text("op")?, value)?, id })
}

/// Whether `text` can name a step, a source or a field of a reference.
fn is_name(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The keys of a table, read with the place they are at for the errors;
/// a key it does not allow is an error.
struct Fields<'a> {
    at: String,
    map: &'a Map<String, Value>,
}

impl<'a> Fields<'a> {
    fn new(at: &str, value: &'a Value, allowed: &[&str]) -> Result<Self, Error> {
        let map = value.as_object().ok_or_else(|| Error::invalid(at, "", "not a table"))?;
        if let Some(key) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(Error::UnknownField { at: at.into(), field: key.clone() });
        }
        Ok(Self { at: at.into(), map })
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key)
    }

    fn invalid(&self, key: &str, detail: &str) -> Error {
        Error::invalid(&self.at, key, detail)
    }

    fn missing(&self, key: &str) -> Error {
        Error::MissingField { at: self.at.clone(), field: key.into() }
    }

    fn text(&self, key: &str) -> Result<Option<String>, Error> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(self.invalid(key, "not a text")),
        }
    }

    fn required_text(&self, key: &str) -> Result<String, Error> {
        self.text(key)?.ok_or_else(|| self.missing(key))
    }

    fn texts(&self, key: &str) -> Result<Vec<String>, Error> {
        let Some(value) = self.get(key) else { return Ok(Vec::new()) };
        let list = value.as_array().ok_or_else(|| self.invalid(key, "not a list"))?;
        list.iter().map(|value| value.as_str().map(str::to_string).ok_or_else(|| self.invalid(key, "not a list of texts"))).collect()
    }

    fn bool(&self, key: &str) -> Result<Option<bool>, Error> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(self.invalid(key, "not true or false")),
        }
    }

    fn table(&self, key: &str) -> Result<Option<&'a Map<String, Value>>, Error> {
        self.get(key).map(|value| value.as_object().ok_or_else(|| self.invalid(key, "not a table"))).transpose()
    }

    fn template(&self, key: &str) -> Result<Option<Template>, Error> {
        self.text(key)?.map(|text| Template::parse(&text).map_err(|detail| self.invalid(key, &detail))).transpose()
    }

    fn required_template(&self, key: &str) -> Result<Template, Error> {
        self.template(key)?.ok_or_else(|| self.missing(key))
    }

    /// A table of templates, empty when absent.
    fn templates(&self, key: &str) -> Result<BTreeMap<String, Template>, Error> {
        let mut templates = BTreeMap::new();
        for (name, value) in self.table(key)?.into_iter().flatten() {
            let Value::String(text) = value else { return Err(self.invalid(key, &format!("`{name}` is not a text"))) };
            templates.insert(name.clone(), Template::parse(text).map_err(|detail| self.invalid(key, &detail))?);
        }
        Ok(templates)
    }

    /// One of `choices`, by its name in the file.
    fn choice<T: Copy>(&self, key: &str, choices: &[(&str, T)]) -> Result<Option<T>, Error> {
        let Some(name) = self.text(key)? else { return Ok(None) };
        let names: Vec<_> = choices.iter().map(|(name, _)| format!("`{name}`")).collect();
        let found = choices.iter().find(|(choice, _)| *choice == name).map(|(_, value)| *value);
        found.map(Some).ok_or_else(|| self.invalid(key, &format!("one of {}", names.join(", "))))
    }

    /// A header set's name, or a table of templates.
    fn headers(&self, key: &str) -> Result<Option<Headers>, Error> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(name)) => Ok(Some(Headers::Set(name.clone()))),
            Some(Value::Object(_)) => Ok(Some(Headers::Table(self.templates(key)?))),
            Some(_) => Err(self.invalid(key, "neither a header set's name nor a table")),
        }
    }
}

/// A text with references to values, `{step.field.0}` style: a reference
/// starts at a `{` followed by a letter or `_`, and may hold another, as in
/// `{pool.{order.0}}`; a field whose name has other characters is quoted,
/// `{config."common:web:client"}`; `|component` percent-encodes the value
/// as one component of a URL. Any other brace is text.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Reference(Reference),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub path: Vec<Segment>,
    /// Percent-encoded as one component.
    pub component: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Name(String),
    /// A segment whose name is another reference's value.
    Reference(Reference),
}

impl Template {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            if !rest[open + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                push_text(&mut parts, &rest[..=open]);
                rest = &rest[open + 1..];
                continue;
            }
            push_text(&mut parts, &rest[..open]);
            let (reference, after) = Reference::parse(&rest[open + 1..])?;
            parts.push(Part::Reference(reference));
            rest = after;
        }
        push_text(&mut parts, rest);
        Ok(Self { parts })
    }

    /// The reference this template is made of, alone: its value keeps its
    /// kind (bytes, a list…) instead of becoming text.
    pub fn whole(&self) -> Option<&Reference> {
        match self.parts.as_slice() {
            [Part::Reference(reference)] if !reference.component => Some(reference),
            _ => None,
        }
    }

    pub fn references(&self) -> impl Iterator<Item = &Reference> {
        self.parts.iter().filter_map(|part| match part {
            Part::Reference(reference) => Some(reference),
            Part::Text(_) => None,
        })
    }
}

fn push_text(parts: &mut Vec<Part>, text: &str) {
    if text.is_empty() {
        return;
    }
    match parts.last_mut() {
        Some(Part::Text(last)) => last.push_str(text),
        _ => parts.push(Part::Text(text.to_string())),
    }
}

impl Reference {
    /// The reference that starts `text`, just after its `{`, and the text
    /// after its `}`.
    fn parse(text: &str) -> Result<(Self, &str), String> {
        let mut path = Vec::new();
        let mut rest = text;
        loop {
            if let Some(inner) = rest.strip_prefix('{') {
                let (reference, after) = Self::parse(inner)?;
                path.push(Segment::Reference(reference));
                rest = after;
            } else if let Some(quoted) = rest.strip_prefix('"') {
                let end = quoted.find('"').ok_or_else(|| format!("quoted name without its closing `\"` near `{}`", preview(text)))?;
                path.push(Segment::Name(quoted[..end].to_string()));
                rest = &quoted[end + 1..];
            } else {
                let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(rest.len());
                if end == 0 {
                    return Err(format!("reference with an empty name near `{}`", preview(text)));
                }
                path.push(Segment::Name(rest[..end].to_string()));
                rest = &rest[end..];
            }
            match rest.chars().next() {
                Some('.') => rest = &rest[1..],
                Some('|') => {
                    let filter = rest[1..].split('}').next().unwrap_or("");
                    if filter != "component" {
                        return Err(format!("unknown filter `{filter}`: only `component`"));
                    }
                    let after = rest[1 + filter.len()..].strip_prefix('}').ok_or("reference without its `}`")?;
                    return Ok((Self { path, component: true }, after));
                }
                Some('}') => return Ok((Self { path, component: false }, &rest[1..])),
                _ => return Err(format!("reference without its `}}` near `{}`", preview(text))),
            }
        }
    }
}

impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("{")?;
        for (n, segment) in self.path.iter().enumerate() {
            if n > 0 {
                f.write_str(".")?;
            }
            match segment {
                Segment::Name(name) if is_name(name) => f.write_str(name)?,
                Segment::Name(name) => write!(f, "\"{name}\"")?,
                Segment::Reference(reference) => write!(f, "{reference}")?,
            }
        }
        f.write_str(if self.component { "|component}" } else { "}" })
    }
}

fn preview(text: &str) -> String {
    text.chars().take(24).collect()
}

/// Why a source file cannot be used. `at` is where: `source`, a flow
/// (`resolve`, `resolvers.envelope`…), a step (`sources.detail`) or a
/// route (`resolve.routes.1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not TOML, as the toml crate says, at a line when it knows it.
    Format { line: Option<usize>, message: String },
    UnknownField { at: String, field: String },
    MissingField { at: String, field: String },
    InvalidValue { at: String, field: String, detail: String },
    /// An operation this version does not have.
    UnsupportedOperation { at: String, op: String },
    /// Both or neither of the choices.
    OneOf { at: String, choices: String },
    /// A reference to nothing before it: a step not yet run, an input the
    /// flow does not have, a field a value cannot have.
    UnknownReference { at: String, reference: String },
    TypeMismatch { at: String, reference: String, expected: Kind, found: Kind },
    UnknownHeaders { at: String, name: String },
    UnknownResolver { at: String, name: String },
    UnknownRecipe { at: String, name: String },
    /// A `call` whose inputs are not the ones the recipe names.
    CallInputs { at: String, recipe: String },
    DuplicateId { at: String, id: String },
    EqualRoutes { at: String },
    /// A recipe that calls itself, through others or not.
    CallCycle { recipe: String },
    /// A regular expression that does not compile or has other than one
    /// group, as the regex crate says.
    BadPattern { at: String, detail: String },
    /// A CSS selector lol_html does not accept, as it says.
    BadSelector { at: String, detail: String },
    /// A flow whose last step cannot give what the flow must: a list, or
    /// with `resolution` a stream or a next candidate.
    WrongOutput { at: String, resolution: bool },
}

impl Error {
    fn invalid(at: &str, field: &str, detail: &str) -> Self {
        Self::InvalidValue { at: at.into(), field: field.into(), detail: detail.into() }
    }

    /// A short name of the rule broken, as the files of
    /// config/examples/rejected are named.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Format { .. } => "format",
            Self::UnknownField { .. } => "unknown-field",
            Self::MissingField { .. } => "missing-field",
            Self::InvalidValue { .. } => "invalid-value",
            Self::UnsupportedOperation { .. } => "unsupported-operation",
            Self::OneOf { .. } => "one-of",
            Self::UnknownReference { .. } => "unknown-reference",
            Self::TypeMismatch { .. } => "type-mismatch",
            Self::UnknownHeaders { .. } => "unknown-headers",
            Self::UnknownResolver { .. } => "unknown-resolver",
            Self::UnknownRecipe { .. } => "unknown-recipe",
            Self::CallInputs { .. } => "call-inputs",
            Self::DuplicateId { .. } => "duplicate-id",
            Self::EqualRoutes { .. } => "equal-routes",
            Self::CallCycle { .. } => "call-cycle",
            Self::BadPattern { .. } => "bad-pattern",
            Self::BadSelector { .. } => "bad-selector",
            Self::WrongOutput { .. } => "wrong-output",
        }
    }
}

impl Localized for Error {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Format { line: Some(line), message } => text!(language, "source-format-at-line", line = line, message = message),
            Self::Format { line: None, message } => text!(language, "source-format", message = message),
            Self::UnknownField { at, field } => text!(language, "source-unknown-field", at = at, field = field),
            Self::MissingField { at, field } => text!(language, "source-missing-field", at = at, field = field),
            Self::InvalidValue { at, field, detail } => {
                text!(language, "source-invalid-value", at = at, field = field, detail = detail)
            }
            Self::UnsupportedOperation { at, op } => text!(language, "source-unsupported-operation", at = at, op = op),
            Self::OneOf { at, choices } => text!(language, "source-one-of", at = at, choices = choices),
            Self::UnknownReference { at, reference } => {
                text!(language, "source-unknown-reference", at = at, reference = reference)
            }
            Self::TypeMismatch { at, reference, expected, found } => text!(
                language,
                "source-type-mismatch",
                at = at,
                reference = reference,
                expected = expected.name(),
                found = found.name()
            ),
            Self::UnknownHeaders { at, name } => text!(language, "source-unknown-headers", at = at, name = name),
            Self::UnknownResolver { at, name } => text!(language, "source-unknown-resolver", at = at, name = name),
            Self::UnknownRecipe { at, name } => text!(language, "source-unknown-recipe", at = at, name = name),
            Self::CallInputs { at, recipe } => text!(language, "source-call-inputs", at = at, recipe = recipe),
            Self::DuplicateId { at, id } => text!(language, "source-duplicate-id", at = at, id = id),
            Self::EqualRoutes { at } => text!(language, "source-equal-routes", at = at),
            Self::CallCycle { recipe } => text!(language, "source-call-cycle", recipe = recipe),
            Self::BadPattern { at, detail } => text!(language, "source-bad-pattern", at = at, detail = detail),
            Self::BadSelector { at, detail } => text!(language, "source-bad-selector", at = at, detail = detail),
            Self::WrongOutput { at, resolution: false } => text!(language, "source-wrong-output-list", at = at),
            Self::WrongOutput { at, resolution: true } => text!(language, "source-wrong-output-resolution", at = at),
        }
    }
}

display_in_english!(Error);

#[cfg(test)]
mod tests;
