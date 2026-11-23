//! The engine on the lab's chains, with a network that answers as the lab
//! does (dev/lab/scenarios.py, binary.py): the examples of config/examples
//! run here as the app runs them, without the lab or a network.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use cbc::cipher::{KeyIvInit, StreamCipher};
use md5::Digest;

use super::*;
use crate::source::Source;

const API: &str = "http://127.0.0.1:8781";
const PLAYER: &str = "http://127.0.0.1:8782";
const MEDIA: &str = "http://127.0.0.1:8783";

fn example(name: &str) -> Source {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/examples").join(format!("{name}.toml"));
    Source::from_toml(&std::fs::read_to_string(path).unwrap()).unwrap()
}

enum Reply {
    Body(Vec<(String, String)>, Vec<u8>),
    Status(u16),
}

fn body(text: impl Into<Vec<u8>>) -> Reply {
    Reply::Body(Vec::new(), text.into())
}

/// A network that answers the URLs of `pages`, exactly, and records the
/// requests made.
struct Web {
    pages: BTreeMap<String, Reply>,
    asked: Rc<RefCell<Vec<Request>>>,
}

impl Web {
    fn new(pages: Vec<(String, Reply)>) -> Self {
        Self { pages: pages.into_iter().collect(), asked: Rc::default() }
    }

    fn fetch(&self) -> impl FnMut(&Request) -> Result<Answer, net::Error> + '_ {
        |request: &Request| {
            self.asked.borrow_mut().push(request.clone());
            match self.pages.get(request.url.as_str()) {
                Some(Reply::Body(headers, data)) => Ok(Answer { url: request.url.clone(), headers: headers.clone(), body: data.clone() }),
                Some(Reply::Status(code)) => Err(net::Error::with_status(&request.url, *code)),
                None => Err(net::Error::with_status(&request.url, 404)),
            }
        }
    }

    fn engine<'a>(&'a self, source: &'a Source) -> Engine<'a> {
        Engine::new(source, "device-1", self.fetch(), || false)
    }
}

fn item(fields: &[(&str, &str)]) -> Item {
    Item { fields: fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
}

fn candidate(fields: &[(&str, &str)]) -> Candidate {
    Candidate { fields: item(fields).fields }
}

fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn media_headers() -> Vec<(String, String)> {
    headers(&[("Referer", "http://127.0.0.1:8782/"), ("User-Agent", "StreamLab/1"), ("X-Lab-Media", "synthetic")])
}

/// The lab's envelope page: AES-256-GCM of `{"u": target}` under a key
/// masked by XOR, as dev/lab/crypto.cjs seals it.
fn envelope(target: &str, tamper: bool) -> String {
    let (key, mask, iv, aad) = ([7u8; 32], [0x5au8; 32], [9u8; 12], b"stream-synthetic-envelope:v1");
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap();
    let plain = format!(r#"{{"u":"{target}","e":1}}"#);
    let mut data = cipher.encrypt(aes_gcm::Nonce::from_slice(&iv), Payload { msg: plain.as_bytes(), aad }).unwrap();
    if tamper {
        data[0] ^= 1;
    }
    let masked: Vec<u8> = key.iter().zip(mask).map(|(k, m)| k ^ m).collect();
    let mut pool: Vec<String> = [&masked[..], &mask, &iv, &data, aad].iter().map(|v| URL_SAFE_NO_PAD.encode(v)).collect();
    pool.extend(["crypto", "subtle", "importKey", "decrypt", "AES-GCM", "raw"].map(|v| URL_SAFE_NO_PAD.encode(v)));
    format!("<script>const _0xp={},_0xi={};</script>", serde_json::to_string(&pool).unwrap(), serde_json::to_string(&(0..11).collect::<Vec<_>>()).unwrap())
}

fn events_web(tamper: bool) -> Web {
    let media = format!("{MEDIA}/protected/master.m3u8?expires=1&token=abc");
    let player = format!("{PLAYER}/player?liveUrl={}", url::form_urlencoded::byte_serialize(media.as_bytes()).collect::<String>());
    let signals = serde_json::json!([
        {"id": "denied", "label": "Denied", "play_type": "m3u8", "url": format!("{MEDIA}/denied.m3u8")},
        {"id": "encrypted", "label": "Encrypted", "play_type": "iframe", "url": format!("{PLAYER}/wrapper")},
    ]);
    Web::new(vec![
        (format!("{API}/catalogue.json"), body(r#"[{"id": "demo", "title": "Synthetic bars and tone"}, {"id": "empty", "title": "Empty event"}]"#)),
        (format!("{API}/events/demo.json"), body(serde_json::json!({"id": "demo", "signals": signals}).to_string())),
        (format!("{API}/events/empty.json"), body(r#"{"id": "empty", "signals": []}"#)),
        (format!("{PLAYER}/wrapper"), body(envelope(&player, tamper))),
    ])
}

#[test]
fn an_event_source_lists_events_their_signals_and_resolves_each() {
    let source = example("lab-events");
    let web = events_web(false);
    let mut engine = web.engine(&source);
    let events = engine.catalogue().unwrap();
    assert_eq!(events.iter().map(|e| e.fields["id"].as_str()).collect::<Vec<_>>(), ["demo", "empty"]);
    let signals = engine.sources(&events[0]).unwrap();
    assert_eq!(signals.iter().map(|s| s.fields["id"].as_str()).collect::<Vec<_>>(), ["denied", "encrypted"]);
    assert_eq!(signals[1].fields["type"], "iframe");
    let direct = engine.resolve(&signals[0], &events[0]).unwrap();
    assert_eq!((direct.url.as_str(), direct.headers), (format!("{MEDIA}/denied.m3u8").as_str(), media_headers()));
    let resolved = engine.resolve(&signals[1], &events[0]).unwrap();
    assert_eq!(resolved.url.as_str(), format!("{MEDIA}/protected/master.m3u8?expires=1&token=abc"));
    assert_eq!(resolved.headers, media_headers());
    let empty = engine.sources(&events[1]).unwrap_err();
    assert_eq!(empty.outcome, Outcome::NoSources);
    assert_eq!(web.asked.borrow()[0].headers, headers(&[("User-Agent", "StreamLab/1")]));
}

#[test]
fn an_envelope_that_does_not_authenticate_is_a_changed_source() {
    let source = example("lab-events");
    let web = events_web(true);
    let mut engine = web.engine(&source);
    let signal = candidate(&[("id", "encrypted"), ("type", "iframe"), ("url", &format!("{PLAYER}/wrapper"))]);
    let failure = engine.resolve(&signal, &item(&[("id", "demo"), ("title", "Demo")])).unwrap_err();
    assert_eq!((failure.outcome, failure.at.as_str()), (Outcome::SourceChanged, "resolvers.envelope.plain"));
    assert!(failure.to_string().contains("GCM authentication failed"), "{failure}");
}

#[test]
fn a_recipe_called_by_a_resolver_resolves_the_same_envelope() {
    let source = example("lab-embedded");
    let rows = serde_json::json!({"streams": [{"_id": "a", "id": "encrypted", "label": "E", "play_type": "iframe", "streaming_link": format!("{PLAYER}/wrapper")}]});
    let page = format!("<script>self.__next_f.push({});</script>", serde_json::json!([1, rows.to_string()]));
    let mut web = events_web(false);
    web.pages.insert(format!("{API}/embedded"), body(page));
    let mut engine = web.engine(&source);
    let demo = &engine.catalogue().unwrap()[0];
    let signals = engine.sources(demo).unwrap();
    assert_eq!(signals[0].fields["url"], format!("{PLAYER}/wrapper"));
    assert_eq!(engine.resolve(&signals[0], demo).unwrap().url.path(), "/protected/master.m3u8");
}

#[test]
fn an_iframe_leads_to_an_api_through_attributes_of_the_pages() {
    let source = example("lab-frame");
    let web = Web::new(vec![
        (format!("{PLAYER}/frame"), body(r#"<iframe src="/embed/demo"></iframe>"#)),
        (format!("{PLAYER}/embed/demo"), body(format!(r#"<p><video data-api="{PLAYER}/api/links/demo"></video>"#))),
        (format!("{PLAYER}/api/links/demo"), body(format!(r#"{{"links": [{{"id": "s", "playUrl": "{MEDIA}/public/master.m3u8"}}]}}"#))),
    ]);
    let mut engine = web.engine(&source);
    let frame = &engine.catalogue().unwrap()[0];
    let resolved = engine.resolve(&Candidate { fields: frame.fields.clone() }, frame).unwrap();
    assert_eq!(resolved.url.as_str(), format!("{MEDIA}/public/master.m3u8"));
}

#[test]
fn an_m3u_catalogue_and_a_json_with_a_token_give_their_urls() {
    let m3u = example("lab-m3u");
    let playlist = format!("#EXTM3U\n#EXTINF:-1 tvg-logo=\"l.png\" group-title=\"G\",Synthetic bars\n{MEDIA}/public/master.m3u8\n");
    let web = Web::new(vec![(format!("{API}/channels.m3u"), body(playlist))]);
    let mut engine = web.engine(&m3u);
    let channels = engine.catalogue().unwrap();
    assert_eq!(channels[0].fields["title"], "Synthetic bars");
    assert_eq!(engine.resolve(&Candidate { fields: channels[0].fields.clone() }, &channels[0]).unwrap().url.path(), "/public/master.m3u8");
    let live = example("lab-live");
    let token = format!(r#"{{"video": {{"content_url": "{MEDIA}/live/master.m3u8?t=abc"}}}}"#);
    let web = Web::new(vec![(format!("{API}/live/resolve.json"), body(token))]);
    let mut engine = web.engine(&live);
    let channel = &engine.catalogue().unwrap()[0];
    assert_eq!(engine.resolve(&Candidate { fields: channel.fields.clone() }, channel).unwrap().url.query(), Some("t=abc"));
}

fn rot47(text: &str) -> String {
    text.chars().map(|c| if ('!'..='~').contains(&c) { char::from(b'!' + (c as u8 - b'!' + 47) % 94) } else { c }).collect()
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value > 127 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn bytes_field(number: u64, value: &[u8]) -> Vec<u8> {
    [varint(number << 3 | 2), varint(value.len() as u64), value.to_vec()].concat()
}

fn number_field(number: u64, value: u64) -> Vec<u8> {
    [varint(number << 3), varint(value)].concat()
}

fn ctr_token(key: &[u8], iv: &[u8], session: &str) -> Vec<u8> {
    let mut data = session.as_bytes().to_vec();
    ctr::Ctr128BE::<aes::Aes256>::new_from_slices(key, iv).unwrap().apply_keystream(&mut data);
    data
}

#[test]
fn a_binary_session_puts_its_token_in_the_path() {
    let source = example("lab-session");
    let (key, iv) = ([3u8; 32], [4u8; 16]);
    let url = rot47(&format!("lab-v1::{MEDIA}/session/master.m3u8"));
    let message = [bytes_field(1, url.as_bytes()), bytes_field(2, &key), bytes_field(3, &iv)].concat();
    let web = Web::new(vec![(format!("{API}/session.pb"), Reply::Body(headers(&[("X-Lab-Session", "s3cr3t")]), message))]);
    let mut engine = web.engine(&source);
    let session = &engine.catalogue().unwrap()[0];
    let resolved = engine.resolve(&Candidate { fields: session.fields.clone() }, session).unwrap();
    let token = URL_SAFE_NO_PAD.encode(ctr_token(&key, &iv, "s3cr3t"));
    assert_eq!(resolved.url.as_str(), format!("{MEDIA}/token-{token}/session/master.m3u8"));
}

/// The lab's signed binary catalogue (dev/lab/binary.py), for two
/// versions, and the session token the media path must carry.
fn binary_web() -> (Web, String) {
    const KEY: &str = "lab-binary-key-0123456789abcdefg";
    const IV: &str = "lab-binary-iv-01";
    let envelope = |inner: Vec<u8>| bytes_field(10, &inner);
    let signed = |kind: &str, version: &str, query: &str| {
        let prefix: String = md5::Md5::digest(query.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>()[..6].to_string();
        format!("{API}/bin/v{prefix}{version}/api/match/{kind}?{query}")
    };
    let config = serde_json::json!({"common:web:client": serde_json::json!({"trd": {"iframePlayerDomains": ["127.0.0.1:8782"]}}).to_string()});
    let versions = [bytes_field(1, &[number_field(1, 100), bytes_field(2, b"a1")].concat()), bytes_field(1, &[number_field(1, 102), bytes_field(2, b"b2")].concat())].concat();
    let matches = [bytes_field(1, &[number_field(1, 7001), bytes_field(19, b"Synthetic derby")].concat()), bytes_field(1, &[number_field(1, 7002), bytes_field(19, b"Empty")].concat())].concat();
    let streams = [bytes_field(2, &[number_field(1, 501), bytes_field(3, b"Feed"), number_field(9, 2001)].concat())].concat();
    let url = rot47(&format!("lab-v1::{MEDIA}/session/master.m3u8"));
    let stream = bytes_field(2, &[number_field(1, 501), bytes_field(4, url.as_bytes())].concat());
    let detail = "matchId=7001&streamId=501&siteType=2001&continent=EU&country=IT";
    let web = Web::new(vec![
        (format!("{API}/bin/api/common/params"), body(rot47(&config.to_string()))),
        (format!("{API}/bin/api/common/bs?code=100"), body(envelope(versions.clone()))),
        (format!("{API}/bin/api/common/bs?code=102"), body(envelope(versions))),
        (signed("live", "a1", "language=8&sportType=1&stream=true"), body(envelope(matches))),
        (signed("detail", "b2", "matchId=7001&language=8&sportType=1&stream=true"), body(envelope(streams))),
        (signed("detail", "b2", "matchId=7002&language=8&sportType=1&stream=true"), body(envelope(Vec::new()))),
        (format!("{API}/bin/api/user/info"), body(envelope([bytes_field(2, b"IT"), bytes_field(3, b"EU")].concat()))),
        (format!("{API}/bin/api/stream/detail?{detail}"), Reply::Body(headers(&[("X-Lab-Session", "abc123")]), envelope(stream))),
    ]);
    (web, STANDARD.encode(ctr_token(KEY.as_bytes(), IV.as_bytes(), "abc123")))
}

#[test]
fn a_signed_binary_catalogue_runs_through_recipes_and_lists_of_messages() {
    let source = example("lab-binary");
    let (web, token) = binary_web();
    let mut engine = web.engine(&source);
    let events = engine.catalogue().unwrap();
    assert_eq!(events.iter().map(|e| (e.fields["id"].as_str(), e.fields["title"].as_str())).collect::<Vec<_>>(), [("7001", "Synthetic derby"), ("7002", "Empty")]);
    assert_eq!(events[0].fields["origin"], PLAYER);
    let signed = web.asked.borrow().iter().skip(1).map(|r| r.headers.clone()).collect::<Vec<_>>();
    assert!(signed.iter().all(|h| h.contains(&("Origin".into(), PLAYER.into()))), "{signed:?}");
    let streams = engine.sources(&events[0]).unwrap();
    assert_eq!((streams[0].fields["id"].as_str(), streams[0].fields["site"].as_str()), ("501", "2001"));
    assert_eq!(engine.sources(&events[1]).unwrap_err().outcome, Outcome::NoSources);
    let resolved = engine.resolve(&streams[0], &events[0]).unwrap();
    let token: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    assert_eq!(resolved.url.as_str(), format!("{MEDIA}/token-{token}a/session/master.m3u8"));
    assert_eq!(resolved.headers, media_headers());
}

fn small(text: &str) -> Source {
    Source::from_toml(&format!("id = \"t\"\nname = \"T\"\n{text}")).unwrap()
}

#[test]
fn refusals_and_limits_have_their_outcomes() {
    let refused = small("[[resolve.steps]]\nid = \"a\"\nop = \"http\"\nurl = \"http://127.0.0.1/a\"\n[[resolve.steps]]\nid = \"s\"\nop = \"stream\"\nurl = \"{a.text}\"\n");
    for (code, outcome) in [(403, Outcome::AccessDenied), (410, Outcome::AccessDenied), (429, Outcome::RateLimited), (503, Outcome::NetworkError)] {
        let web = Web::new(vec![("http://127.0.0.1/a".into(), Reply::Status(code))]);
        let failure = web.engine(&refused).resolve(&candidate(&[("id", "x")]), &item(&[])).unwrap_err();
        assert_eq!((failure.outcome, failure.at.as_str()), (outcome, "resolve.a"), "{code}");
    }
    let unrouted = small("[[resolve.routes]]\nwhen = { host = \"a.example\" }\nuse = \"stream\"\n");
    let web = Web::new(vec![]);
    let failure = web.engine(&unrouted).resolve(&candidate(&[("id", "x"), ("url", "http://b.example/x")]), &item(&[])).unwrap_err();
    assert_eq!((failure.outcome, failure.cause), (Outcome::UnsupportedHost, Cause::Problem("b.example".into())));
    let looping = small("[[resolve.steps]]\nid = \"n\"\nop = \"next\"\ncandidate = { id = \"x\", url = \"{input.candidate.url}\" }\n");
    let failure = web.engine(&looping).resolve(&candidate(&[("id", "x"), ("url", "http://a.example/")]), &item(&[])).unwrap_err();
    assert_eq!(failure.outcome, Outcome::Limit);
    let many = small(
        "[[catalogue.steps]]\nid = \"urls\"\nop = \"http\"\nurl = \"http://a.example/list\"\n\
         [[catalogue.steps]]\nid = \"pages\"\nop = \"list.map\"\nlist = \"{urls.json}\"\ncall = \"get\"\ninput = { url = \"{item}\" }\n\
         [[resolve.steps]]\nid = \"s\"\nop = \"stream\"\nurl = \"http://a.example/\"\n\
         [recipes.get]\ninputs = [\"url\"]\n[[recipes.get.steps]]\nid = \"a\"\nop = \"http\"\nurl = \"{input.url}\"\n",
    );
    let urls: Vec<String> = (0..20).map(|n| format!("http://a.example/{n}")).collect();
    let mut pages: Vec<_> = urls.iter().map(|url| (url.clone(), body("x"))).collect();
    pages.push(("http://a.example/list".into(), body(serde_json::to_string(&urls).unwrap())));
    let web = Web::new(pages);
    let failure = web.engine(&many).catalogue().unwrap_err();
    assert_eq!((failure.outcome, failure.at.as_str()), (Outcome::Limit, "recipes.get.a"));
    assert_eq!(web.asked.borrow().len(), MAX_REQUESTS);
}

#[test]
fn a_cancelled_run_stops_before_its_next_step() {
    let source = example("lab-live");
    let web = Web::new(vec![]);
    let mut engine = Engine::new(&source, "d", web.fetch(), || true);
    let failure = engine.resolve(&candidate(&[("id", "live")]), &item(&[])).unwrap_err();
    assert_eq!(failure.outcome, Outcome::Cancelled);
    assert!(web.asked.borrow().is_empty());
}

#[test]
fn stream_headers_are_filled_and_patterns_are_not_templates() {
    let source = small(
        "[[resolve.steps]]\nid = \"origin\"\nop = \"text.match\"\nfrom = \"{input.candidate.url}\"\npattern = '^(https?://[^/]+)'\n\
         [[resolve.steps]]\nid = \"six\"\nop = \"text.match\"\nfrom = \"{input.candidate.id}\"\npattern = '^(.{6})'\n\
         [[resolve.steps]]\nid = \"s\"\nop = \"stream\"\nurl = \"{input.candidate.url}\"\nheaders = { Referer = \"{origin}/\", X-Id = \"{six}\" }\n",
    );
    let web = Web::new(vec![]);
    let resolved = web.engine(&source).resolve(&candidate(&[("id", "abcdefgh"), ("url", "https://p.example/live?u=1")]), &item(&[])).unwrap();
    assert_eq!(resolved.headers, headers(&[("Referer", "https://p.example/"), ("X-Id", "abcdef")]));
}

#[test]
fn values_convert_as_the_contract_says() {
    let source = small(
        "[[resolve.steps]]\nid = \"b\"\nop = \"base64.decode\"\nfrom = \"aGk\"\nalphabet = \"url\"\n\
         [[resolve.steps]]\nid = \"h\"\nop = \"hex.encode\"\nfrom = \"{b}\"\n\
         [[resolve.steps]]\nid = \"m\"\nop = \"md5\"\nfrom = \"hi\"\n\
         [[resolve.steps]]\nid = \"r\"\nop = \"rot47\"\nfrom = \"lab\"\n\
         [[resolve.steps]]\nid = \"q\"\nop = \"url.query_after\"\nfrom = \"https://p.example/?a=1&url=https://m.example/x?t=1&txTime=2\"\nname = \"url\"\n\
         [[resolve.steps]]\nid = \"u\"\nop = \"url.set\"\nfrom = \"https://p.example/?a=1&b=2\"\nname = \"a\"\nvalue = \"3\"\n\
         [[resolve.steps]]\nid = \"d\"\nop = \"date\"\nfrom = \"23/11/2026 20:30\"\nformat = \"%d/%m/%Y %H:%M\"\nzone = \"+08:00\"\n\
         [[resolve.steps]]\nid = \"s\"\nop = \"stream\"\nurl = \"{q}\"\nheaders = { h = \"{h}\", m = \"{m}\", r = \"{r}\", u = \"{u}\", c = \"{r|component}\", d = \"{d}\" }\n",
    );
    let resolved = Web::new(vec![]).engine(&source).resolve(&candidate(&[("id", "x")]), &item(&[])).unwrap();
    assert_eq!(resolved.url.as_str(), "https://m.example/x?t=1&txTime=2");
    let got: BTreeMap<_, _> = resolved.headers.into_iter().collect();
    assert_eq!(got["h"], "6869");
    assert_eq!(got["m"], "49f68a5c8493ec2c0bf489821c21fc3b");
    assert_eq!(got["r"], "=23");
    assert_eq!(got["u"], "https://p.example/?b=2&a=3");
    assert_eq!(got["c"], "%3D23");
    assert_eq!(got["d"], "2026-11-23T12:30:00Z");
}
