//! The viewing session: one source plays at a time, on its own thread.
//! Stopping or changing a viewing returns at once: its threads end on their
//! own, at their next check, and a new viewing opens its backend only once
//! the one before has released its own (Turn), so the remote never waits
//! for a request hanging on the network. Only closing the session waits
//! for the threads. Clients (the TV's remote, the unattended tests, the
//! desktop app) start, change and stop viewings only through it.

mod reopen;
mod stats;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::live::QualityChoice;
use crate::live::source::Source;
use crate::log::log;
use crate::playback::{OpenBackend, Settings};
pub use crate::live::Quality;
pub use crate::live::source::EXTENSION as STREAM_EXTENSION;
pub use stats::{State, Stats};
use stats::Shared;

/// What a viewing plays.
pub enum Program {
    /// A local file, or a `.stream` file describing a live source.
    Path(PathBuf),
    /// A live source, as a channel list gives it.
    Live(Source),
}

pub struct Session {
    open: OpenBackend,
    viewing: Option<Viewing>,
    /// The threads of the viewings stopped, until they end.
    stopping: Vec<JoinHandle<()>>,
    /// The backend, which the viewings take in turn.
    turn: Arc<Turn>,
    /// For every viewing, until changed.
    quality: QualityChoice,
}

struct Viewing {
    name: String,
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    thread: JoinHandle<()>,
}

impl Session {
    /// A session whose viewings play on the backends `open` makes.
    pub fn new(open: OpenBackend) -> Self {
        Self { open, viewing: None, stopping: Vec::new(), turn: Arc::default(), quality: QualityChoice::default() }
    }

    /// Plays `program`, shown as `name`, after stopping what was playing;
    /// returns at once.
    pub fn play(&mut self, name: String, program: Program, settings: Settings) -> Result<(), String> {
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared::default());
        let (thread_stop, thread_shared) = (stop.clone(), shared.clone());
        let (open, turn) = (self.open.clone(), self.turn.clone());
        let quality = self.quality.clone();
        log!("riproduzione {name} con {settings:?}, qualità {}", quality.get());
        let thread = std::thread::Builder::new()
            .name("feeder".into())
            .spawn(move || {
                let in_turn = InTurn::new(turn);
                watch(&program, &settings, &in_turn.opener(open, &thread_stop), &thread_stop, &thread_shared, &quality);
            })
            .map_err(|e| e.to_string())?;
        self.viewing = Some(Viewing { name, stop, shared, thread });
        Ok(())
    }

    /// Stops the viewing, if any, and returns at once: its threads end on
    /// their own.
    pub fn stop(&mut self) {
        self.stopping.retain(|thread| !thread.is_finished());
        let Some(viewing) = self.viewing.take() else { return };
        viewing.stop.store(true, Ordering::Relaxed);
        self.stopping.push(viewing.thread);
    }

    /// Whether the threads of viewings stopped are still running.
    pub fn is_stopping(&self) -> bool {
        self.stopping.iter().any(|thread| !thread.is_finished())
    }

    /// Stops the viewing, if any, waiting at most `limit` for the threads
    /// of all those stopped: for when the process is about to end, and the
    /// system frees what they still hold (uMediaServer unloads the
    /// pipelines of a client gone from the bus). Returns whether they
    /// finished; those still running are left to the process end.
    pub fn stop_within(&mut self, limit: Duration) -> bool {
        self.stop();
        let deadline = Instant::now() + limit;
        while self.is_stopping() {
            if Instant::now() >= deadline {
                self.stopping.clear();
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.stopping.drain(..).for_each(|thread| drop(thread.join()));
        true
    }

    /// Sets the quality of live streams with variants, for this viewing
    /// from its next segment and for the next ones; the heights to choose
    /// from, and the quality, are in the viewing's stats.
    pub fn set_quality(&self, quality: Quality) {
        if quality != self.quality.get() {
            log!("qualità {quality}");
        }
        self.quality.set(quality);
    }

    pub fn is_playing(&self) -> bool {
        self.viewing.is_some()
    }

    /// The viewing's state and counters, while there is one.
    pub fn stats(&self) -> Option<Stats> {
        self.viewing.as_ref().map(|v| v.shared.merge(&v.name, self.quality.get()))
    }
}

/// The backend, which the viewings take in turn: one is opened only once
/// the viewing before has released its own (the TV plays one pipeline at a
/// time), and a viewing stopped meanwhile gives up its turn.
#[derive(Default)]
struct Turn {
    taken: Mutex<bool>,
    given_back: Condvar,
}

impl Turn {
    /// Waits for the turn; false once `stop` is set instead.
    fn take(&self, stop: &AtomicBool) -> bool {
        let mut taken = self.taken.lock().unwrap();
        while *taken && !stop.load(Ordering::Relaxed) {
            taken = self.given_back.wait_timeout(taken, Duration::from_millis(50)).unwrap().0;
        }
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        *taken = true;
        true
    }

    fn give_back(&self) {
        *self.taken.lock().unwrap() = false;
        self.given_back.notify_all();
    }
}

/// A viewing's turn: taken at its first backend, given back when the
/// viewing's thread drops it, however that thread ends.
struct InTurn {
    turn: Arc<Turn>,
    holds: Arc<AtomicBool>,
}

impl InTurn {
    fn new(turn: Arc<Turn>) -> Self {
        Self { turn, holds: Arc::new(AtomicBool::new(false)) }
    }

    /// Opens backends with `open` once the viewing has the turn; an error
    /// once `stop` is set first.
    fn opener(&self, open: OpenBackend, stop: &Arc<AtomicBool>) -> OpenBackend {
        let (turn, stop, holds) = (self.turn.clone(), stop.clone(), self.holds.clone());
        Arc::new(move || {
            if !holds.load(Ordering::Relaxed) && !turn.take(&stop) {
                return Err("fermato prima di aprire il decoder".into());
            }
            holds.store(true, Ordering::Relaxed);
            open()
        })
    }
}

impl Drop for InTurn {
    fn drop(&mut self) {
        if self.holds.load(Ordering::Relaxed) {
            self.turn.give_back();
        }
    }
}

/// Plays `program` to its end, on the viewing's thread, and records how it
/// ended for the clients.
fn watch(
    program: &Program,
    settings: &Settings,
    open: &OpenBackend,
    stop: &Arc<AtomicBool>,
    shared: &Shared,
    quality: &QualityChoice,
) {
    let result = reopen::run(program, settings, open, stop, shared, quality);
    // Unblocks the HLS downloaders when playback ends on its own.
    stop.store(true, Ordering::Relaxed);
    let state = match result {
        Ok(()) => State::Stopped,
        Err(error) => State::Failed(error.to_string()),
    };
    log!("visione: {state}");
    *shared.ended.lock().unwrap() = Some(state);
}

impl Drop for Session {
    /// Waits for the threads of every viewing.
    fn drop(&mut self) {
        self.stop();
        self.stopping.drain(..).for_each(|thread| drop(thread.join()));
    }
}

#[cfg(test)]
mod tests;
