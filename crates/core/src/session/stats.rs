//! What a viewing shows the screens: its state, with what live and playback
//! keep each for itself.

use std::sync::{Arc, Mutex};

use crate::live::Quality;
use crate::playback::Phase;
use crate::{live, playback};

/// Where a viewing is. It reads in the log as `avvio`, `riproduzione`,
/// `fine`, `fermato` or `errore: …`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Before its decoder is loaded.
    #[default]
    Starting,
    Playing,
    /// Playing out the end of a file.
    Ending,
    /// Ended without an error: at its end, or stopped.
    Stopped,
    /// Ended by this error.
    Failed(String),
}

impl State {
    pub fn ended(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_))
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Starting => f.write_str("avvio"),
            Self::Playing => f.write_str("riproduzione"),
            Self::Ending => f.write_str("fine"),
            Self::Stopped => f.write_str("fermato"),
            Self::Failed(error) => write!(f, "errore: {error}"),
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct Stats {
    pub state: State,
    /// File being played, without extension.
    pub name: String,
    pub playback: playback::Stats,
    pub live: live::Stats,
    /// The quality asked for, among the heights in `live`.
    pub quality: Quality,
}

/// Where a viewing's thread and the capabilities it runs write.
#[derive(Default)]
pub(super) struct Shared {
    /// How the viewing ended, once it did.
    pub ended: Mutex<Option<State>>,
    pub playback: Mutex<playback::Stats>,
    pub live: Arc<Mutex<live::Stats>>,
}

impl Shared {
    pub fn merge(&self, name: &str, quality: Quality) -> Stats {
        let playback = self.playback.lock().unwrap().clone();
        let state = self.ended.lock().unwrap().clone().unwrap_or(match playback.phase {
            Phase::Opening => State::Starting,
            Phase::Playing => State::Playing,
            Phase::Draining => State::Ending,
        });
        Stats { state, name: name.into(), playback, live: self.live.lock().unwrap().clone(), quality }
    }
}
