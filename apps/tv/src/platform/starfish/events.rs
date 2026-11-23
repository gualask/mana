//! What the Starfish pipeline reports through its load callback: the
//! event codes, the state they leave for the backend, and the wait for it.

use std::ffi::{CStr, c_char, c_int, c_longlong};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use mana_core::log::debug;

const EVENT_FRAME_READY: c_int = 0x00;
const EVENT_ERROR: c_int = 0x12;
const EVENT_LOAD_COMPLETED: c_int = 0x16;
const EVENT_UNLOAD_COMPLETED: c_int = 0x17;
/// Sent repeatedly while the pipeline's input buffer is low: counted only.
const EVENT_BUFFER_LOW: c_int = 0x2e;

/// What the pipeline reported, drained by the feeder.
#[derive(Default)]
pub(super) struct Shared {
    /// Presentation time of the last displayed frame (ns).
    pub(super) presented_ns: Option<i64>,
    pub(super) load_completed: bool,
    pub(super) unload_completed: bool,
    pub(super) error: Option<String>,
    pub(super) buffer_low: u64,
}

pub(super) static SHARED: Mutex<Shared> = Mutex::new(Shared {
    presented_ns: None,
    load_completed: false,
    unload_completed: false,
    error: None,
    buffer_low: 0,
});
static CHANGED: Condvar = Condvar::new();

/// Load callback. The API passes no user data, hence the global state; only
/// one pipeline exists at a time.
pub(super) unsafe extern "C" fn on_event(kind: c_int, value: c_longlong, text: *const c_char) {
    let text = if text.is_null() {
        String::new()
    } else {
        // SAFETY: the pipeline passes a NUL-terminated string or null.
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        // Some events carry binary data or trailing control bytes.
        text.chars().filter(|c| !c.is_control() && *c != char::REPLACEMENT_CHARACTER).collect()
    };
    let mut shared = SHARED.lock().unwrap();
    match kind {
        EVENT_FRAME_READY => shared.presented_ns = Some(value),
        EVENT_LOAD_COMPLETED => shared.load_completed = true,
        EVENT_UNLOAD_COMPLETED => shared.unload_completed = true,
        EVENT_ERROR => shared.error = Some(format!("{value} {text}")),
        EVENT_BUFFER_LOW => shared.buffer_low += 1,
        _ => {}
    }
    if kind != EVENT_FRAME_READY && kind != EVENT_BUFFER_LOW {
        debug!("event 0x{kind:x} {value} {text}");
    }
    CHANGED.notify_all();
}

/// Waits until the callback state satisfies `done`, up to `timeout`.
pub(super) fn wait_for(done: impl Fn(&Shared) -> bool, timeout: Duration) -> bool {
    let shared = SHARED.lock().unwrap();
    let (shared, _) = CHANGED.wait_timeout_while(shared, timeout, |s| !done(s)).unwrap();
    done(&shared)
}
