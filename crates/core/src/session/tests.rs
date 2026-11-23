use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use super::*;
use crate::playback::fake;

/// A request the test server answered: its path and its header lines,
/// lowercase.
struct Asked {
    path: String,
    headers: Vec<String>,
}

/// Serves `answer(path)` as (status, body) on a local port, the body of
/// a 307 as its Location; records each request.
fn serve(answer: fn(&str) -> (u16, Vec<u8>)) -> (String, Arc<Mutex<Vec<Asked>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (mut line, mut headers) = (String::new(), Vec::new());
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                headers.push(line.trim().to_lowercase());
                line.clear();
            }
            log.lock().unwrap().push(Asked { path: path.clone(), headers });
            let (status, body) = answer(&path);
            let mut stream = reader.into_inner();
            let head = if status == 307 {
                format!("HTTP/1.1 307 X\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", String::from_utf8_lossy(&body))
            } else {
                format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
            };
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
        }
    });
    (address, asked)
}

/// A `.stream` file pointing at `url`, in a directory of its own.
fn stream_file(name: &str, url: &str) -> Program {
    let dir = std::env::temp_dir().join(format!("mana-session-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.stream"));
    std::fs::write(&path, format!("url = {url}\n")).unwrap();
    Program::Path(path)
}

/// Waits for the viewing to end and returns its final state, as the log
/// reads it.
fn ended(session: &Session) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = session.stats().unwrap().state;
        if state.ended() {
            return state.to_string();
        }
        assert!(Instant::now() < deadline, "the viewing does not end: {state}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stopping_within_a_limit_does_not_wait_for_a_request_that_hangs() {
    // Accepts the connection, never answers: the resolution blocks in it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let held: Vec<_> = listener.incoming().take(1).collect();
        std::thread::sleep(Duration::from_secs(30));
        drop(held);
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("hanging".into(), stream_file("hanging", &format!("{address}/master.m3u8")), Settings::default()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let stopping = Instant::now();
    assert!(!session.stop_within(Duration::from_millis(200)));
    assert!(stopping.elapsed() < Duration::from_secs(1));
    assert!(!session.is_playing());

    let (open, _) = fake::open();
    let mut idle = Session::new(open);
    assert!(idle.stop_within(Duration::ZERO));
}

#[test]
fn changing_channel_returns_at_once_while_a_request_is_pending() {
    // Answers nothing for 750 ms, then closes: the resolutions block in it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(750));
                drop(stream);
            });
        }
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    let pressed = Instant::now();
    for channel in ["uno", "due", "tre"] {
        session.play(channel.into(), stream_file(channel, &format!("{address}/{channel}.m3u8")), Settings::default()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    session.stop();
    assert!(pressed.elapsed() < Duration::from_millis(400), "{:?}", pressed.elapsed());
    assert!(!session.is_playing() && session.is_stopping());
    // Each ends once its request does.
    let deadline = Instant::now() + Duration::from_secs(5);
    while session.is_stopping() {
        assert!(Instant::now() < deadline, "the stopped viewings do not end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn backends_open_in_turn_and_a_viewing_stopped_meanwhile_opens_none() {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let counting = |name: &'static str| -> OpenBackend {
        let opened = opened.clone();
        let (open, _) = fake::open();
        Arc::new(move || {
            opened.lock().unwrap().push(name);
            open()
        })
    };
    let turn = Arc::new(Turn::default());
    let viewing = |name, stop: &Arc<AtomicBool>| {
        let in_turn = InTurn::new(turn.clone());
        let open = in_turn.opener(counting(name), stop);
        (in_turn, open)
    };
    let stops: Vec<Arc<AtomicBool>> = (0..3).map(|_| Arc::new(AtomicBool::new(false))).collect();
    let (first, open_first) = viewing("first", &stops[0]);
    let backend = open_first().unwrap();
    assert!(first.holds.load(Ordering::Relaxed));
    // Two channels change while the first still has its backend.
    let waiting: Vec<_> = [("second", &stops[1]), ("third", &stops[2])]
        .into_iter()
        .map(|(name, stop)| {
            let (in_turn, open) = viewing(name, stop);
            std::thread::spawn(move || {
                let opened = open().map(drop);
                drop(in_turn);
                opened
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(*opened.lock().unwrap(), ["first"]);
    stops[0].store(true, Ordering::Relaxed);
    stops[1].store(true, Ordering::Relaxed);
    // The first viewing's thread ends: its turn goes back.
    drop(backend);
    drop(first);
    let results: Vec<_> = waiting.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results[0].is_err() && results[1].is_ok());
    assert_eq!(*opened.lock().unwrap(), ["first", "third"]);
}

const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nmedia.m3u8\n";

#[test]
fn a_refused_token_is_resolved_again_within_the_budget() {
    let (address, asked) = serve(|path| match path {
        "/master.m3u8" => (200, MASTER.into()),
        _ => (403, Vec::new()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("refused".into(), stream_file("refused", &format!("{address}/master.m3u8")), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.contains(&format!("source refused even after {} new resolutions", reopen::MAX_REOPENINGS)), "{state}");
    let masters = asked.lock().unwrap().iter().filter(|a| a.path == "/master.m3u8").count();
    assert_eq!(masters, 1 + reopen::MAX_REOPENINGS);
    session.stop();
    assert!(!session.is_playing());
}

#[test]
fn a_stream_file_with_a_recipe_is_refused_naming_its_key() {
    let dir = std::env::temp_dir().join(format!("mana-session-recipe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("recipe.stream");
    std::fs::write(&path, "resolve = get https://a.invalid/token\n").unwrap();
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("recipe".into(), Program::Path(path), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.ends_with("recipe.stream: resolve"), "{state}");
}

#[test]
fn a_direct_url_refused_at_the_opening_ends_with_its_status() {
    let (address, asked) = serve(|_| (403, Vec::new()));
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("denied".into(), stream_file("denied", &format!("{address}/master.m3u8")), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.contains("HTTP 403 for /master.m3u8"), "{state}");
    assert!(state.contains(&format!("even after {} new resolutions", reopen::MAX_REOPENINGS)), "{state}");
    assert_eq!(asked.lock().unwrap().len(), 1 + reopen::MAX_REOPENINGS);
}

#[test]
fn a_media_playlist_given_as_the_stream_is_asked_for_once() {
    let (address, asked) = serve(|path| match path {
        "/media.m3u8" => (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec()),
        _ => (200, packed_audio_segment()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("media".into(), stream_file("media", &format!("{address}/media.m3u8")), Settings::default()).unwrap();
    ended(&session);
    let paths: Vec<String> = asked.lock().unwrap().iter().map(|a| a.path.clone()).collect();
    assert_eq!(paths, ["/media.m3u8", "/audio.aac"]);
}

#[test]
fn a_stream_without_video_ends_without_reopening() {
    let (address, asked) = serve(|path| match path {
        "/master.m3u8" => (200, MASTER.into()),
        "/media.m3u8" => (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec()),
        _ => (200, packed_audio_segment()),
    });
    let (open, fed) = fake::open();
    let mut session = Session::new(open);
    session.play("audio".into(), stream_file("audio", &format!("{address}/master.m3u8")), Settings::default()).unwrap();
    let state = ended(&session);
    assert_eq!(state, "Error: the stream has no video");
    assert_eq!(asked.lock().unwrap().iter().filter(|a| a.path == "/media.m3u8").count(), 1);
    assert!(fed.lock().unwrap().loads.is_empty());
}

#[test]
fn a_channel_s_headers_go_with_every_request_of_its_stream() {
    let (address, asked) = serve(|path| match path {
        "/master.m3u8" => (200, MASTER.into()),
        "/media.m3u8" => (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec()),
        _ => (200, packed_audio_segment()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    let headers = vec![("User-Agent".into(), "HbbTV/1.6.1".into()), ("Referer".into(), "https://b.invalid/".into())];
    let program = Program::Live(Playable::direct(format!("{address}/master.m3u8").parse().unwrap(), headers));
    session.play("Canale".into(), program, Settings::default()).unwrap();
    ended(&session);
    let asked = asked.lock().unwrap();
    let paths: Vec<&str> = asked.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, ["/master.m3u8", "/media.m3u8", "/audio.aac"]);
    for request in asked.iter() {
        assert!(request.headers.contains(&"user-agent: hbbtv/1.6.1".into()), "{}: {:?}", request.path, request.headers);
        assert!(request.headers.contains(&"referer: https://b.invalid/".into()), "{}: {:?}", request.path, request.headers);
        // The channel's User-Agent replaces the app's, not joins it.
        assert_eq!(request.headers.iter().filter(|h| h.starts_with("user-agent:")).count(), 1);
    }
}

/// An item of the source `text`, its `{ADDRESS}` the test server's.
fn source_item(text: &str, address: &str, fields: &[(&str, &str)]) -> Program {
    let source = crate::source::Source::from_toml(&text.replace("{ADDRESS}", address)).unwrap();
    let fields = fields.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let item = crate::source::Item { fields };
    Program::Source(Selection { source: Arc::new(source), item, device: "d-1".into(), candidate: None })
}

/// As `source_item`, the candidate `id` chosen.
fn chosen(text: &str, address: &str, id: &str) -> Program {
    let Program::Source(mut selection) = source_item(text, address, &[("id", "e1")]) else { unreachable!() };
    selection.candidate = Some(crate::source::run::Candidate { fields: [("id".to_string(), id.to_string())].into() });
    Program::Source(selection)
}

const MEDIA: &[u8] = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n";

#[test]
fn a_channel_s_source_resolves_its_item_with_the_device_and_its_headers() {
    // As Pluto's: the boot answer gives the host, the query and the token
    // the master URL is composed of.
    static ADDRESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let (address, asked) = serve(|path| match path {
        "/v4/start?clientID=d-1&channelID=c7" => {
            let host = ADDRESS.get().unwrap();
            (200, format!(r#"{{"servers":{{"stitcher":"{host}"}},"stitcherParams":"a=1%202","sessionToken":"t"}}"#).into_bytes())
        }
        "/stitch/c7/master.m3u8?a=1%202&jwt=t" => (200, MASTER.into()),
        "/stitch/c7/media.m3u8" => (200, MEDIA.to_vec()),
        "/stitch/c7/audio.aac" => (200, packed_audio_segment()),
        _ => (404, Vec::new()),
    });
    ADDRESS.set(address.clone()).unwrap();
    let source = r#"
id = "pluto"
name = "Pluto"
[headers.app]
User-Agent = "Mana-test"
[[resolve.steps]]
id = "boot"
op = "http"
url = "{ADDRESS}/v4/start?clientID={input.device}&channelID={input.item.id}"
headers = "app"
[[resolve.steps]]
id = "stream"
op = "stream"
url = "{boot.json.servers.stitcher}/stitch/{input.item.id}/master.m3u8?{boot.json.stitcherParams}&jwt={boot.json.sessionToken}"
headers = "app"
"#;
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("Pluto".into(), source_item(source, &address, &[("id", "c7"), ("title", "Pluto")]), Settings::default()).unwrap();
    let state = ended(&session);
    assert_eq!(state, "Error: the stream has no video");
    let asked = asked.lock().unwrap();
    let paths: Vec<&str> = asked.iter().map(|a| a.path.as_str()).collect();
    let master = "/stitch/c7/master.m3u8?a=1%202&jwt=t";
    assert_eq!(paths, ["/v4/start?clientID=d-1&channelID=c7", master, "/stitch/c7/media.m3u8", "/stitch/c7/audio.aac"]);
    assert!(asked.iter().all(|a| a.headers.contains(&"user-agent: mana-test".into())));
}

#[test]
fn a_source_answer_over_the_limit_ends_without_retrying() {
    let (address, asked) = serve(|_| (200, vec![b' '; crate::net::MAX_TEXT_BYTES as usize + 1]));
    let source = "id = \"big\"\nname = \"Big\"\n[[resolve.steps]]\nid = \"token\"\nop = \"http\"\nurl = \"{ADDRESS}/token\"\n\
                  [[resolve.steps]]\nid = \"stream\"\nop = \"stream\"\nurl = \"{token.text}\"\n";
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("big".into(), source_item(source, &address, &[("id", "x")]), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.contains("/token: answer over"), "{state}");
    assert_eq!(asked.lock().unwrap().len(), 1);
}

/// Three signals: one the API refuses to resolve, one whose media is
/// refused, one that plays.
const SIGNALS: &str = r#"
id = "events"
name = "Events"
[[sources.steps]]
id = "detail"
op = "http"
url = "{ADDRESS}/event.json"
[[sources.steps]]
id = "signals"
op = "list.map"
list = "{detail.json}"
item = { id = "{item}" }
[[resolve.steps]]
id = "link"
op = "http"
url = "{ADDRESS}/link/{input.candidate.id}"
[[resolve.steps]]
id = "stream"
op = "stream"
url = "{link.text}"
"#;

/// Answers for SIGNALS: one the API refuses to resolve, one whose host
/// refuses connections, one that plays; `hosts` are the URL of a port
/// nothing listens on and the test server's address.
fn signals(path: &str, hosts: &(String, String)) -> (u16, Vec<u8>) {
    match path {
        "/event.json" => (200, br#"["closed", "unreachable", "open"]"#.to_vec()),
        "/link/closed" => (403, Vec::new()),
        "/link/unreachable" => (200, hosts.0.clone().into_bytes()),
        "/link/open" => (200, format!("{}/master.m3u8", hosts.1).into_bytes()),
        "/master.m3u8" => (200, MASTER.into()),
        "/media.m3u8" => (200, MEDIA.to_vec()),
        "/audio.aac" => (200, packed_audio_segment()),
        _ => (404, Vec::new()),
    }
}

/// A server of `signals` for one test, `hosts` its own static.
fn serve_signals(answer: fn(&str) -> (u16, Vec<u8>), hosts: &std::sync::OnceLock<(String, String)>) -> (String, Arc<Mutex<Vec<Asked>>>) {
    let (address, asked) = serve(answer);
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    hosts.set((format!("http://{closed}/master.m3u8"), address.clone())).unwrap();
    (address, asked)
}

fn paths(asked: &Mutex<Vec<Asked>>) -> Vec<String> {
    asked.lock().unwrap().iter().map(|a| a.path.clone()).collect()
}

#[test]
fn the_candidates_of_an_item_come_on_their_own_thread_in_the_source_s_order() {
    static HOSTS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    let (address, _) = serve_signals(|path| signals(path, HOSTS.get().unwrap()), &HOSTS);
    let Program::Source(selection) = source_item(SIGNALS, &address, &[("id", "e1")]) else { unreachable!() };
    let asked = Candidates::ask(&selection);
    let deadline = Instant::now() + Duration::from_secs(5);
    let found = loop {
        if let Some(found) = asked.poll() {
            break found.unwrap();
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    let ids: Vec<&str> = found.iter().map(|c| c.fields["id"].as_str()).collect();
    assert_eq!(ids, ["closed", "unreachable", "open"]);
}

#[test]
fn a_chosen_candidate_plays_alone() {
    static HOSTS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    let (address, asked) = serve_signals(|path| signals(path, HOSTS.get().unwrap()), &HOSTS);
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("Event".into(), chosen(SIGNALS, &address, "open"), Settings::default()).unwrap();
    assert_eq!(ended(&session), "Error: the stream has no video");
    assert_eq!(paths(&asked), ["/link/open", "/master.m3u8", "/media.m3u8", "/audio.aac"]);
}

#[test]
fn a_chosen_candidate_that_fails_ends_with_its_reason_and_no_other_is_tried() {
    static HOSTS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    let (address, asked) = serve_signals(|path| signals(path, HOSTS.get().unwrap()), &HOSTS);
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("Event".into(), chosen(SIGNALS, &address, "closed"), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.contains("resolve.link: access denied"), "{state}");
    assert_eq!(paths(&asked), ["/link/closed"]);
}

#[test]
fn a_chosen_unreachable_candidate_ends_after_a_few_attempts() {
    static HOSTS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    let (address, asked) = serve_signals(|path| signals(path, HOSTS.get().unwrap()), &HOSTS);
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    let started = Instant::now();
    session.play("Event".into(), chosen(SIGNALS, &address, "unreachable"), Settings::default()).unwrap();
    let state = ended(&session);
    assert!(state.contains("/master.m3u8"), "{state}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert_eq!(paths(&asked), ["/link/unreachable"]);
}

#[test]
fn a_refused_token_has_the_source_resolve_the_candidate_again() {
    static ADDRESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static TOKENS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let (address, asked) = serve(|path| match path {
        "/event.json" => (200, br#"["one"]"#.to_vec()),
        "/link/one" => {
            let token = TOKENS.fetch_add(1, Ordering::Relaxed);
            (200, format!("{}/master.m3u8?t={token}", ADDRESS.get().unwrap()).into_bytes())
        }
        "/master.m3u8?t=0" => (403, Vec::new()),
        "/master.m3u8?t=1" => (200, MASTER.into()),
        "/media.m3u8" => (200, MEDIA.to_vec()),
        "/audio.aac" => (200, packed_audio_segment()),
        _ => (404, Vec::new()),
    });
    ADDRESS.set(address.clone()).unwrap();
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("Event".into(), source_item(SIGNALS, &address, &[("id", "e1")]), Settings::default()).unwrap();
    assert_eq!(ended(&session), "Error: the stream has no video");
    let paths: Vec<String> = asked.lock().unwrap().iter().map(|a| a.path.clone()).collect();
    assert_eq!(paths[..5], ["/event.json", "/link/one", "/master.m3u8?t=0", "/link/one", "/master.m3u8?t=1"]);
}

#[test]
fn an_item_without_candidates_ends_saying_so() {
    let (address, _) = serve(|_| (200, b"[]".to_vec()));
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("Event".into(), source_item(SIGNALS, &address, &[("id", "e1")]), Settings::default()).unwrap();
    assert_eq!(ended(&session), "Error: sources: no streams for now");
}

#[test]
fn a_header_no_request_can_carry_is_left_out_and_the_stream_plays() {
    let (address, asked) = serve(|path| match path {
        "/master.m3u8" => (200, MASTER.into()),
        "/media.m3u8" => (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec()),
        _ => (200, packed_audio_segment()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    let headers =
        vec![("User-Agent".into(), "HbbTV/1.6.1".into()), ("Referer".into(), "https://b.invalid/\r\nX-Injected: 1".into())];
    let program = Program::Live(Playable::direct(format!("{address}/master.m3u8").parse().unwrap(), headers));
    session.play("Canale".into(), program, Settings::default()).unwrap();
    // The segment arrives: the stream ends on its formats, not on requests
    // the transport refuses.
    let state = ended(&session);
    assert_eq!(state, "Error: the stream has no video");
    for request in asked.lock().unwrap().iter() {
        assert!(request.headers.contains(&"user-agent: hbbtv/1.6.1".into()), "{}: {:?}", request.path, request.headers);
        assert!(!request.headers.iter().any(|h| h.starts_with("referer:") || h.starts_with("x-injected:")), "{:?}", request.headers);
    }
}

#[test]
fn relative_urls_resolve_against_where_a_redirect_led() {
    // As netrw.it does for RaiPlay: the token lives in the redirected path.
    let (address, asked) = serve(|path| match path {
        "/live/master.m3u8" => (307, b"/@token=1/live/master.m3u8".to_vec()),
        "/@token=1/live/master.m3u8" => (200, b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n../hd/media.m3u8\n".to_vec()),
        "/@token=1/hd/media.m3u8" => (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec()),
        "/@token=1/hd/audio.aac" => (200, packed_audio_segment()),
        _ => (403, Vec::new()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("netrw".into(), stream_file("netrw", &format!("{address}/live/master.m3u8")), Settings::default()).unwrap();
    // The segment arrives: the stream ends on its formats, not on a 403.
    let state = ended(&session);
    assert_eq!(state, "Error: the stream has no video");
    assert!(asked.lock().unwrap().iter().all(|a| a.path == "/live/master.m3u8" || a.path.starts_with("/@token=1/")));
}

#[test]
fn a_segment_that_cannot_be_read_is_skipped() {
    let (address, _) = serve(|path| match path {
        "/master.m3u8" => (200, MASTER.into()),
        "/media.m3u8" => {
            (200, b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\nbad.aac\n#EXTINF:2,\naudio.aac\n#EXT-X-ENDLIST\n".to_vec())
        }
        // ADTS frames without the ID3 timestamp packed audio needs.
        "/bad.aac" => (200, packed_audio_segment().split_off(packed_audio_segment().len() - 16 * 10)),
        _ => (200, packed_audio_segment()),
    });
    let (open, _) = fake::open();
    let mut session = Session::new(open);
    session.play("skipped".into(), stream_file("skipped", &format!("{address}/master.m3u8")), Settings::default()).unwrap();
    // The good segment arrives: the stream ends on its formats, not on the bad segment.
    let state = ended(&session);
    assert_eq!(state, "Error: the stream has no video");
}

/// Two seconds of silent AAC as HLS packed audio: an ID3 tag with the
/// first timestamp, then ADTS frames (48 kHz stereo).
fn packed_audio_segment() -> Vec<u8> {
    const OWNER: &[u8] = b"com.apple.streaming.transportStreamTimestamp\0";
    let mut priv_frame = b"PRIV".to_vec();
    priv_frame.extend_from_slice(&((OWNER.len() + 8) as u32).to_be_bytes());
    priv_frame.extend_from_slice(&[0, 0]);
    priv_frame.extend_from_slice(OWNER);
    priv_frame.extend_from_slice(&90_000u64.to_be_bytes());
    let size = priv_frame.len();
    let mut segment = b"ID3\x04\x00\x00".to_vec();
    segment.extend((0..4).rev().map(|i| ((size >> (7 * i)) & 0x7F) as u8));
    segment.extend_from_slice(&priv_frame);
    // A silent AAC-LC raw data block for two channels (as hls.js has it).
    let payload = [0x21, 0x00, 0x49, 0x90, 0x02, 0x19, 0x00, 0x23, 0x80];
    let length = 7 + payload.len();
    let frame = [0xFF, 0xF1, 0x4C, 0x80 | (length >> 11) as u8, (length >> 3) as u8, ((length & 7) << 5) as u8 | 0x1F, 0xFC];
    for _ in 0..94 {
        segment.extend_from_slice(&frame);
        segment.extend_from_slice(&payload);
    }
    segment
}
