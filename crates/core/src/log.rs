//! Line log to stderr and, on the TV, to a file readable over SSH.
//!
//! Two levels: `log!` for what a viewing, a list or the app does once in a
//! while (a start, an error, a switch), always written; `debug!` for what
//! repeats while playing (positions, sections, decoder events, keys),
//! written only while debugging (`set_debug`), and not even formatted
//! otherwise.
//!
//! Never log URLs with tokens, cookies or keys.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static FILE: Mutex<Option<File>> = Mutex::new(None);
static DEBUG: AtomicBool = AtomicBool::new(false);
/// The log lives in RAM on the TV (/tmp) and system libraries write to it
/// through stderr too, some several times a second: past this size it is
/// emptied.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

pub fn init(path: Option<&str>) {
    if let Some(path) = path {
        // Append mode: stderr shares this file and truncation must not leave
        // either writer at a stale offset.
        let file = OpenOptions::new().create(true).append(true).open(path).ok();
        let file = file.filter(|f| f.set_len(0).is_ok());
        *FILE.lock().unwrap() = file;
    }
    std::panic::set_hook(Box::new(|info| write(&format!("panic: {info}"))));
}

/// Writes the `debug!` lines too, or stops writing them.
pub fn set_debug(on: bool) {
    DEBUG.store(on, Ordering::Relaxed);
}

/// Whether the `debug!` lines are written.
pub fn debugging() -> bool {
    DEBUG.load(Ordering::Relaxed)
}

/// Empties the log once it exceeds MAX_LOG_BYTES.
pub fn trim() {
    let mut guard = FILE.lock().unwrap();
    let Some(file) = guard.as_mut() else { return };
    if file.metadata().is_ok_and(|m| m.len() > MAX_LOG_BYTES) && file.set_len(0).is_ok() {
        let _ = file.write_all(b"log svuotato: superato il limite di dimensione\n");
    }
}

pub fn write(message: &str) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let line = format!("{}.{:03} {message}\n", now.as_secs(), now.subsec_millis());
    match FILE.lock().unwrap().as_mut() {
        // On the TV stderr is redirected to this file too (crash.c).
        Some(file) => {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        None => {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
/// `log!("…", …)`: one formatted line, as `write`.
pub use __log as log;

#[doc(hidden)]
#[macro_export]
macro_rules! __debug {
    ($($arg:tt)*) => {
        if $crate::log::debugging() {
            $crate::log::write(&format!($($arg)*))
        }
    };
}
/// `debug!("…", …)`: as `log!`, only while debugging.
pub use __debug as debug;
