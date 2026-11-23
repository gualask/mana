//! The values steps give and read, and the filling of templates.

use std::collections::BTreeMap;
use std::rc::Rc;

use serde_json::Value as Json;

use super::Resolved;
use crate::net::Answer;
use crate::source::{Part, Reference, Segment, Template};

#[derive(Clone, Debug)]
pub(super) enum Value {
    Text(String),
    Bytes(Vec<u8>),
    Json(Json),
    List(Vec<Value>),
    /// Named values: an item, a candidate, a recipe's inputs, a response's
    /// headers (their names in lowercase).
    Record(BTreeMap<String, Value>),
    /// A Protobuf message: field number → its values; an absent field is
    /// an empty list, as Protobuf reads it.
    Message(BTreeMap<String, Vec<Value>>),
    Response(Rc<Answer>),
    Stream(Resolved),
    Next(BTreeMap<String, String>),
}

impl Value {
    /// The value at `key` inside this one, if it has it.
    pub fn get(&self, key: &str) -> Result<Value, String> {
        let missing = || format!("no `{key}`");
        match self {
            Self::Json(json) => {
                let inner = match json {
                    Json::Object(map) => map.get(key),
                    Json::Array(list) => key.parse::<usize>().ok().and_then(|n| list.get(n)),
                    _ => None,
                };
                inner.map(|json| Self::Json(json.clone())).ok_or_else(missing)
            }
            Self::List(list) => key.parse::<usize>().ok().and_then(|n| list.get(n)).cloned().ok_or_else(missing),
            Self::Record(record) => {
                record.get(key).or_else(|| record.get(&key.to_ascii_lowercase())).cloned().ok_or_else(missing)
            }
            Self::Message(fields) => Ok(Self::List(fields.get(key).cloned().unwrap_or_default())),
            Self::Response(answer) => match key {
                "body" => Ok(Self::Bytes(answer.body.clone())),
                "text" => Ok(Self::Text(String::from_utf8_lossy(&answer.body).into_owned())),
                "url" => Ok(Self::Text(answer.url.to_string())),
                "json" => serde_json::from_slice(&answer.body).map(Self::Json).map_err(|e| format!("not JSON: {e}")),
                "header" => {
                    let headers = answer.headers.iter().map(|(name, value)| (name.to_ascii_lowercase(), Self::Text(value.clone())));
                    Ok(Self::Record(headers.collect()))
                }
                _ => Err(missing()),
            },
            Self::Next(fields) => fields.get(key).map(|text| Self::Text(text.clone())).ok_or_else(missing),
            Self::Text(_) | Self::Bytes(_) | Self::Stream(_) => Err(missing()),
        }
    }

    /// As text: bytes as UTF-8, a JSON text, number or boolean as written.
    pub fn text(&self) -> Result<String, String> {
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::Bytes(bytes) => String::from_utf8(bytes.clone()).map_err(|_| "bytes that are not UTF-8 text".into()),
            Self::Json(Json::String(text)) => Ok(text.clone()),
            Self::Json(json @ (Json::Number(_) | Json::Bool(_))) => Ok(json.to_string()),
            other => Err(format!("{} where a text is needed", other.name())),
        }
    }

    /// As bytes: text as UTF-8.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::Bytes(bytes) => Ok(bytes.clone()),
            other => other.text().map(String::into_bytes),
        }
    }

    pub fn list(&self) -> Result<Vec<Value>, String> {
        match self {
            Self::List(list) => Ok(list.clone()),
            Self::Json(Json::Array(list)) => Ok(list.iter().cloned().map(Self::Json).collect()),
            other => Err(format!("{} where a list is needed", other.name())),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Text(_) => "a text",
            Self::Bytes(_) => "bytes",
            Self::Json(Json::Null) => "null",
            Self::Json(Json::Object(_)) => "a JSON object",
            Self::Json(Json::Array(_)) => "a JSON list",
            Self::Json(_) => "a JSON value",
            Self::List(_) => "a list",
            Self::Record(_) | Self::Message(_) => "a record",
            Self::Response(_) => "a response",
            Self::Stream(_) => "a stream",
            Self::Next(_) => "a next candidate",
        }
    }
}

/// What a template can read: the flow's inputs, the steps run so far and,
/// inside `list.map`, the entry.
pub(super) struct Scope<'a> {
    pub inputs: &'a BTreeMap<String, Value>,
    pub steps: &'a BTreeMap<String, Value>,
    pub entry: Option<&'a Value>,
}

impl Scope<'_> {
    /// The template's value: a reference alone keeps its value, anything
    /// else is text, each value inserted as it is or, with `|component`,
    /// percent-encoded.
    pub fn fill(&self, template: &Template) -> Result<Value, String> {
        if let Some(reference) = template.whole() {
            return self.resolve(reference);
        }
        let mut text = String::new();
        for part in &template.parts {
            match part {
                Part::Text(literal) => text.push_str(literal),
                Part::Reference(reference) => {
                    let value = self.resolve(reference)?.text().map_err(|e| format!("{reference}: {e}"))?;
                    if reference.component {
                        text.extend(url::form_urlencoded::byte_serialize(value.as_bytes()).map(|s| s.replace('+', "%20")));
                    } else {
                        text.push_str(&value);
                    }
                }
            }
        }
        Ok(Value::Text(text))
    }

    pub fn text(&self, template: &Template) -> Result<String, String> {
        self.fill(template)?.text()
    }

    fn resolve(&self, reference: &Reference) -> Result<Value, String> {
        let mut path = reference.path.iter();
        let first = match path.next() {
            Some(Segment::Name(name)) => name.as_str(),
            _ => return Err(format!("{reference}: no name")),
        };
        let mut value = match first {
            "input" => Value::Record(self.inputs.clone()),
            "item" if self.entry.is_some() => self.entry.cloned().unwrap_or(Value::Text(String::new())),
            step => self.steps.get(step).cloned().ok_or_else(|| format!("{reference}: no step `{step}`"))?,
        };
        for segment in path {
            let key = match segment {
                Segment::Name(name) => name.clone(),
                Segment::Reference(inner) => self.resolve(inner)?.text().map_err(|e| format!("{inner}: {e}"))?,
            };
            value = value.get(&key).map_err(|e| format!("{reference}: {e}"))?;
        }
        Ok(value)
    }
}
