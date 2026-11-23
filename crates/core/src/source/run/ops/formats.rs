//! The formats steps read whole: M3U playlists, HTML attributes, the data
//! a Next.js page pushes, Protobuf messages. Each reads data only: nothing
//! of a page runs.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;

use serde_json::Value as Json;

use super::super::value::Value;

/// An M3U playlist's entries: title, url, logo, group.
pub(super) fn m3u(text: &str) -> Vec<Value> {
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
pub(super) fn html_attributes(html: &str, selector: &str, attribute: &str, first: bool) -> Result<Vec<String>, String> {
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
pub(super) fn nextflight(html: &str, key: &str) -> Result<Value, String> {
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
pub(super) fn protobuf(data: &[u8]) -> Result<Value, String> {
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
