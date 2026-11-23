//! Channel logos: fetched and decoded off the UI thread, already scaled to
//! their cell, then kept as RGBA in fixed slots of one texture, the least
//! recently drawn given up first. Only logos still wanted are fetched, in
//! the order asked for (visible rows first), each once however many
//! channels share it, and uploads are spread over frames: the GL driver
//! converts texture data on the CPU.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use image::{ImageReader, Limits};
use mana_core::log::log;
use mana_core::net::Client;
use url::Url;

use super::gfx::{IMAGE_SLOT, Renderer};

/// Fetching and decoding threads; more raise the CPU peaks while scrolling.
const WORKERS: usize = 2;
/// Decoded logos turned into textures per frame.
const UPLOADS_PER_FRAME: usize = 3;
/// A slot drawn within this many frames (about 2 s) is not given up.
const KEEP_FRAMES: u64 = 120;
const MAX_LOGO_BYTES: u64 = 2 * 1024 * 1024;
/// The largest logo decoded, in pixels per side and in memory: a small file
/// can unpack into a huge image, and each of the WORKERS decodes one.
const MAX_LOGO_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 32 * 1024 * 1024;
/// A logo taking longer holds one of the WORKERS from the others.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Failures logged before only counting them.
const LOGGED_FAILURES: u64 = 5;

enum State {
    Ready { slot: usize, w: u32, h: u32, drawn: u64 },
    Failed,
}

struct Decoded {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

/// URLs to fetch, in order, and those taken by a worker whose result the
/// UI has not received yet: neither is asked for again meanwhile.
#[derive(Default)]
struct Queue {
    pending: VecDeque<String>,
    fetching: HashSet<String>,
}

pub struct Logos {
    states: HashMap<String, State>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    results: Receiver<(String, Result<Decoded, String>)>,
    /// Logos taken by a worker whose result is not received yet.
    in_flight: Arc<AtomicUsize>,
    free: Vec<usize>,
    frame: u64,
    pub loaded: u64,
    pub failed: u64,
}

impl Logos {
    pub fn new(slots: usize) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let (sender, results) = channel();
        let in_flight = Arc::new(AtomicUsize::new(0));
        spawn_workers(&queue, &sender, &in_flight);
        let free = (0..slots).rev().collect();
        Self { states: HashMap::new(), queue, results, in_flight, free, frame: 0, loaded: 0, failed: 0 }
    }

    /// Whether logos asked for are still being fetched, decoded or uploaded.
    pub fn loading(&self) -> bool {
        self.in_flight.load(Ordering::Acquire) > 0 || !self.queue.0.lock().unwrap().pending.is_empty()
    }

    /// The logos to have, most wanted first, each once: those not loaded
    /// yet replace what was queued before.
    pub fn want<'a>(&mut self, urls: impl Iterator<Item = &'a str>) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap();
        let mut seen = HashSet::new();
        let pending: VecDeque<String> = urls
            .filter(|url| !self.states.contains_key(*url) && !queue.fetching.contains(*url) && seen.insert(*url))
            .map(str::to_owned)
            .collect();
        queue.pending = pending;
        wake.notify_all();
    }

    /// Turns a few decoded logos into textures; call once per frame.
    pub fn upload(&mut self, r: &mut Renderer) {
        self.frame += 1;
        for (slot, image) in self.receive() {
            r.upload_image(slot, image.w as i32, image.h as i32, &image.rgba);
        }
    }

    /// Up to UPLOADS_PER_FRAME logos received, with the slot each now has;
    /// failures are recorded. A logo leaves `fetching` only here, once its
    /// state is known: before, `want` would queue it again.
    fn receive(&mut self) -> Vec<(usize, Decoded)> {
        let mut ready = Vec::new();
        for _ in 0..UPLOADS_PER_FRAME {
            let Ok((url, result)) = self.results.try_recv() else { break };
            self.queue.0.lock().unwrap().fetching.remove(&url);
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            if let Some(placed) = self.place(url, result) {
                ready.push(placed);
            }
        }
        ready
    }

    /// Records the logo of `url` as `result` gives it: in a slot, which it
    /// returns with the image to upload, or failed. Without a slot free, it
    /// is asked for again when drawn.
    fn place(&mut self, url: String, result: Result<Decoded, String>) -> Option<(usize, Decoded)> {
        match result {
            Ok(image) => {
                let slot = self.slot()?;
                self.states.insert(url, State::Ready { slot, w: image.w, h: image.h, drawn: self.frame });
                self.loaded += 1;
                Some((slot, image))
            }
            Err(error) => {
                self.failed += 1;
                if self.failed <= LOGGED_FAILURES {
                    log!("logos: {error}");
                }
                self.states.insert(url, State::Failed);
                None
            }
        }
    }

    /// Draws the logo of `url` fitted and centred in the box, if loaded.
    pub fn draw(&mut self, r: &mut Renderer, url: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
        let Some(State::Ready { slot, w: iw, h: ih, drawn }) = self.states.get_mut(url) else { return false };
        *drawn = self.frame;
        let scale = (w / *iw as f32).min(h / *ih as f32);
        let (dw, dh) = (*iw as f32 * scale, *ih as f32 * scale);
        r.image(*slot, *iw as i32, *ih as i32, x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh);
        true
    }

    /// Logos ready in texture slots.
    pub fn resident(&self) -> usize {
        self.states.values().filter(|s| matches!(s, State::Ready { .. })).count()
    }

    /// A free slot, or the one drawn least recently if not within KEEP_FRAMES.
    fn slot(&mut self) -> Option<usize> {
        if let Some(slot) = self.free.pop() {
            return Some(slot);
        }
        let (url, slot) = self
            .states
            .iter()
            .filter_map(|(url, state)| match state {
                State::Ready { slot, drawn, .. } if drawn + KEEP_FRAMES < self.frame => Some((url, *slot, *drawn)),
                _ => None,
            })
            .min_by_key(|(_, _, drawn)| *drawn)
            .map(|(url, slot, _)| (url.clone(), slot))?;
        self.states.remove(&url);
        Some(slot)
    }
}

/// Starts the WORKERS threads that fetch and decode the logos `queue`
/// asks for, sending them to `results`.
fn spawn_workers(
    queue: &Arc<(Mutex<Queue>, Condvar)>,
    results: &Sender<(String, Result<Decoded, String>)>,
    in_flight: &Arc<AtomicUsize>,
) {
    let client = match Client::new() {
        Ok(client) => client,
        Err(error) => {
            log!("logos: HTTP client not created: {error}");
            return;
        }
    };
    for i in 0..WORKERS {
        let (queue, results, client, in_flight) = (queue.clone(), results.clone(), client.clone(), in_flight.clone());
        let spawned = std::thread::Builder::new()
            .name(format!("logo{i}"))
            .spawn(move || work(&queue, &results, &client, &in_flight));
        if let Err(error) = spawned {
            log!("logos: thread not started: {error}");
        }
    }
}

fn work(
    queue: &(Mutex<Queue>, Condvar),
    results: &Sender<(String, Result<Decoded, String>)>,
    client: &Client,
    in_flight: &AtomicUsize,
) {
    loop {
        let url = next_url(queue, in_flight);
        let result = fetch(client, &url).and_then(|bytes| decode(&bytes)).map_err(|e| format!("{}: {e}", short(&url)));
        if results.send((url, result)).is_err() {
            return;
        }
    }
}

/// Waits for a pending logo and takes it, marked as fetching and counted in
/// flight before the queue is unlocked: `loading` never sees a logo neither
/// pending nor in flight.
fn next_url((lock, wake): &(Mutex<Queue>, Condvar), in_flight: &AtomicUsize) -> String {
    let mut queue = wake.wait_while(lock.lock().unwrap(), |q| q.pending.is_empty()).unwrap();
    let url = queue.pending.pop_front().expect("woken with a pending logo");
    in_flight.fetch_add(1, Ordering::AcqRel);
    queue.fetching.insert(url.clone());
    url
}

/// Downloads the logo, or reads it when `url` is a local path (fixtures).
fn fetch(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("http") {
        return std::fs::read(url).map_err(|e| e.to_string());
    }
    client.get_small(&Url::parse(url).map_err(|e| e.to_string())?, MAX_LOGO_BYTES, FETCH_TIMEOUT).map_err(|e| e.to_string())
}

/// Decodes and scales the logo to fit a texture slot, keeping its shape;
/// one larger than MAX_LOGO_SIDE or MAX_DECODE_BYTES is refused.
fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_LOGO_SIDE);
    limits.max_image_height = Some(MAX_LOGO_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let image = image.thumbnail(IMAGE_SLOT.0 as u32, IMAGE_SLOT.1 as u32).to_rgba8();
    let (w, h) = image.dimensions();
    Ok(Decoded { w, h, rgba: image.into_raw() })
}

/// Host and file name of a URL, for the log.
fn short(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split('/').next().unwrap_or("");
    let file = rest.rsplit('/').next().unwrap_or("");
    format!("{host}/…/{}", file.split('?').next().unwrap_or(""))
}

#[cfg(test)]
mod tests;
