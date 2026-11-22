use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Instant;

use image::{ImageFormat, RgbaImage};

use super::*;

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut data = Cursor::new(Vec::new());
    RgbaImage::new(w, h).write_to(&mut data, ImageFormat::Png).unwrap();
    data.into_inner()
}

/// A server answering every request with `body`; the second value
/// counts the requests.
fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = requests.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (body, counted) = (body.clone(), counted.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut stream = stream;
                loop {
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                        line.clear();
                    }
                    if line != "\r\n" {
                        return;
                    }
                    counted.fetch_add(1, Ordering::SeqCst);
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                    if stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&body)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (address, requests)
}

/// Receives until nothing is loading any more.
fn settle(logos: &mut Logos) -> Vec<(usize, Decoded)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = Vec::new();
    while logos.loading() {
        assert!(Instant::now() < deadline, "logos still loading");
        ready.extend(logos.receive());
        std::thread::sleep(Duration::from_millis(5));
    }
    ready
}

#[test]
fn a_logo_shared_by_several_channels_is_fetched_once_into_one_slot() {
    let (address, requests) = serve(png(200, 100));
    let url = format!("{address}/shared.png");
    let mut logos = Logos::new(8);
    for _ in 0..3 {
        logos.want(std::iter::repeat_n(url.as_str(), 5));
        std::thread::sleep(Duration::from_millis(20));
    }
    let ready = settle(&mut logos);
    logos.want(std::iter::repeat_n(url.as_str(), 5));
    assert!(!logos.loading());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(ready.len(), 1);
    assert_eq!((ready[0].1.w, ready[0].1.h), (128, 64));
    assert_eq!((logos.resident(), logos.free.len()), (1, 7));
}

#[test]
fn a_logo_too_large_to_decode_fails_and_the_next_one_loads() {
    let huge = png(MAX_LOGO_SIDE + 1, 1);
    assert!(huge.len() as u64 <= MAX_LOGO_BYTES);
    let (huge, _) = serve(huge);
    let (small, _) = serve(png(64, 64));
    let mut logos = Logos::new(4);
    let urls = [format!("{huge}/huge.png"), format!("{small}/small.png")];
    logos.want(urls.iter().map(String::as_str));
    let ready = settle(&mut logos);
    assert_eq!((logos.failed, logos.loaded, ready.len()), (1, 1, 1));
    assert!(matches!(logos.states[&urls[0]], State::Failed));
}
