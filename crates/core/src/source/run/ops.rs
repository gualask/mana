//! What each operation of the vocabulary does with its parameters.

use std::collections::BTreeMap;
use std::rc::Rc;

use base64::Engine as _;
use serde_json::Value as Json;
use url::Url;

mod cipher;
mod formats;

use formats::{html_attributes, m3u, nextflight, protobuf};

use super::value::{Scope, Value};
use super::{Engine, Failure, MAX_ENTRIES, Outcome, Request, Resolved};
use crate::log::debug;
use crate::source::{Alphabet, Field, Mapping, Method, Operation, Template};

/// The value `operation` gives, at `at`.
pub(super) fn run(
    engine: &mut Engine,
    at: &str,
    operation: &Operation,
    inputs: &BTreeMap<String, Value>,
    steps: &BTreeMap<String, Value>,
) -> Result<Value, Failure> {
    let scope = Scope { inputs, steps, entry: None };
    let changed = |detail: String| Failure::changed(at, detail);
    match operation {
        Operation::Http { url, method, query, body, headers } => {
            let mut url = Url::parse(&scope.text(url).map_err(changed)?).map_err(|e| changed(format!("not a URL: {e}")))?;
            for (name, template) in query {
                let value = scope.text(template).map_err(changed)?;
                url.query_pairs_mut().append_pair(name, &value);
            }
            let body = body.as_ref().map(|t| scope.fill(t)?.bytes()).transpose().map_err(changed)?;
            let headers = engine.headers(headers.as_ref(), &scope).map_err(changed)?;
            let method = match method {
                Method::Get => "GET",
                Method::Post => "POST",
            };
            let answer = engine.send(at, Request { method, url, headers, body })?;
            Ok(Value::Response(Rc::new(answer)))
        }
        Operation::ListMap { list, map, filter, present } => list_map(engine, at, &scope, list, map, (filter, present)),
        Operation::Call { recipe, input } => {
            let given = input.iter().map(|(k, t)| Ok((k.clone(), scope.fill(t)?))).collect::<Result<_, String>>();
            engine.call(at, recipe, given.map_err(changed)?)
        }
        Operation::Stream { url, headers, format, expires_at } => {
            let url = Url::parse(&scope.text(url).map_err(changed)?).map_err(|e| changed(format!("not a URL: {e}")))?;
            let headers = engine.headers(headers.as_ref(), &scope).map_err(changed)?;
            let expires_at = expires_at.as_ref().map(|t| scope.text(t)).transpose().map_err(changed)?;
            Ok(Value::Stream(Resolved { url, headers, format: format.clone(), expires_at }))
        }
        other => pure(other, &scope).map_err(changed),
    }
}

/// `list.map`: each entry of `list` that passes `filter` and has the
/// fields `present`, mapped; an entry of another shape left out.
fn list_map(
    engine: &mut Engine,
    at: &str,
    scope: &Scope,
    list: &Template,
    map: &Mapping,
    (filter, present): (&BTreeMap<String, Template>, &[String]),
) -> Result<Value, Failure> {
    let changed = |detail: String| Failure::changed(at, detail);
    let entries = scope.fill(list).and_then(|v| v.list()).map_err(changed)?;
    if entries.len() > MAX_ENTRIES {
        return Err(Failure::limit(at, format!("a list of {} entries, at most {MAX_ENTRIES}", entries.len())));
    }
    let (mut mapped, mut skipped) = (Vec::new(), Vec::new());
    for entry in &entries {
        let scope = Scope { inputs: scope.inputs, steps: scope.steps, entry: Some(entry) };
        if !keep(entry, filter, &scope).map_err(changed)? || !present.iter().all(|field| has(entry, field)) {
            continue;
        }
        match map_entry(engine, at, map, &scope) {
            Ok(value) => mapped.push(value),
            // An entry of another shape is left out, as an M3U reader leaves out a
            // broken line: one odd event must not take the catalogue with it.
            Err(failure) if failure.outcome == Outcome::SourceChanged => skipped.push(failure),
            Err(failure) => return Err(failure),
        }
    }
    mapped_unless_all_left_out(at, mapped, &skipped)
}

/// What `list.map` makes of the entry `scope` holds: its `item`, or what
/// its recipe gives.
fn map_entry(engine: &mut Engine, at: &str, map: &Mapping, scope: &Scope) -> Result<Value, Failure> {
    let changed = |detail: String| Failure::changed(at, detail);
    match map {
        Mapping::Fields(fields) => record(fields, scope).map(Value::Record).map_err(changed),
        Mapping::Call { recipe, input } => {
            let given = input.iter().map(|(k, t)| Ok((k.clone(), scope.fill(t)?))).collect::<Result<_, String>>();
            given.map_err(changed).and_then(|given| engine.call(at, recipe, given))
        }
    }
}

/// The entries `list.map` made, when it made any or left none out; else
/// why the first one was left out.
fn mapped_unless_all_left_out(at: &str, mapped: Vec<Value>, skipped: &[Failure]) -> Result<Value, Failure> {
    let Some(first) = skipped.first() else { return Ok(Value::List(mapped)) };
    if mapped.is_empty() {
        return Err(first.clone());
    }
    debug!("{at}: {} of {} entries left out, the first: {first}", skipped.len(), skipped.len() + mapped.len());
    Ok(Value::List(mapped))
}

/// The operations that only transform values.
fn pure(operation: &Operation, scope: &Scope) -> Result<Value, String> {
    Ok(match operation {
        Operation::Json { from } => {
            let data = scope.fill(from)?.bytes()?;
            Value::Json(serde_json::from_slice(&data).map_err(|e| format!("not JSON: {e}"))?)
        }
        Operation::M3u { from } => Value::List(m3u(&scope.text(from)?)),
        Operation::HtmlAttr { from, selector, attribute, base, all } => {
            let mut found = html_attributes(&scope.text(from)?, selector, attribute, !all)?;
            if let Some(base) = base {
                let base = Url::parse(&scope.text(base)?).map_err(|e| format!("base not a URL: {e}"))?;
                found = found.into_iter().map(|v| base.join(&v).map(String::from).unwrap_or(v)).collect();
            }
            if *all {
                texts(found)
            } else {
                Value::Text(found.into_iter().next().ok_or_else(|| format!("no element `{selector}` with `{attribute}`"))?)
            }
        }
        Operation::NextFlight { from, key } => nextflight(&scope.text(from)?, key)?,
        Operation::Protobuf { from } => protobuf(&scope.fill(from)?.bytes()?)?,
        Operation::TextMatch { from, pattern } => Value::Text(group(pattern, &scope.text(from)?)?),
        Operation::Text { template } => Value::Text(scope.text(template)?),
        Operation::Record { fields } => Value::Record(record(fields, scope)?),
        Operation::Date { from, format, zone } => Value::Text(date(&scope.text(from)?, format, zone)?),
        Operation::UrlParam { from, name } => {
            let url = Url::parse(&scope.text(from)?).map_err(|e| format!("not a URL: {e}"))?;
            let found = url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
            Value::Text(found.ok_or_else(|| format!("no parameter `{name}`"))?)
        }
        Operation::UrlQueryAfter { from, name } => {
            let url = Url::parse(&scope.text(from)?).map_err(|e| format!("not a URL: {e}"))?;
            let query = url.query().unwrap_or("");
            let start = format!("{name}=");
            let at = if query.starts_with(&start) { Some(0) } else { query.find(&format!("&{start}")).map(|n| n + 1) };
            Value::Text(at.map(|n| query[n + start.len()..].to_string()).ok_or_else(|| format!("no parameter `{name}`"))?)
        }
        Operation::UrlSet { from, name, value } => {
            let mut url = Url::parse(&scope.text(from)?).map_err(|e| format!("not a URL: {e}"))?;
            let others: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != name).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            url.query_pairs_mut().clear().extend_pairs(others).append_pair(name, &scope.text(value)?);
            Value::Text(url.into())
        }
        Operation::Base64Decode { from, alphabet } => {
            let text = scope.text(from)?;
            let engine = base64_engine(*alphabet, false);
            Value::Bytes(engine.decode(text.trim()).map_err(|e| format!("not base64: {e}"))?)
        }
        Operation::Base64Encode { from, alphabet, padding } => Value::Text(base64_engine(*alphabet, *padding).encode(scope.fill(from)?.bytes()?)),
        Operation::HexDecode { from } => Value::Bytes(hex(&scope.text(from)?)?),
        Operation::HexEncode { from } => Value::Text(scope.fill(from)?.bytes()?.iter().map(|b| format!("{b:02x}")).collect()),
        Operation::Rot47 { from } => Value::Text(rot47(&scope.fill(from)?.text()?)),
        Operation::BytesXor { a, b } => {
            let (a, b) = (scope.fill(a)?.bytes()?, scope.fill(b)?.bytes()?);
            if a.len() != b.len() {
                return Err(format!("XOR of {} and {} bytes", a.len(), b.len()));
            }
            Value::Bytes(a.iter().zip(&b).map(|(x, y)| x ^ y).collect())
        }
        Operation::Md5 { from } => {
            use md5::Digest;
            Value::Text(md5::Md5::digest(scope.fill(from)?.bytes()?).iter().map(|b| format!("{b:02x}")).collect())
        }
        Operation::AesDecrypt { mode, key, iv, data, aad } => {
            let aad = aad.as_ref().map(|t| scope.fill(t)?.bytes()).transpose()?;
            let (key, iv, data) = (scope.fill(key)?.bytes()?, scope.fill(iv)?.bytes()?, scope.fill(data)?.bytes()?);
            Value::Bytes(cipher::decrypt(*mode, &key, &iv, &data, aad.as_deref())?)
        }
        Operation::AesEncrypt { key, iv, data } => {
            let (key, iv, data) = (scope.fill(key)?.bytes()?, scope.fill(iv)?.bytes()?, scope.fill(data)?.bytes()?);
            Value::Bytes(cipher::ctr(&key, &iv, &data)?)
        }
        Operation::Next { candidate } => {
            Value::Next(candidate.iter().map(|(k, t)| Ok((k.clone(), scope.text(t)?))).collect::<Result<_, String>>()?)
        }
        Operation::Http { .. } | Operation::ListMap { .. } | Operation::Call { .. } | Operation::Stream { .. } => {
            unreachable!("run handles the operations that reach the engine")
        }
    })
}

/// `text` in `format` at the offset `zone`, as RFC 3339 in UTC; format and
/// zone are checked before a source is used.
fn date(text: &str, format: &str, zone: &str) -> Result<String, String> {
    let offset = crate::source::date::offset(zone).unwrap_or(0);
    Ok(crate::source::date::rfc3339(crate::source::date::parse(text, format, offset)?))
}

/// Whether `entry` has `field`, not empty: a text, a list or an object
/// with something in it, or any other value.
fn has(entry: &Value, field: &str) -> bool {
    match entry.get(field) {
        Ok(Value::Text(text)) => !text.is_empty(),
        Ok(Value::List(list)) => !list.is_empty(),
        Ok(Value::Json(Json::Null)) | Err(_) => false,
        Ok(Value::Json(Json::String(text))) => !text.is_empty(),
        Ok(Value::Json(Json::Array(list))) => !list.is_empty(),
        Ok(Value::Json(Json::Object(map))) => !map.is_empty(),
        Ok(_) => true,
    }
}

fn texts(found: Vec<String>) -> Value {
    Value::List(found.into_iter().map(Value::Text).collect())
}

/// Whether `entry`'s fields equal those `filter` gives.
fn keep(entry: &Value, filter: &BTreeMap<String, Template>, scope: &Scope) -> Result<bool, String> {
    for (field, template) in filter {
        let Ok(value) = entry.get(field) else { return Ok(false) };
        if value.text().ok() != Some(scope.text(template)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The fields of `list.map`'s `item` for one entry, or of a `record`, as
/// text.
fn record(fields: &BTreeMap<String, Field>, scope: &Scope) -> Result<BTreeMap<String, Value>, String> {
    let mut record = BTreeMap::new();
    for (name, field) in fields {
        let text = match field {
            Field::Value(template) => scope.text(template)?,
            Field::Match { from, pattern } => group(pattern, &scope.text(from)?)?,
            Field::Date { from, format, zone } => date(&scope.text(from)?, format, zone)?,
        };
        record.insert(name.clone(), Value::Text(text));
    }
    Ok(record)
}

/// The group of `pattern`'s first match in `text`; patterns are checked
/// before a source is used.
fn group(pattern: &str, text: &str) -> Result<String, String> {
    let regex = regex::Regex::new(pattern).map_err(|e| e.to_string())?;
    let captures = regex.captures(text).ok_or_else(|| format!("`{pattern}` does not match"))?;
    Ok(captures.get(1).map_or("", |m| m.as_str()).to_string())
}

fn base64_engine(alphabet: Alphabet, padding: bool) -> base64::engine::GeneralPurpose {
    use base64::alphabet::{STANDARD, URL_SAFE};
    use base64::engine::{DecodePaddingMode, GeneralPurposeConfig};
    let alphabet = match alphabet {
        Alphabet::Standard => &STANDARD,
        Alphabet::Url => &URL_SAFE,
    };
    let config = GeneralPurposeConfig::new().with_encode_padding(padding).with_decode_padding_mode(DecodePaddingMode::Indifferent);
    base64::engine::GeneralPurpose::new(alphabet, config)
}

fn hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err("hex of odd length".into());
    }
    (0..text.len()).step_by(2).map(|n| u8::from_str_radix(&text[n..n + 2], 16).map_err(|_| "not hex".to_string())).collect()
}

fn rot47(text: &str) -> String {
    text.chars().map(|c| if ('!'..='~').contains(&c) { char::from(b'!' + (c as u8 - b'!' + 47) % 94) } else { c }).collect()
}
