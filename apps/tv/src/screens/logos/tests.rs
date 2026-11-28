use std::time::{Duration, Instant};

use image::{ImageFormat, RgbaImage};

use super::*;

/// A PNG of `w` × `h` in a file of its own, as a fixture's local logo.
fn fixture(name: &str, w: u32, h: u32) -> String {
    let path = std::env::temp_dir().join(format!("mana-tv-logo-{}-{name}.png", std::process::id()));
    RgbaImage::new(w, h).save_with_format(&path, ImageFormat::Png).unwrap();
    path.to_string_lossy().into_owned()
}

/// Receives until nothing is loading any more.
fn settle(logos: &mut Logos) -> Vec<(usize, Logo)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = Vec::new();
    while logos.loader.loading() {
        assert!(Instant::now() < deadline, "logos still loading");
        ready.extend(logos.receive());
        std::thread::sleep(Duration::from_millis(5));
    }
    ready
}

#[test]
fn a_logo_shared_by_several_channels_takes_one_slot_and_is_not_asked_for_again() {
    let url = fixture("shared", 200, 100);
    let mut logos = Logos::new(8, &std::env::temp_dir().join(format!("mana-tv-logos-{}", std::process::id())));
    logos.want(std::iter::repeat_n(url.as_str(), 5));
    let ready = settle(&mut logos);
    logos.want(std::iter::repeat_n(url.as_str(), 5));
    assert!(!logos.loader.loading());
    assert_eq!(ready.len(), 1);
    assert_eq!((ready[0].1.w, ready[0].1.h), (128, 64));
    assert_eq!((logos.resident(), logos.free.len()), (1, 7));
}
