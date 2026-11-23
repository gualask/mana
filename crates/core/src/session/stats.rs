//! What a viewing shows the screens: its state, with what live and playback
//! keep each for itself.

use std::sync::{Arc, Mutex};

use super::reopen::MAX_REOPENINGS;
use crate::language::{Language, Localized, display_in_english};
use crate::live::playable;
use crate::live::{Cause, Interruption, Quality};
use crate::playback::Phase;
use crate::{live, playback};
use crate::text;

/// Where a viewing is.
#[derive(Clone, Debug, Default, PartialEq)]
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
    Failed(Failure),
}

impl State {
    pub fn ended(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_))
    }
}

impl Localized for State {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Starting => text!(language, "state-starting"),
            Self::Playing => text!(language, "state-playing"),
            Self::Ending => text!(language, "state-ending"),
            Self::Stopped => text!(language, "state-stopped"),
            Self::Failed(failure) => text!(language, "state-failed", failure = failure.text(language)),
        }
    }
}

/// Why a viewing ended, or could not start.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// A `.stream` file that cannot be used.
    Playable(playable::Error),
    /// The live stream stopped, and opening it again cannot help.
    Live(Interruption),
    Playback(playback::Error),
    /// The stream's token was refused after every new resolution allowed.
    RefusedAgain(Cause),
    /// The viewing's thread could not start, as the system says.
    Thread(String),
}

impl From<playback::Error> for Failure {
    fn from(error: playback::Error) -> Self {
        Self::Playback(error)
    }
}

impl Localized for Failure {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Playable(error) => error.text(language),
            Self::Live(interruption) => interruption.text(language),
            Self::Playback(error) => error.text(language),
            Self::RefusedAgain(cause) => {
                text!(language, "live-refused-again", count = MAX_REOPENINGS, cause = cause.text(language))
            }
            Self::Thread(detail) => detail.clone(),
        }
    }
}

display_in_english!(State, Failure);

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
