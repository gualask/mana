//! The vocabulary of the steps: each operation's parameters, read from the
//! file, and the kinds of value it takes and gives. A new operation is
//! added here, with a new name: an operation never changes meaning.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Error, Fields, Template};

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    /// A request; gives the response.
    Http { url: Template, method: Method, query: BTreeMap<String, Template>, body: Option<Template>, headers: Option<Headers> },
    Json { from: Template },
    /// An M3U playlist's entries: title, url, logo, group.
    M3u { from: Template },
    /// The `attribute` of the first element `selector` matches, or of all
    /// of them; a relative URL is resolved against `base`.
    HtmlAttr { from: Template, selector: String, attribute: String, base: Option<Template>, all: bool },
    /// The JSON value after `"key":` in the payloads a page pushes to
    /// `self.__next_f`.
    NextFlight { from: Template, key: String },
    /// A Protobuf message: field number → its values.
    Protobuf { from: Template },
    /// The group of `pattern`'s first match.
    TextMatch { from: Template, pattern: String },
    /// Each entry of `list` made into the fields of `item`, or into what a
    /// recipe gives for it; only the entries whose fields equal `filter`
    /// and whose fields `present` are there and not empty.
    ListMap { list: Template, map: Mapping, filter: BTreeMap<String, Template>, present: Vec<String> },
    Text { template: Template },
    /// Fields made into one record, as `list.map` makes each entry's
    /// `item`: what a recipe gives for each entry of a list, when the
    /// fields are nested deeper than a template over the entry reaches.
    Record { fields: BTreeMap<String, Field> },
    /// A query parameter, decoded.
    UrlParam { from: Template, name: String },
    /// The query after `name=`, as it is, to its end.
    UrlQueryAfter { from: Template, name: String },
    /// A date as the source writes it, in `format` (`%Y %m %d %H %M %S` and
    /// text), at the UTC offset `zone` (`+08:00`), as an instant: RFC 3339
    /// in UTC, `2026-11-23T12:30:00Z`. `unix` and `unix_ms` read numbers.
    Date { from: Template, format: String, zone: String },
    /// The URL with its query parameter `name` set to `value`, the others kept.
    UrlSet { from: Template, name: String, value: Template },
    Base64Decode { from: Template, alphabet: Alphabet },
    Base64Encode { from: Template, alphabet: Alphabet, padding: bool },
    HexDecode { from: Template },
    HexEncode { from: Template },
    Rot47 { from: Template },
    BytesXor { a: Template, b: Template },
    /// The MD5 digest, as lowercase hex.
    Md5 { from: Template },
    AesDecrypt { mode: Mode, key: Template, iv: Template, data: Template, aad: Option<Template> },
    /// AES-CTR only.
    AesEncrypt { key: Template, iv: Template, data: Template },
    /// Runs a recipe with `input` as its inputs; gives its result.
    Call { recipe: String, input: BTreeMap<String, Template> },
    /// The media to play: the end of a resolution.
    Stream { url: Template, headers: Option<Headers>, format: Option<String>, expires_at: Option<Template> },
    /// A new candidate, routed again.
    Next { candidate: BTreeMap<String, Template> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alphabet {
    Standard,
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Gcm,
    Ctr,
    Cbc,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Headers {
    /// A header set of the source, by name.
    Set(String),
    Table(BTreeMap<String, Template>),
}

/// What `list.map` makes of each entry.
#[derive(Clone, Debug, PartialEq)]
pub enum Mapping {
    Fields(BTreeMap<String, Field>),
    /// The result of the recipe run with `input`, where `{item}` is the
    /// entry: to decode each message of a Protobuf list, for instance.
    Call { recipe: String, input: BTreeMap<String, Template> },
}

/// A field of `list.map`: a template, a group of a pattern applied to it,
/// or a date read as the `date` operation reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Value(Template),
    Match { from: Template, pattern: String },
    Date { from: Template, format: String, zone: String },
}

/// What a value is, as far as it can be known before running: what a step
/// gives, what a parameter takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Bytes,
    /// Text or bytes, as bytes.
    Data,
    /// Whatever JSON gives: checked when it runs.
    Json,
    List,
    /// A response: `body`, `text`, `json`, `header.Name`, `url`.
    Response,
    /// An item or a candidate: text fields.
    Fields,
    /// A Protobuf message or a recipe's inputs: known when it runs.
    Record,
    Stream,
    Next,
    /// Not known before running.
    Any,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Bytes => "bytes",
            Self::Data => "text or bytes",
            Self::Json => "JSON",
            Self::List => "list",
            Self::Response => "response",
            Self::Fields => "fields",
            Self::Record => "record",
            Self::Stream => "stream",
            Self::Next => "next candidate",
            Self::Any => "any value",
        }
    }

    /// Whether a value of this kind may be where `expected` is.
    pub fn fits(self, expected: Kind) -> bool {
        match expected {
            Self::Any => true,
            Self::Data => matches!(self, Self::Text | Self::Bytes | Self::Data | Self::Json | Self::Any),
            Self::Text => matches!(self, Self::Text | Self::Json | Self::Any),
            Self::List => matches!(self, Self::List | Self::Json | Self::Any),
            _ => self == expected || self == Self::Any,
        }
    }
}

impl Operation {
    /// The operation `op` of the step at `at`, with its parameters.
    pub(super) fn read(at: &str, op: &str, step: &Value) -> Result<Self, Error> {
        let fields = |allowed: &[&str]| Fields::new(at, step, &[&["id", "op"], allowed].concat());
        let alphabet = |f: &Fields| f.choice("alphabet", &[("standard", Alphabet::Standard), ("url", Alphabet::Url)]);
        Ok(match op {
            "http" => {
                let f = fields(&["url", "method", "query", "body", "headers"])?;
                let method = f.choice("method", &[("GET", Method::Get), ("POST", Method::Post)])?.unwrap_or(Method::Get);
                let (query, body, headers) = (f.templates("query")?, f.template("body")?, f.headers("headers")?);
                Self::Http { url: f.required_template("url")?, method, query, body, headers }
            }
            "json" => Self::Json { from: fields(&["from"])?.required_template("from")? },
            "m3u" => Self::M3u { from: fields(&["from"])?.required_template("from")? },
            "html.attr" => {
                let f = fields(&["from", "selector", "attribute", "base", "all"])?;
                Self::HtmlAttr {
                    from: f.required_template("from")?,
                    selector: f.required_text("selector")?,
                    attribute: f.required_text("attribute")?,
                    base: f.template("base")?,
                    all: f.bool("all")?.unwrap_or(false),
                }
            }
            "nextflight" => {
                let f = fields(&["from", "key"])?;
                Self::NextFlight { from: f.required_template("from")?, key: f.required_text("key")? }
            }
            "protobuf" => Self::Protobuf { from: fields(&["from"])?.required_template("from")? },
            "text.match" => {
                let f = fields(&["from", "pattern"])?;
                Self::TextMatch { from: f.required_template("from")?, pattern: f.required_text("pattern")? }
            }
            "list.map" => {
                let f = fields(&["list", "item", "call", "input", "where", "present"])?;
                let map = match (f.table("item")?, f.text("call")?) {
                    (Some(item), None) if f.get("input").is_none() => Mapping::Fields(fields_of(&format!("{at}.item"), item)?),
                    (None, Some(recipe)) => Mapping::Call { recipe, input: f.templates("input")? },
                    _ => return Err(Error::OneOf { at: at.into(), choices: "`item`, `call` with its `input`".into() }),
                };
                Self::ListMap { list: f.required_template("list")?, map, filter: f.templates("where")?, present: f.texts("present")? }
            }
            "text" => Self::Text { template: fields(&["template"])?.required_template("template")? },
            "record" => {
                let f = fields(&["fields"])?;
                let table = f.table("fields")?.ok_or_else(|| f.missing("fields"))?;
                Self::Record { fields: fields_of(&format!("{at}.fields"), table)? }
            }
            "date" => {
                let f = fields(&["from", "format", "zone"])?;
                let format = f.required_text("format")?;
                let zone = date_zone(&f)?;
                if let Err(detail) = super::date::check_format(&format) {
                    return Err(f.invalid("format", &detail));
                }
                Self::Date { from: f.required_template("from")?, format, zone }
            }
            "url.param" | "url.query_after" => {
                let f = fields(&["from", "name"])?;
                let (from, name) = (f.required_template("from")?, f.required_text("name")?);
                if op == "url.param" { Self::UrlParam { from, name } } else { Self::UrlQueryAfter { from, name } }
            }
            "url.set" => {
                let f = fields(&["from", "name", "value"])?;
                Self::UrlSet { from: f.required_template("from")?, name: f.required_text("name")?, value: f.required_template("value")? }
            }
            "base64.decode" => {
                let f = fields(&["from", "alphabet"])?;
                Self::Base64Decode { from: f.required_template("from")?, alphabet: alphabet(&f)?.unwrap_or(Alphabet::Standard) }
            }
            "base64.encode" => {
                let f = fields(&["from", "alphabet", "padding"])?;
                Self::Base64Encode {
                    from: f.required_template("from")?,
                    alphabet: alphabet(&f)?.unwrap_or(Alphabet::Standard),
                    padding: f.bool("padding")?.unwrap_or(true),
                }
            }
            "hex.decode" => Self::HexDecode { from: fields(&["from"])?.required_template("from")? },
            "hex.encode" => Self::HexEncode { from: fields(&["from"])?.required_template("from")? },
            "rot47" => Self::Rot47 { from: fields(&["from"])?.required_template("from")? },
            "bytes.xor" => {
                let f = fields(&["a", "b"])?;
                Self::BytesXor { a: f.required_template("a")?, b: f.required_template("b")? }
            }
            "md5" => Self::Md5 { from: fields(&["from"])?.required_template("from")? },
            "aes.decrypt" => {
                let f = fields(&["mode", "key", "iv", "data", "aad"])?;
                let modes = [("gcm", Mode::Gcm), ("ctr", Mode::Ctr), ("cbc", Mode::Cbc)];
                let mode = f.choice("mode", &modes)?.ok_or_else(|| f.missing("mode"))?;
                let aad = f.template("aad")?;
                if aad.is_some() && mode != Mode::Gcm {
                    return Err(f.invalid("aad", "only with `mode = \"gcm\"`"));
                }
                Self::AesDecrypt { mode, key: f.required_template("key")?, iv: f.required_template("iv")?, data: f.required_template("data")?, aad }
            }
            "aes.encrypt" => {
                let f = fields(&["mode", "key", "iv", "data"])?;
                f.choice("mode", &[("ctr", Mode::Ctr)])?.ok_or_else(|| f.missing("mode"))?;
                Self::AesEncrypt { key: f.required_template("key")?, iv: f.required_template("iv")?, data: f.required_template("data")? }
            }
            "call" => {
                let f = fields(&["recipe", "input"])?;
                Self::Call { recipe: f.required_text("recipe")?, input: f.templates("input")? }
            }
            "stream" => {
                let f = fields(&["url", "headers", "format", "expires_at"])?;
                let format = f.choice("format", &[("hls", "hls"), ("dash", "dash"), ("progressive", "progressive")])?;
                Self::Stream {
                    url: f.required_template("url")?,
                    headers: f.headers("headers")?,
                    format: format.map(str::to_string),
                    expires_at: f.template("expires_at")?,
                }
            }
            "next" => {
                let f = fields(&["candidate"])?;
                let candidate = f.templates("candidate")?;
                if !candidate.contains_key("id") || !candidate.contains_key("url") {
                    return Err(f.invalid("candidate", "needs `id` and `url`"));
                }
                Self::Next { candidate }
            }
            other => return Err(Error::UnsupportedOperation { at: at.into(), op: other.into() }),
        })
    }

    /// What the step gives.
    pub fn output(&self) -> Kind {
        match self {
            Self::Http { .. } => Kind::Response,
            Self::Json { .. } | Self::NextFlight { .. } => Kind::Json,
            Self::M3u { .. } | Self::ListMap { .. } => Kind::List,
            Self::HtmlAttr { all: true, .. } => Kind::List,
            Self::Protobuf { .. } => Kind::Record,
            Self::Base64Decode { .. } | Self::HexDecode { .. } | Self::BytesXor { .. } => Kind::Bytes,
            Self::AesDecrypt { .. } | Self::AesEncrypt { .. } => Kind::Bytes,
            Self::Call { .. } => Kind::Any,
            Self::Record { .. } => Kind::Fields,
            Self::Stream { .. } => Kind::Stream,
            Self::Next { .. } => Kind::Next,
            Self::HtmlAttr { .. } | Self::TextMatch { .. } | Self::Text { .. } | Self::UrlParam { .. } => Kind::Text,
            Self::UrlQueryAfter { .. } | Self::UrlSet { .. } | Self::Base64Encode { .. } | Self::HexEncode { .. } => Kind::Text,
            Self::Rot47 { .. } | Self::Md5 { .. } | Self::Date { .. } => Kind::Text,
        }
    }

    /// Each template of the step, with what it must give. Those of
    /// `list.map` but its `list` are not here: they read the entry
    /// (`{item.…}`); nor are the fields of `record`, which may be
    /// patterns and dates.
    pub fn parameters(&self) -> Vec<(&'static str, &Template, Kind)> {
        use Kind::*;
        let mut parameters = match self {
            Self::Http { url, query, body, .. } => {
                let mut p = vec![("url", url, Text)];
                p.extend(query.values().map(|t| ("query", t, Text)));
                p.extend(body.iter().map(|t| ("body", t, Data)));
                p
            }
            Self::Json { from } => vec![("from", from, Data)],
            Self::M3u { from } | Self::NextFlight { from, .. } | Self::TextMatch { from, .. } => vec![("from", from, Text)],
            Self::HtmlAttr { from, base, .. } => {
                let mut p = vec![("from", from, Text)];
                p.extend(base.iter().map(|t| ("base", t, Text)));
                p
            }
            Self::Protobuf { from } | Self::Base64Encode { from, .. } | Self::HexEncode { from } => vec![("from", from, Bytes)],
            Self::ListMap { list, .. } => vec![("list", list, List)],
            Self::Text { template } => vec![("template", template, Text)],
            Self::Date { from, .. } => vec![("from", from, Text)],
            Self::UrlParam { from, .. } | Self::UrlQueryAfter { from, .. } => vec![("from", from, Text)],
            Self::UrlSet { from, value, .. } => vec![("from", from, Text), ("value", value, Text)],
            Self::Base64Decode { from, .. } | Self::HexDecode { from } => vec![("from", from, Text)],
            Self::Rot47 { from } | Self::Md5 { from } => vec![("from", from, Data)],
            Self::BytesXor { a, b } => vec![("a", a, Bytes), ("b", b, Bytes)],
            Self::AesDecrypt { key, iv, data, aad, .. } => {
                let mut p = vec![("key", key, Data), ("iv", iv, Data), ("data", data, Bytes)];
                p.extend(aad.iter().map(|t| ("aad", t, Data)));
                p
            }
            Self::AesEncrypt { key, iv, data } => vec![("key", key, Data), ("iv", iv, Data), ("data", data, Data)],
            Self::Call { input, .. } => input.values().map(|t| ("input", t, Any)).collect(),
            Self::Record { .. } => Vec::new(),
            Self::Stream { url, expires_at, .. } => {
                let mut p = vec![("url", url, Text)];
                p.extend(expires_at.iter().map(|t| ("expires_at", t, Text)));
                p
            }
            Self::Next { candidate } => candidate.values().map(|t| ("candidate", t, Text)).collect(),
        };
        if let Self::Http { headers: Some(Headers::Table(headers)), .. } | Self::Stream { headers: Some(Headers::Table(headers)), .. } = self {
            parameters.extend(headers.values().map(|t| ("headers", t, Text)));
        }
        parameters
    }
}

/// The fields of a `list.map` item or of a `record`.
fn fields_of(at: &str, table: &serde_json::Map<String, Value>) -> Result<BTreeMap<String, Field>, Error> {
    table.iter().map(|(name, value)| Ok((name.clone(), field(at, name, value)?))).collect()
}

fn field(at: &str, name: &str, value: &Value) -> Result<Field, Error> {
    match value {
        Value::String(text) => Template::parse(text).map(Field::Value).map_err(|detail| Error::invalid(at, name, &detail)),
        Value::Object(_) => {
            let f = Fields::new(&format!("{at}.{name}"), value, &["from", "pattern", "format", "zone"])?;
            let from = f.required_template("from")?;
            match (f.text("pattern")?, f.text("format")?) {
                (Some(pattern), None) if f.get("zone").is_none() => Ok(Field::Match { from, pattern }),
                (None, Some(format)) => {
                    let zone = date_zone(&f)?;
                    super::date::check_format(&format).map_err(|detail| f.invalid("format", &detail))?;
                    Ok(Field::Date { from, format, zone })
                }
                _ => Err(Error::OneOf { at: format!("{at}.{name}"), choices: "`pattern`, `format` with its `zone`".into() }),
            }
        }
        _ => Err(Error::invalid(at, name, "neither a text nor `{ from, pattern }` nor `{ from, format, zone }`")),
    }
}

/// The `zone` of a date, UTC when not given.
fn date_zone(f: &Fields) -> Result<String, Error> {
    let zone = f.text("zone")?.unwrap_or_else(|| "+00:00".into());
    match super::date::offset(&zone) {
        Some(_) => Ok(zone),
        None => Err(f.invalid("zone", "an offset as `+08:00` or `-05:30`")),
    }
}
