//! What each operation of the vocabulary does with its parameters.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use base64::Engine as _;
use serde_json::Value as Json;
use url::Url;

use super::value::{Scope, Value};
use super::{Engine, Failure, MAX_ENTRIES, Request, Resolved};
use crate::source::{Alphabet, Field, Mapping, Method, Operation};

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
        Operation::ListMap { list, map, filter, present } => {
            let entries = scope.fill(list).and_then(|v| v.list()).map_err(changed)?;
            if entries.len() > MAX_ENTRIES {
                return Err(Failure::limit(at, format!("a list of {} entries, at most {MAX_ENTRIES}", entries.len())));
            }
            let mut mapped = Vec::new();
            for entry in &entries {
                let scope = Scope { inputs, steps, entry: Some(entry) };
                if !keep(entry, filter, &scope).map_err(changed)? || !present.iter().all(|field| has(entry, field)) {
                    continue;
                }
                mapped.push(match map {
                    Mapping::Fields(fields) => Value::Record(record(fields, &scope).map_err(changed)?),
                    Mapping::Call { recipe, input } => {
                        let given = input.iter().map(|(k, t)| Ok((k.clone(), scope.fill(t)?))).collect::<Result<_, String>>();
                        engine.call(at, recipe, given.map_err(changed)?)?
                    }
                });
            }
            Ok(Value::List(mapped))
        }
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
fn keep(entry: &Value, filter: &BTreeMap<String, crate::source::Template>, scope: &Scope) -> Result<bool, String> {
    for (field, template) in filter {
        let Ok(value) = entry.get(field) else { return Ok(false) };
        if value.text().ok() != Some(scope.text(template)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The fields of `list.map`'s `item` for one entry, as text.
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

/// An M3U playlist's entries: title, url, logo, group.
fn m3u(text: &str) -> Vec<Value> {
    let attribute = regex::Regex::new(r#"([\w-]+)="([^"]*)""#).expect("constant pattern");
    let mut entries = Vec::new();
    let mut info: Option<BTreeMap<String, Value>> = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let attributes: BTreeMap<&str, &str> = attribute.captures_iter(rest).map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str())).collect();
            let title = rest.rsplit_once(',').map_or("", |(_, t)| t).trim();
            let mut fields = BTreeMap::from([("title".to_string(), Value::Text(title.into()))]);
            for (field, name) in [("logo", "tvg-logo"), ("group", "group-title")] {
                fields.insert(field.into(), Value::Text(attributes.get(name).copied().unwrap_or("").into()));
            }
            info = Some(fields);
        } else if !line.is_empty() && !line.starts_with('#') {
            let mut fields = info.take().unwrap_or_else(|| BTreeMap::from([("title".to_string(), Value::Text(line.into()))]));
            fields.insert("url".into(), Value::Text(line.into()));
            entries.push(Value::Record(fields));
        }
    }
    entries
}

/// The values of `attribute` on the elements `selector` matches, in
/// document order; the first only, when `first`.
fn html_attributes(html: &str, selector: &str, attribute: &str, first: bool) -> Result<Vec<String>, String> {
    let selector: lol_html::Selector = selector.parse().map_err(|e: lol_html::errors::SelectorError| e.to_string())?;
    let found = RefCell::new(Vec::new());
    let handler = lol_html::ElementContentHandlers::default().element(|element: &mut lol_html::html_content::Element| {
        if let Some(value) = element.get_attribute(attribute) {
            found.borrow_mut().push(value);
        }
        Ok(())
    });
    let settings = lol_html::Settings::new().append_element_content_handler((Cow::Owned(selector), handler));
    let mut rewriter = lol_html::HtmlRewriter::new(settings, |_: &[u8]| {});
    rewriter.write(html.as_bytes()).map_err(|e| e.to_string())?;
    rewriter.end().map_err(|e| e.to_string())?;
    let mut found = found.into_inner();
    if first {
        found.truncate(1);
    }
    Ok(found)
}

/// The JSON value after `"key":` in the payloads a page pushes to
/// `self.__next_f`, decoded as data: nothing of the page runs.
fn nextflight(html: &str, key: &str) -> Result<Value, String> {
    let marker = regex::Regex::new(&format!(r#""{}"\s*:\s*"#, regex::escape(key))).map_err(|e| e.to_string())?;
    for (start, _) in html.match_indices("self.__next_f.push(") {
        let rest = &html[start + "self.__next_f.push(".len()..];
        let Some(Ok(Json::Array(argument))) = serde_json::Deserializer::from_str(rest).into_iter::<Json>().next() else { continue };
        let Some(Json::String(payload)) = argument.get(1) else { continue };
        for found in marker.find_iter(payload) {
            if let Some(Ok(value)) = serde_json::Deserializer::from_str(&payload[found.end()..]).into_iter::<Json>().next() {
                return Ok(Value::Json(value));
            }
        }
    }
    Err(format!("no `{key}` in the page's data"))
}

/// A Protobuf message: varints and fixed numbers as JSON numbers,
/// length-delimited fields as bytes (a text, or a message to read with
/// another step).
fn protobuf(data: &[u8]) -> Result<Value, String> {
    let mut fields: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut pos = 0;
    let varint = |pos: &mut usize| -> Result<u64, String> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *data.get(*pos).ok_or("Protobuf varint cut short")?;
            *pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        Err("Protobuf varint too long".into())
    };
    while pos < data.len() {
        let tag = varint(&mut pos)?;
        let (field, wire) = (tag >> 3, tag & 7);
        if field == 0 {
            return Err("Protobuf field 0".into());
        }
        let value = match wire {
            0 => Value::Json(varint(&mut pos)?.into()),
            1 | 5 => {
                let size = if wire == 1 { 8 } else { 4 };
                let bytes = data.get(pos..pos + size).ok_or("Protobuf number cut short")?;
                pos += size;
                let mut buffer = [0u8; 8];
                buffer[..size].copy_from_slice(bytes);
                Value::Json(u64::from_le_bytes(buffer).into())
            }
            2 => {
                let size = usize::try_from(varint(&mut pos)?).map_err(|_| "Protobuf field too long")?;
                let bytes = data.get(pos..pos.saturating_add(size)).ok_or("Protobuf field cut short")?;
                pos += size;
                Value::Bytes(bytes.to_vec())
            }
            other => return Err(format!("Protobuf wire type {other}")),
        };
        fields.entry(field.to_string()).or_default().push(value);
    }
    Ok(Value::Message(fields))
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

/// AES with a key of 16, 24 or 32 bytes.
mod cipher {
    use aes::{Aes128, Aes192, Aes256};
    use aes_gcm::aead::consts::U12;
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{AesGcm, Nonce};
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, block_padding::Pkcs7};

    use crate::source::Mode;

    macro_rules! by_key {
        ($key:expr, $cipher:ident => $body:expr) => {
            match $key.len() {
                16 => {
                    type $cipher = Aes128;
                    $body
                }
                24 => {
                    type $cipher = Aes192;
                    $body
                }
                32 => {
                    type $cipher = Aes256;
                    $body
                }
                n => Err(format!("AES key of {n} bytes")),
            }
        };
    }

    pub fn decrypt(mode: Mode, key: &[u8], iv: &[u8], data: &[u8], aad: Option<&[u8]>) -> Result<Vec<u8>, String> {
        match mode {
            Mode::Gcm => {
                if iv.len() != 12 {
                    return Err(format!("GCM IV of {} bytes, 12 are needed", iv.len()));
                }
                by_key!(key, C => {
                    let cipher = AesGcm::<C, U12>::new_from_slice(key).map_err(|e| e.to_string())?;
                    cipher
                        .decrypt(Nonce::<U12>::from_slice(iv), Payload { msg: data, aad: aad.unwrap_or_default() })
                        .map_err(|_| "GCM authentication failed".to_string())
                })
            }
            Mode::Ctr => ctr(key, iv, data),
            Mode::Cbc => by_key!(key, C => {
                let decryptor = cbc::Decryptor::<C>::new_from_slices(key, iv).map_err(|e| e.to_string())?;
                let mut buffer = data.to_vec();
                let plain = decryptor.decrypt_padded_mut::<Pkcs7>(&mut buffer).map_err(|_| "CBC padding wrong".to_string())?;
                Ok(plain.to_vec())
            }),
        }
    }

    /// AES-CTR, the same both ways.
    pub fn ctr(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
        by_key!(key, C => {
            let mut cipher = ctr::Ctr128BE::<C>::new_from_slices(key, iv).map_err(|e| e.to_string())?;
            let mut out = data.to_vec();
            cipher.apply_keystream(&mut out);
            Ok(out)
        })
    }
}
