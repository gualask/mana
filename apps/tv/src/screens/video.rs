//! Over the video: the keys of the channel list, the quality menu and the
//! video itself, and the drawing of the panel and the menu.

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
            Action::Select => match self.play_focused_channel() {
                Ok(()) => self.channels_open = false,
                Err(reason) => self.channels.notice(reason),
            },
            Action::Back => self.channels_open = false,
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
                log!("riproduzione fermata");
            }
            Action::Select => self.channels_open = true,
            Action::Up | Action::Down => self.info.toggle(),
            Action::Right => self.open_quality_menu(),
            Action::Left => {}
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
    pub(super) fn draw_over_video(&mut self, r: &mut Renderer, t: &mut Text, s: f32, surface: (i32, i32)) {
        let Some(stats) = self.session.stats() else { return };
        if loader::waiting(&stats) {
            loader::draw(r, s, surface, self.info.watched());
        }
        self.info.draw(r, t, s, &stats);
        if let Some(menu) = &self.quality_menu
            && !stats.live.heights.is_empty()
        {
            menu.draw(r, t, s, &stats.live.heights, stats.quality, &mut self.hits);
        }
    }
}
