//! Over the video: the keys of the channel list, the quality menu and the
//! video itself, and the drawing of the panel and the menu.

use mana_core::catalog::Play;
use mana_core::language::Language;
use mana_core::log::log;

use super::gfx::Renderer;
use super::quality_menu::QualityMenu;
use super::text::Text;
use super::{App, channel_list, loader};
use crate::platform::keys::Action;

impl App {
    pub(super) fn handle_channels(&mut self, action: Action) {
        match action {
            Action::Up => self.channels.step(-1),
            Action::Down => self.channels.step(1),
            Action::Left => self.channels.step(-channel_list::PAGE),
            Action::Right => self.channels.step(channel_list::PAGE),
            Action::Select => {
                // A folder opens in the list, which stays open.
                let folder = matches!(self.channels.focused().map(|c| &c.play), Some(Ok(Play::Folder(_))));
                match self.play_focused_channel() {
                    Ok(()) => self.channels_open = folder,
                    Err(reason) => self.channels.notice(reason),
                }
            }
            Action::Back => self.channels_open = false,
        }
    }

    pub(super) fn handle_signals(&mut self, action: Action) {
        let Some(menu) = &mut self.signals else {
            self.signals_open = false;
            return;
        };
        match action {
            Action::Up => menu.step(-1),
            Action::Down => menu.step(1),
            Action::Select if !menu.candidates().is_empty() => {
                let focused = menu.focused();
                self.play_signal(focused);
            }
            Action::Select | Action::Back | Action::Left | Action::Right => self.signals_open = false,
        }
    }

    pub(super) fn handle_quality_menu(&mut self, action: Action) {
        let Some(menu) = &mut self.quality_menu else { return };
        let heights = self.session.stats().map(|s| s.live.heights).unwrap_or_default();
        match action {
            Action::Up => menu.step(&heights, -1),
            Action::Down => menu.step(&heights, 1),
            Action::Select => {
                self.session.set_quality(menu.focused(&heights));
                self.quality_menu = None;
            }
            Action::Back | Action::Left | Action::Right => self.quality_menu = None,
        }
    }

    pub(super) fn handle_video(&mut self, action: Action) {
        match action {
            Action::Back => {
                self.session.stop();
                log!("playback stopped");
                self.refresh_folder();
            }
            Action::Select => {
                self.channels_open = true;
                self.refresh_folder();
            }
            Action::Up | Action::Down => self.info.toggle(),
            Action::Right => self.open_quality_menu(),
            Action::Left => self.signals_open = self.signals.is_some(),
        }
    }

    /// Opens the quality menu on the quality in use, when the live stream
    /// offers heights to choose from.
    fn open_quality_menu(&mut self) {
        let Some(stats) = self.session.stats() else { return };
        if !stats.live.heights.is_empty() {
            self.quality_menu = Some(QualityMenu::new(&stats.live.heights, stats.quality));
        }
    }

    /// The loader until the first frame, the panel, when shown, and the
    /// quality menu, when open.
    pub(super) fn draw_over_video(&mut self, r: &mut Renderer, t: &mut Text, s: f32, surface: (i32, i32), language: Language) {
        let Some(stats) = self.session.stats() else { return };
        if loader::waiting(&stats) {
            loader::draw(r, s, surface, self.info.watched());
        }
        self.info.draw(r, t, s, &stats, language);
        if let Some(menu) = &self.quality_menu
            && !stats.live.heights.is_empty()
        {
            menu.draw(r, t, s, &stats, &mut self.hits, language);
        }
    }
}
