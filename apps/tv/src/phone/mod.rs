//! The way in for the phone while the settings screen is open: a page at
//! `http://<TV's address>:<port>/<code>`, served on the local network, where
//! the list's URL is pasted. The server hands each URL to the app and waits
//! for what the TV did with it; it never touches the screens.

mod http;
mod page;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mana_core::catalog::update::list_url;
use mana_core::language::{Language, Localized};
use mana_core::log::log;
use mana_core::text;
use url::Url;

use http::Method;
use page::Notice;

/// The first port tried, then the next ones while taken.
const FIRST_PORT: u16 = 8080;
const PORTS_TRIED: u16 = 10;
/// The code's characters: none that reads as another (0 and O, 1, I, L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 4;
/// How long a connection may stay silent: a browser opens one ahead and
/// sends on it only when «Save» is pressed.
const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// The most connections served at once: a phone opens two or three.
const MAX_CONNECTIONS: usize = 8;
/// How long the phone waits for the TV to download the list.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// A URL the phone sent, and where its answer goes.
pub struct Request {
    pub url: Url,
    reply: Sender<Reply>,
}

impl Request {
    /// Tells the phone what the TV did with the URL.
    pub fn answer(&self, saved: bool, text: String) {
        drop(self.reply.send(Reply { saved, text }));
    }
}

struct Reply {
    saved: bool,
    text: String,
}

/// The server, listening until dropped.
pub struct Server {
    address: String,
    local: SocketAddr,
    requests: Receiver<Request>,
    site: Arc<Site>,
    stop: Arc<AtomicBool>,
}

/// What every connection serves: the page at `path`, with the URL in use,
/// in the app's language.
struct Site {
    path: String,
    current: Mutex<Option<String>>,
    language: Mutex<Language>,
    requests: Sender<Request>,
}

impl Site {
    fn current(&self) -> Option<String> {
        self.current.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn language(&self) -> Language {
        *self.language.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Why the page cannot be served.
pub enum StartError {
    NoNetwork,
    /// No port free from FIRST_PORT on.
    NoFreePort,
    /// What the system says.
    System(String),
}

impl Localized for StartError {
    fn text(&self, language: Language) -> String {
        let last = FIRST_PORT + PORTS_TRIED - 1;
        match self {
            Self::NoNetwork => text!(language, "phone-no-network"),
            Self::NoFreePort => text!(language, "phone-no-free-port", first = FIRST_PORT, last = last),
            Self::System(detail) => detail.clone(),
        }
    }
}

impl Server {
    /// Listens on the TV's address on the local network, at a new code,
    /// serving the page in `language`; why not, if it cannot.
    pub fn start(language: Language) -> Result<Self, StartError> {
        let ip = local_ip().ok_or(StartError::NoNetwork)?;
        let listener = bind(ip).ok_or(StartError::NoFreePort)?;
        let local = listener.local_addr().map_err(|e| StartError::System(e.to_string()))?;
        let path = format!("/{}", code());
        let address = format!("http://{local}{path}");
        let (sender, requests) = channel();
        let site = Arc::new(Site { path, current: Mutex::new(None), language: Mutex::new(language), requests: sender });
        let stop = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("phone".into())
            .spawn({
                let (site, stop) = (site.clone(), stop.clone());
                move || listen(&listener, &site, &stop)
            })
            .map_err(|e| StartError::System(e.to_string()))?;
        // Without the code, which alone keeps the page from others.
        log!("phone: page on http://{local}/…");
        Ok(Self { address, local, requests, site, stop })
    }

    /// The page's address, as the QR code and the screen give it.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The next URL the phone sent, if any.
    pub fn request(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }

    /// The URL the form shows when it opens.
    pub fn set_current(&self, url: Option<&Url>) {
        *self.site.current.lock().unwrap_or_else(|e| e.into_inner()) = url.map(|u| u.to_string());
    }

    /// The language of the page from the next request.
    pub fn set_language(&self, language: Language) {
        *self.site.language.lock().unwrap_or_else(|e| e.into_inner()) = language;
    }
}

impl Drop for Server {
    /// Stops listening: the thread, woken by a connection of its own,
    /// closes the port.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        drop(TcpStream::connect_timeout(&self.local, Duration::from_millis(200)));
        log!("phone: page closed");
    }
}

/// Serves each connection in a thread of its own until `stop`, so that
/// one left silent does not hold up the others.
fn listen(listener: &TcpListener, site: &Arc<Site>, stop: &AtomicBool) {
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Ok(stream) = stream else { continue };
        let Some(slot) = Slot::take(&open) else { continue };
        let site = site.clone();
        let spawned = std::thread::Builder::new().name("phone-connection".into()).spawn(move || {
            serve(stream, &site);
            drop(slot);
        });
        if let Err(error) = spawned {
            log!("phone: connection not served: {error}");
        }
    }
}

/// One of the MAX_CONNECTIONS, given back when dropped.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>) -> Option<Self> {
        let free = open.fetch_add(1, Ordering::AcqRel) < MAX_CONNECTIONS;
        let slot = Slot(open.clone());
        free.then_some(slot)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Answers the request on `stream`; a connection closed or silent before
/// sending anything is closed without an answer, which the browser would
/// otherwise read as the answer to the request it sends on it later.
fn serve(mut stream: TcpStream, site: &Site) {
    drop(stream.set_read_timeout(Some(IO_TIMEOUT)));
    drop(stream.set_write_timeout(Some(IO_TIMEOUT)));
    let (status, body) = match http::read(&mut stream) {
        Ok(None) => return,
        Ok(Some(request)) => answer(&request, site),
        Err(refused) => (refused.status(), String::new()),
    };
    if let Err(error) = http::respond(&mut stream, status, &body) {
        log!("phone: answer not sent: {error}");
    }
}

/// The status and page for `request`.
fn answer(request: &http::Request, site: &Site) -> (u16, String) {
    if !request.path.eq_ignore_ascii_case(&site.path) {
        return (404, String::new());
    }
    match request.method {
        Method::Get => (200, page::page(&site.path, site.current().as_deref(), Notice::None, site.language())),
        Method::Post => {
            let sent = http::form_value(&request.body, "url").unwrap_or_default();
            (200, posted(site, &sent))
        }
    }
}

/// The page after «Save»: what the TV did with the URL `sent`.
fn posted(site: &Site, sent: &str) -> String {
    let (path, language) = (site.path.as_str(), site.language());
    let refused = |text: String| page::page(path, site.current().as_deref(), Notice::Refused { text: &text, sent }, language);
    let url = match list_url(sent) {
        Ok(url) => url,
        Err(reason) => {
            let reason = reason.text(language);
            return refused(text!(language, "notice-not-saved", reason = reason));
        }
    };
    let in_use = url.to_string();
    let (reply, replies) = channel();
    if site.requests.send(Request { url, reply }).is_err() {
        return refused(text!(language, "phone-settings-closed"));
    }
    match replies.recv_timeout(REPLY_TIMEOUT) {
        Ok(Reply { saved: true, text }) => page::page(path, Some(&in_use), Notice::Saved(&text), language),
        Ok(Reply { saved: false, text }) => refused(text),
        Err(RecvTimeoutError::Timeout) => refused(
            text!(language, "phone-still-downloading"),
        ),
        Err(RecvTimeoutError::Disconnected) => refused(
            text!(language, "phone-closed-meanwhile"),
        ),
    }
}

/// A port of `ip`: the first one free from FIRST_PORT.
fn bind(ip: IpAddr) -> Option<TcpListener> {
    (FIRST_PORT..FIRST_PORT + PORTS_TRIED).find_map(|port| TcpListener::bind((ip, port)).ok())
}

/// The TV's address on the local network: the one a datagram to the
/// outside would leave from (connecting a UDP socket sends nothing).
fn local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip()).filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
}

/// A new random code, from the system's randomness that seeds the hashes.
fn code() -> String {
    let mut bits = RandomState::new().build_hasher().finish();
    (0..CODE_LENGTH)
        .map(|_| {
            let c = CODE_ALPHABET[(bits % CODE_ALPHABET.len() as u64) as usize];
            bits /= CODE_ALPHABET.len() as u64;
            char::from(c)
        })
        .collect()
}

#[cfg(test)]
mod tests;
