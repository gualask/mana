//! Feasibility screens: the channels of the list and the local test clips,
//! and over the video the information panel, the channel list and the quality
//! menu. Not the product UI.

pub mod channel_list;
mod folders;
pub mod gfx;
mod hits;
mod home;
mod info;
mod language;
mod list_updates;
mod loader;
pub mod logos;
mod qr;
pub mod quality_menu;
mod search;
pub mod signals_menu;
mod setup;
pub mod text;
mod video;

use std::path::{Path, PathBuf};

use channel_list::ChannelList;
use folders::OpenFolder;
use hits::Hits;
use info::InfoPanel;
use gfx::{Color, IMAGE_SLOTS, Renderer};
use language::Languages;
pub use loader::REDRAW as LOADER_REDRAW;
use logos::Logos;
use mana_core::catalog::folder::Folders;
use mana_core::catalog::{Channel, ChannelError, Play};
use mana_core::catalog::update::{self, Updater};
use mana_core::language::{Language, Localized};
use mana_core::session::{Failure, Program, Session, State, Stats};
use quality_menu::QualityMenu;
use search::Search;
use signals_menu::SignalsMenu;
use setup::Setup;
use text::Text;

use crate::palette;
use crate::platform::DATA_DIR;
use crate::platform::keys::Action;
use crate::settings;

/// Layout is authored for a 1920×1080 surface and scaled to the real one.
const DESIGN_HEIGHT: f32 = 1080.0;

const BACKGROUND: Color = Color::hex(palette::BACKGROUND);
const ROW: Color = Color::hex(palette::ROW);
const FOCUS: Color = Color::hex(palette::FOCUS);
const TEXT: Color = Color::hex(palette::TEXT);
const MUTED: Color = Color::hex(palette::MUTED);
const ERROR: Color = Color::hex(palette::ERROR);
const OK: Color = Color::hex(palette::OK);

pub enum Outcome {
    Continue,
    Exit,
}

/// What the home screen says went wrong, kept as it happened: it reads in
/// the language of each frame.
enum Message {
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

pub struct App {
    session: Session,
    /// The language the screens and the phone's page are in.
    languages: Languages,
    clips: Vec<PathBuf>,
    /// The home screen's focused row, among the channels then the clips.
    focus: usize,
    /// The first row shown: the list scrolls only when the focus leaves
    /// the rows shown, so that a row stays under the pointer.
    home_first: usize,
    message: Option<Message>,
    /// The channels, on the home screen and over the video: the list's,
    /// or the events of the folder open.
    channels: ChannelList,
    /// The sources' catalogues, read as their folders open.
    folders: Folders,
    /// The folder whose events are shown, if one is open.
    folder: Option<OpenFolder>,
    /// Downloads the list, at start and when the phone sends a URL; none
    /// with the tests' lists.
    updater: Option<Updater>,
    /// The settings file whose `list` wins over the URL chosen, if any.
    list_file: Option<PathBuf>,
    /// The home screen's «Settings» has the focus instead of a row.
    settings_focused: bool,
    /// The home screen's «Search» has the focus instead of a row.
    search_focused: bool,
    /// The search, while open: over the home screen, found again after
    /// what it played.
    search: Option<Search>,
    /// The settings screen, while open.
    setup: Option<Setup>,
    /// Why there is no list to show, if the update failed without one.
    list_failure: Option<update::Failure>,
    channels_open: bool,
    logos: Logos,
    /// Open over the video with Right, while the live stream offers heights.
    quality_menu: Option<QualityMenu>,
    /// The signals of the channel chosen last, when its source gives
    /// several: kept while it plays, so that Left opens them again.
    signals: Option<SignalsMenu>,
    signals_open: bool,
    /// Over the video, toggled with Up or Down and closed at first: a static
    /// panel left on screen can mark an OLED panel.
    info: InfoPanel,
    /// The rows of the last frame, for the pointer.
    hits: Hits,
}

impl App {
    /// The quality starts as the settings set it, then follows the menu for
    /// every channel. The channels are those of the tests' list, if copied,
    /// else the copy of the list saved, replaced by the list downloaded
    /// once it arrives.
    pub fn new(session: Session) -> Self {
        let loaded = settings::load();
        mana_core::log::set_debug(loaded.debug);
        let languages = Languages::start(loaded.locale);
        session.set_quality(loaded.quality);
        let (list, list_file) = loaded.list.unzip();
        let (channels, updater) = list_updates::channels(list, loaded.tests.ui_test);
        Self {
            session,
            languages,
            clips: home::clips(),
            focus: 0,
            home_first: 0,
            message: None,
            channels,
            folders: Folders::new(Path::new(DATA_DIR)),
            folder: None,
            updater,
            list_file,
            settings_focused: false,
            search_focused: false,
            search: None,
            setup: None,
            list_failure: None,
            channels_open: false,
            logos: Logos::new(IMAGE_SLOTS),
            quality_menu: None,
            signals: None,
            signals_open: false,
            info: InfoPanel::default(),
            hits: Hits::default(),
        }
    }

    /// Whether the screens know their language: the first frame waits for
    /// the TV's, for at most `locale::LIMIT`.
    pub fn language_ready(&self) -> bool {
        self.languages.ready()
    }

    /// Whether logos are still arriving for the channels shown: the screen
    /// is redrawn often enough to show them, not at every frame.
    pub fn loading_logos(&self) -> bool {
        (self.channels_open || !self.session.is_playing()) && self.logos.loading()
    }

    pub fn is_playing(&self) -> bool {
        self.session.is_playing()
    }

    /// Whether a viewing waits for its first frame, the loader turning, or
    /// the signals of a channel are on their way.
    pub fn waiting_for_video(&self) -> bool {
        self.session.stats().is_some_and(|stats| loader::waiting(&stats)) || self.signals.as_ref().is_some_and(SignalsMenu::asking)
    }

    pub fn clips(&self) -> &[PathBuf] {
        &self.clips
    }

    /// The clip focused on the home screen, if a clip is: the clips are
    /// not in a folder.
    pub fn focused_clip(&self) -> Option<usize> {
        self.focus.checked_sub(self.channels.len()).filter(|&i| i < self.clips_shown().len())
    }

    /// The clips the home screen shows: none in a folder.
    fn clips_shown(&self) -> &[PathBuf] {
        if self.folder.is_some() { &[] } else { &self.clips }
    }

    pub fn channels(&self) -> &ChannelList {
        &self.channels
    }

    pub fn channels_open(&self) -> bool {
        self.channels_open
    }

    pub fn logos(&self) -> &Logos {
        &self.logos
    }

    /// The viewing's state and counters, while there is one.
    pub fn stats(&self) -> Option<Stats> {
        self.session.stats()
    }

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

    /// Whether a viewing stopped is still ending its threads.
    pub fn is_stopping(&self) -> bool {
        self.session.is_stopping()
    }

    /// Before the process ends: stops playback, waiting at most `limit`.
    /// Returns whether playback is done.
    pub fn close(&mut self, limit: std::time::Duration) -> bool {
        self.session.stop_within(limit)
    }

    /// Sends a key to what has it: the channel list or the quality menu
    /// open over the video, the video itself, or the list of clips.
    pub fn handle(&mut self, action: Action) -> Outcome {
        // The screen may change: its rows are known again once drawn.
        self.hits.clear();
        if self.signals_open {
            self.handle_signals(action);
            return Outcome::Continue;
        }
        if !self.session.is_playing() {
            self.quality_menu = None;
            if self.setup.is_some() {
                self.handle_setup(action);
                return Outcome::Continue;
            }
            if self.search.is_some() {
                self.handle_search(action);
                return Outcome::Continue;
            }
            return self.handle_home(action);
        }
        if self.channels_open {
            self.handle_channels(action);
        } else if self.quality_menu.is_some() {
            self.handle_quality_menu(action);
        } else {
            self.handle_video(action);
        }
        Outcome::Continue
    }

    /// Plays the focused channel; the reason it cannot be played, if so.
    fn play_focused_channel(&mut self) -> Result<(), ChannelError> {
        let Some(channel) = self.channels.focused().cloned() else { return Ok(()) };
        self.play(channel)
    }

    /// Plays `channel`, or opens it when it is a folder; the reason it
    /// cannot be played, if so.
    fn play(&mut self, channel: Channel) -> Result<(), ChannelError> {
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

    /// Plays the signal at `index` of the menu, which stays for Left.
    fn play_signal(&mut self, index: usize) {
        let Some(mut menu) = self.signals.take() else { return };
        let chosen = menu.choose(index);
        let name = menu.name.clone();
        if let Some(selection) = chosen {
            self.start(name, Program::Source(selection));
        }
        self.signals = Some(menu);
        self.signals_open = false;
    }

    /// Takes the signals once they come, playing the only one at once, and
    /// opens the menu again when the signal in use failed.
    fn refresh_signals(&mut self) {
        let Some(menu) = &mut self.signals else { return };
        if menu.refresh() && menu.candidates().len() == 1 {
            self.play_signal(0);
            return;
        }
        if menu.in_use()
            && let Some(Stats { state: State::Failed(failure), .. }) = self.session.stats()
        {
            menu.failed(failure);
            self.signals_open = true;
        }
    }

    /// Starts a viewing with the playback settings of the files as they are
    /// now, read again each time so they can change over SSH; a failure to
    /// start shows on the home screen. The settings screen closes, with its
    /// page.
    fn start(&mut self, name: String, program: Program) {
        self.setup = None;
        self.signals = None;
        self.signals_open = false;
        self.info.started();
        self.message = self.session.play(name, program, settings::load().playback).err().map(Message::NotStarted);
    }

    /// Draws the frame on a surface of `surface` drawable pixels.
    pub fn draw(&mut self, r: &mut Renderer, t: &mut Text, surface: (i32, i32)) {
        let s = surface.1 as f32 / DESIGN_HEIGHT;
        let language = self.languages.current();
        self.hits.clear();
        if !self.session.is_playing() && self.setup.is_some() {
            self.draw_setup(r, t, s, surface, language);
        } else if !self.session.is_playing()
            && let Some(search) = &self.search
        {
            r.begin(surface.0, surface.1, BACKGROUND);
            search.draw(r, t, s, &mut self.hits, language);
        } else if !self.session.is_playing() {
            self.draw_home(r, t, s, surface, language);
        } else if self.channels_open {
            r.begin(surface.0, surface.1, Color::TRANSPARENT);
            self.logos.upload(r);
            self.channels.set_status(self.folder_status(language));
            self.channels.draw(r, t, &mut self.logos, s, &mut self.hits, language);
        } else {
            r.begin(surface.0, surface.1, Color::TRANSPARENT);
            self.draw_over_video(r, t, s, surface, language);
        }
        if self.signals_open
            && let Some(menu) = &self.signals
        {
            menu.draw(r, t, s, &mut self.hits, language);
        }
        r.finish();
    }
}
