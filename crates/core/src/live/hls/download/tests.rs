use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::playback::Input;

/// Serves `answer(path)` on a local port, recording the paths asked.
fn serve(answer: fn(&str) -> Vec<u8>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let body = answer(&path);
            log.lock().unwrap().push(path);
            let mut stream = reader.into_inner();
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body));
        }
    });
    (address, asked)
}

#[test]
fn a_slow_variant_gives_way_to_a_lighter_one_at_the_same_time_in_a_new_section() {
    // The best variant, light enough to start on, asks for far more than
    // its first segment's download rate: the next segment comes from the
    // lighter one, the one after the same time, however each numbers its
    // segments.
    // `/master-H-L[-d]`: high numbered from H, low from L, with a
    // discontinuity before their second segment when `-d`.
    let (address, asked) = serve(|path| {
        let Some(name) = path.strip_prefix('/').and_then(|p| p.strip_suffix(".m3u8")) else {
            return path.as_bytes()[1..].to_vec(); // a segment
        };
        let mut parts = name.split('-');
        match parts.next().unwrap() {
            "master" => {
                let (high, low, rest) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap_or(""));
                format!(
                    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nlow-{low}-{rest}.m3u8\n\
                     #EXT-X-STREAM-INF:BANDWIDTH=2000000\nhigh-{high}-{rest}.m3u8\n"
                )
                .into_bytes()
            }
            variant @ ("low" | "high") => {
                let first = parts.next().unwrap();
                let discontinuity = if parts.next() == Some("d") { "#EXT-X-DISCONTINUITY\n" } else { "" };
                let segment = |i| format!("#EXTINF:2,\n{variant}-{i}.ts\n");
                let segments = format!("{}{discontinuity}{}{}", segment(0), segment(1), segment(2));
                format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n{segments}#EXT-X-ENDLIST\n").into_bytes()
            }
            other => panic!("playlist {other}"),
        }
    });
    for (numbering, discontinuity) in [("0-0", 0), ("0-500", 0), ("500-0", 0), ("0-500-d", 1)] {
        asked.lock().unwrap().clear();
        let viewing = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let source = Source::direct(format!("{address}/master-{numbering}.m3u8"), Vec::new());
        let live = Live::resolve(&source, &Stop::new(&viewing), stats, &QualityChoice::default()).unwrap().unwrap();
        let mut stream = live.open(&Stop::new(&viewing)).unwrap().main;
        let mut first = String::new();
        stream.read_to_string(&mut first).unwrap();
        assert_eq!(first, "high-0.ts");
        assert!(stream.next_section());
        assert_eq!(stream.discontinuity_sequence(), discontinuity, "{numbering}: the timestamps go on");
        let mut rest = String::new();
        stream.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "low-1.tslow-2.ts", "{numbering}");
        assert!(!stream.next_section());
        let asked: Vec<String> = asked.lock().unwrap().iter().filter(|p| p.ends_with(".ts")).cloned().collect();
        assert_eq!(asked, ["/high-0.ts", "/low-1.ts", "/low-2.ts"], "{numbering}");
    }
}

#[test]
fn a_playlist_is_never_a_continuous_stream() {
    // A `G` one packet apart, as GROUP-ID may put it.
    let mut text = b"\xef\xbb\xbf\n#EXTM3U\n".to_vec();
    text.resize(ts::HEAD, b' ');
    for start in (20..ts::HEAD).step_by(ts::PACKET) {
        text[start] = b'G';
    }
    assert!(ts::is_transport_stream(&text));
    assert!(!is_continuous(&text));
}
