//! Starting what the viewer chose: a clip, a channel, a folder to open, the
//! signals of a source to choose among, and what the home screen says when
//! it cannot start.

use mana_core::catalog::{Channel, ChannelError, Play};
use mana_core::language::{Language, Localized};
use mana_core::session::{Failure, Program};

use super::App;
use super::signals_menu::SignalsMenu;
use crate::settings;

/// What the home screen says went wrong, kept as it happened: it reads in
/// the language of each frame.
pub(super) enum Message {
    /// The channel cannot be played.
    Unplayable { channel: Box<Channel>, error: ChannelError },
    NotStarted(Failure),
}

impl Localized for Message {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Unplayable { channel, error } => format!("{}: {}", channel.title(language), error.text(language)),
            Self::NotStarted(failure) => failure.text(language),
        }
    }
}

impl App {
    /// Plays the clip at `index`, after stopping what was playing.
    pub fn play_clip(&mut self, index: usize) {
        self.close_folder();
        self.focus = self.channels.len() + index;
        self.play_focused();
    }

    /// Plays the channel at `index`; returns false, with the reason on the
    /// home screen, when it cannot be played.
    pub fn play_channel(&mut self, index: usize) -> bool {
        self.channels.set_focus(index);
        self.focus = index;
        match self.play_focused_channel() {
            Ok(()) => true,
            Err(error) => {
                if let Some(channel) = self.channels.focused().cloned().map(Box::new) {
                    self.message = Some(Message::Unplayable { channel, error });
                }
                false
            }
        }
    }

    /// Plays the focused channel; the reason it cannot be played, if so.
    pub(super) fn play_focused_channel(&mut self) -> Result<(), ChannelError> {
        let Some(channel) = self.channels.focused().cloned() else { return Ok(()) };
        self.play(channel)
    }

    /// Plays `channel`, or opens it when it is a folder; the reason it
    /// cannot be played, if so.
    pub(super) fn play(&mut self, channel: Channel) -> Result<(), ChannelError> {
        let name = channel.title(self.languages.current()).to_string();
        let program = match channel.play? {
            Play::Folder(folder) => {
                self.open_folder(folder);
                return Ok(());
            }
            Play::Stream(playable) => Program::Live(playable),
            // Several signals: the viewer chooses one, what plays goes on.
            Play::Source(selection) if selection.source.sources.is_some() => {
                self.signals = Some(SignalsMenu::ask(name, selection));
                self.signals_open = true;
                return Ok(());
            }
            Play::Source(selection) => Program::Source(selection),
        };
        self.start(name, program);
        Ok(())
    }

    /// Starts a viewing with the playback settings of the files as they are
    /// now, read again each time so they can change over SSH; a failure to
    /// start shows on the home screen. The settings screen closes, with its
    /// page.
    pub(super) fn start(&mut self, name: String, program: Program) {
        self.setup = None;
        self.signals = None;
        self.signals_open = false;
        self.info.started();
        self.message = self.session.play(name, program, settings::load().playback).err().map(Message::NotStarted);
    }
}
