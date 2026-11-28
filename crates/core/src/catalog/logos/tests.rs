use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Instant;

use std::sync::atomic::{AtomicUsize, Ordering};

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

/// An empty data directory of its own.
fn data_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mana-logos-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Takes until nothing is loading any more.
fn settle(loader: &mut Loader) -> Vec<(String, Result<Logo, String>)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut taken = Vec::new();
    while loader.loading() {
        assert!(Instant::now() < deadline, "logos still loading");
        taken.extend(std::iter::from_fn(|| loader.take()));
        std::thread::sleep(Duration::from_millis(5));
    }
    taken
}

#[test]
fn a_logo_shared_by_several_channels_is_loaded_once() {
    let (address, requests) = serve(png(200, 100));
    let url = format!("{address}/shared.png");
    let mut loader = Loader::start(&data_dir("shared"));
    for _ in 0..3 {
        loader.want(std::iter::repeat_n(url.as_str(), 5));
        std::thread::sleep(Duration::from_millis(20));
    }
    let taken = settle(&mut loader);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(taken.len(), 1);
    let logo = taken[0].1.as_ref().unwrap();
    assert_eq!((logo.w, logo.h, logo.rgba.len()), (128, 64, 128 * 64 * 4));
}

#[test]
fn a_logo_too_large_to_decode_fails_and_the_next_one_loads() {
    let huge = png(MAX_LOGO_SIDE + 1, 1);
    assert!(huge.len() as u64 <= MAX_LOGO_BYTES);
    let (huge, _) = serve(huge);
    let (small, _) = serve(png(64, 64));
    let mut loader = Loader::start(&data_dir("huge"));
    let urls = [format!("{huge}/huge.png"), format!("{small}/small.png")];
    loader.want(urls.iter().map(String::as_str));
    let taken = settle(&mut loader);
    assert_eq!((loader.failed, loader.loaded, taken.len()), (1, 1, 2));
    assert!(taken.iter().any(|(url, result)| *url == urls[0] && result.is_err()));
}

#[test]
fn a_logo_kept_on_disk_loads_at_the_next_start_without_asking_its_server() {
    let (address, requests) = serve(png(200, 100));
    let url = format!("{address}/kept.png");
    let dir = data_dir("kept");
    for _ in 0..2 {
        let mut loader = Loader::start(&dir);
        loader.want(std::iter::once(url.as_str()));
        let taken = settle(&mut loader);
        let logo = taken[0].1.as_ref().unwrap();
        assert_eq!((logo.w, logo.h), (128, 64));
    }
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test]
fn a_cache_grown_past_its_size_is_emptied_at_start() {
    let dir = data_dir("full");
    let kept = dir.join(CACHE);
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("old.png"), vec![0; MAX_CACHE_BYTES as usize + 1]).unwrap();
    Loader::start(&dir);
    assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 0);
}
