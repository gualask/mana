//! Channel logos as thumbnails, loaded off the interface's thread: each
//! downloaded once for its URL, decoded and scaled to THUMBNAIL, kept on
//! disk and read from there at the next starts without asking its server
//! again (a logo that changes comes with a new URL). Only logos still
//! wanted are loaded, in the order asked for (visible rows first), each once
//! however many channels share it. How a logo is drawn is each app's.

use std::collections::{HashSet, VecDeque};
use std::io::Cursor;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use image::{ImageFormat, ImageReader, Limits, RgbaImage};
use url::Url;

use super::Channel;
use crate::log::{debug, log};
use crate::net::{self, Client};

/// The largest thumbnail, in pixels: a logo's cell on the TV.
pub const THUMBNAIL: (u32, u32) = (128, 72);
/// A logo's box in a row, in pixels of a 1080p screen: the TV draws it at
/// this size, the desktop in the same shape at its own scale.
pub const BOX: (f32, f32) = (92.0, 48.0);
/// Loading threads; more raise the CPU peaks while scrolling.
const WORKERS: usize = 2;
const MAX_LOGO_BYTES: u64 = 2 * 1024 * 1024;
/// The largest logo decoded, in pixels per side and in memory: a small file
/// can unpack into a huge image, and each of the WORKERS decodes one.
const MAX_LOGO_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: u64 = 32 * 1024 * 1024;
/// A logo taking longer holds one of the WORKERS from the others.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// Failures logged before only counting them.
const LOGGED_FAILURES: u64 = 5;
/// The folder of the thumbnails kept, in the data directory.
const CACHE: &str = "logos";
/// Past this size at start the folder is emptied, and fills again with the
/// logos drawn: about a thousand thumbnails.
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

/// What a logo's box shows while the channel has none, or it has not come:
/// a folder's mark, or the channel's initial.
#[derive(Debug, PartialEq)]
pub enum Placeholder {
    Folder,
    Initial(String),
}

/// Whether the rows of `channels` leave room for a logo, each of them: when
/// any channel has one, so that the names line up.
pub fn room_for_logos<'a>(channels: impl IntoIterator<Item = &'a Channel>) -> bool {
    channels.into_iter().any(|channel| channel.logo.is_some())
}

/// A thumbnail: `w` × `h` straight-alpha RGBA pixels, rows top first.
pub struct Logo {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// URLs to load, in order, and those taken by a worker whose result was not
/// taken yet: neither is asked for again meanwhile.
#[derive(Default)]
struct Queue {
    pending: VecDeque<String>,
    loading: HashSet<String>,
}

pub struct Loader {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    results: Receiver<(String, Result<Logo, String>)>,
    pub loaded: u64,
    pub failed: u64,
}

impl Loader {
    /// Starts the WORKERS threads, with the thumbnails kept in `data_dir`.
    pub fn start(data_dir: &Path) -> Self {
        let cache = data_dir.join(CACHE);
        trim(&cache);
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let (sender, results) = channel();
        spawn_workers(&queue, &sender, &cache);
        Self { queue, results, loaded: 0, failed: 0 }
    }

    /// Whether logos asked for are still being loaded, or wait to be taken.
    pub fn loading(&self) -> bool {
        let queue = self.queue.0.lock().unwrap();
        !queue.pending.is_empty() || !queue.loading.is_empty()
    }

    /// The logos to load, most wanted first, each once: they replace what
    /// was queued before. The caller leaves out those it has.
    pub fn want<'a>(&self, urls: impl Iterator<Item = &'a str>) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap();
        let mut seen = HashSet::new();
        let pending: VecDeque<String> =
            urls.filter(|url| !queue.loading.contains(*url) && seen.insert(*url)).map(str::to_owned).collect();
        queue.pending = pending;
        wake.notify_all();
    }

    /// A logo loaded, or why it could not be, if one came. A logo stops
    /// being loading only here, once the caller knows it: before, `want`
    /// would queue it again.
    pub fn take(&mut self) -> Option<(String, Result<Logo, String>)> {
        let (url, result) = self.results.try_recv().ok()?;
        self.queue.0.lock().unwrap().loading.remove(&url);
        match &result {
            Ok(_) => self.loaded += 1,
            Err(error) => {
                self.failed += 1;
                if self.failed <= LOGGED_FAILURES {
                    log!("logos: {error}");
                }
            }
        }
        Some((url, result))
    }
}

/// Empties `cache` once it passes MAX_CACHE_BYTES, and makes sure it exists.
fn trim(cache: &Path) {
    let size: u64 = std::fs::read_dir(cache)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum();
    if size > MAX_CACHE_BYTES {
        log!("logos: {} MB kept on disk, emptied", size / 1024 / 1024);
        let _ = std::fs::remove_dir_all(cache);
    }
    if let Err(error) = std::fs::create_dir_all(cache) {
        log!("logos: {} not created: {error}", cache.display());
    }
}

/// Starts the WORKERS threads that load the logos `queue` asks for,
/// sending them to `results`.
fn spawn_workers(
    queue: &Arc<(Mutex<Queue>, Condvar)>,
    results: &Sender<(String, Result<Logo, String>)>,
    cache: &Path,
) {
    let client = match Client::new() {
        Ok(client) => client,
        Err(error) => {
            log!("logos: HTTP client not created: {error}");
            return;
        }
    };
    for i in 0..WORKERS {
        let (queue, results, client) = (queue.clone(), results.clone(), client.clone());
        let cache = cache.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name(format!("logo{i}"))
            .spawn(move || work(&queue, &results, &client, &cache));
        if let Err(error) = spawned {
            log!("logos: thread not started: {error}");
        }
    }
}

fn work(
    queue: &(Mutex<Queue>, Condvar),
    results: &Sender<(String, Result<Logo, String>)>,
    client: &Client,
    cache: &Path,
) {
    loop {
        let url = next_url(queue);
        let result = load(client, cache, &url).map_err(|e| format!("{}: {e}", loggable(&url)));
        if results.send((url, result)).is_err() {
            return;
        }
    }
}

/// Waits for a pending logo and takes it, marked as loading before the
/// queue is unlocked: `loading` never sees a logo neither pending nor
/// loading.
fn next_url((lock, wake): &(Mutex<Queue>, Condvar)) -> String {
    let mut queue = wake.wait_while(lock.lock().unwrap(), |q| q.pending.is_empty()).unwrap();
    let url = queue.pending.pop_front().expect("woken with a pending logo");
    queue.loading.insert(url.clone());
    url
}

/// The logo of `url`: its thumbnail kept in `cache`, or else downloaded,
/// decoded and kept there; read, not kept, when `url` is a local path
/// (fixtures).
fn load(client: &Client, cache: &Path, url: &str) -> Result<Logo, String> {
    if !url.starts_with("http") {
        return decode(&std::fs::read(url).map_err(|e| e.to_string())?);
    }
    let kept = cache.join(super::url_file_name(url, ".png"));
    if let Some(logo) = std::fs::read(&kept).ok().and_then(|bytes| decode(&bytes).ok()) {
        return Ok(logo);
    }
    let bytes = client.get_small(&Url::parse(url).map_err(|e| e.to_string())?, MAX_LOGO_BYTES, FETCH_TIMEOUT);
    let logo = decode(&bytes.map_err(|e| e.to_string())?)?;
    keep(&kept, &logo);
    Ok(logo)
}

/// Saves `logo` as a PNG at `path`, whole or not at all
/// (`files::write_whole`): a start reads only thumbnails written to the end.
fn keep(path: &Path, logo: &Logo) {
    let Some(image) = RgbaImage::from_raw(logo.w, logo.h, logo.rgba.clone()) else { return };
    let mut png = Cursor::new(Vec::new());
    let saved = image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| e.to_string())
        .and_then(|()| crate::files::write_whole(path, png.get_ref()).map_err(|e| e.to_string()));
    if let Err(error) = saved {
        debug!("logos: {} not kept: {error}", path.display());
    }
}

/// Decodes the logo and scales it to THUMBNAIL, keeping its shape; one
/// larger than MAX_LOGO_SIDE or MAX_DECODE_BYTES is refused.
fn decode(bytes: &[u8]) -> Result<Logo, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_LOGO_SIDE);
    limits.max_image_height = Some(MAX_LOGO_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let image = image.thumbnail(THUMBNAIL.0, THUMBNAIL.1).to_rgba8();
    let (w, h) = image.dimensions();
    Ok(Logo { w, h, rgba: image.into_raw() })
}

/// A logo's address for the log: a URL without its tokens, as the network
/// logs them (`net::loggable`); a local path as it is.
fn loggable(url: &str) -> String {
    Url::parse(url).map_or_else(|_| url.to_owned(), |url| net::loggable(&url))
}

#[cfg(test)]
mod tests;
