//! Reading a step's operation from the file: its parameters, checked one
//! by one, with the place and the reason of what is wrong.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Alphabet, Field, Mapping, Method, Mode, Operation};
use crate::source::{Error, Fields, Template, date};

impl Operation {
    /// The operation `op` of the step at `at`, with its parameters.
    pub(in crate::source) fn read(at: &str, op: &str, step: &Value) -> Result<Self, Error> {
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
                if let Err(detail) = date::check_format(&format) {
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
                    date::check_format(&format).map_err(|detail| f.invalid("format", &detail))?;
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
    match date::offset(&zone) {
        Some(_) => Ok(zone),
        None => Err(f.invalid("zone", "an offset as `+08:00` or `-05:30`")),
    }
}
