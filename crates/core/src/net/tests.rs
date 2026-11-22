use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Instant;

use super::*;

#[test]
fn a_small_resource_that_trickles_is_given_up() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/logo.png", listener.local_addr().unwrap())).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            line.clear();
        }
        let mut stream = reader.into_inner();
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
        // One byte every 100 ms: each read is quick, the whole is not.
        for _ in 0..100 {
            if stream.write_all(b"x").is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    assert!(Client::new().unwrap().get_small(&url, 1000, Duration::from_millis(500)).is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn answers_are_asked_compressed_and_unpacked() {
    const PLAYLIST: &[u8] = b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n";
    const GZIPPED: [u8; 50] = [
        31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 83, 118, 141, 8, 241, 53, 14, 229, 82, 6, 210, 186, 17, 186, 33, 142,
        65, 238, 174, 33, 46, 161, 65, 142, 33, 158, 254, 126, 86, 102, 92, 0, 41, 214, 206, 174, 32, 0, 0, 0,
    ];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/chunklist.m3u8", listener.local_addr().unwrap())).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut accepts_gzip = false;
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
            let (name, value) = line.split_once(':').unwrap_or_default();
            accepts_gzip |= name.eq_ignore_ascii_case("accept-encoding") && value.contains("gzip");
            line.clear();
        }
        let mut stream = reader.into_inner();
        let head = format!("HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\r\n", GZIPPED.len());
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&GZIPPED).unwrap();
        accepts_gzip
    });
    assert_eq!(Client::new().unwrap().get(&url, 1024).unwrap(), PLAYLIST);
    assert!(server.join().unwrap(), "la richiesta non accetta gzip");
}

/// Answers every request on a local port with `body`, recording the
/// headers of each.
fn serve(body: &'static [u8]) -> (Url, Arc<std::sync::Mutex<Vec<Vec<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = Url::parse(&format!("http://{}/a", listener.local_addr().unwrap())).unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let (mut line, mut headers) = (String::new(), Vec::new());
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                headers.push(line.trim().to_ascii_lowercase());
                line.clear();
            }
            recorded.lock().unwrap().push(headers);
            let mut stream = reader.into_inner();
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body));
        }
    });
    (url, requests)
}

#[test]
fn an_answer_longer_than_its_bound_is_refused_not_cut() {
    let (url, _) = serve(b"0123456789");
    let client = Client::new().unwrap();
    assert_eq!(client.get(&url, 10).unwrap(), b"0123456789");
    let error = client.get(&url, 9).unwrap_err();
    assert!(error.is_oversized() && !error.is_refused(), "{error}");
    assert!(client.get_small(&url, 9, Duration::from_secs(5)).unwrap_err().is_oversized());
}

#[test]
fn credentials_go_only_to_the_origins_the_source_names() {
    let (named, at_named) = serve(b"");
    let (other, at_other) = serve(b"");
    let headers = vec![
        ("User-Agent".to_string(), "VLC/3.0".to_string()),
        ("Referer".to_string(), "https://lab.invalid/".to_string()),
        ("Cookie".to_string(), "session=finta".to_string()),
        ("authorization".to_string(), "Bearer finto".to_string()),
    ];
    let client = Client::for_source(headers, std::slice::from_ref(&named), None).unwrap();
    client.get(&named.join("/playlist.m3u8").unwrap(), 10).unwrap();
    client.get(&other, 10).unwrap();
    let has = |requests: &Arc<std::sync::Mutex<Vec<Vec<String>>>>, prefix: &str| {
        requests.lock().unwrap()[0].iter().any(|h| h.starts_with(prefix))
    };
    for prefix in ["user-agent: vlc/3.0", "referer:", "cookie: session=finta", "authorization: bearer finto"] {
        assert!(has(&at_named, prefix), "{prefix}");
    }
    assert!(has(&at_other, "user-agent: vlc/3.0") && has(&at_other, "referer:"));
    assert!(!has(&at_other, "cookie:") && !has(&at_other, "authorization:"));
}

#[test]
fn logged_paths_leave_out_tokens() {
    let url = Url::parse("https://cdn.invalid/hls/live/rai1_2400/exp=1~acl=%2f*~data=hdntl~hmac=ab/chunklist.m3u8?s=1").unwrap();
    assert_eq!(loggable_path(&url), "/hls/live/rai1_2400/…/chunklist.m3u8");
    assert_eq!(loggable(&url), "cdn.invalid/hls/live/rai1_2400/…/chunklist.m3u8");
}
