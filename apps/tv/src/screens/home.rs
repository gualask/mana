//! The screen the app opens on: the channels of the list, then the local
//! test clips and `.stream` files; its keys, its drawing, and the clips it
//! offers.

use std::path::{Path, PathBuf};

use mana_core::language::{Language, Localized};
use mana_core::session::{Program, STREAM_EXTENSION};
use mana_core::text;

use super::channel_list::{self, LOGO_SIZE, draw_logo};
use super::gfx::Renderer;
use super::hits::Target;
use super::list_updates::ListState;
use super::text::Text;
use super::{App, BACKGROUND, ERROR, FOCUS, MUTED, Outcome, ROW, TEXT};
use crate::platform::{DATA_DIR, clock};
use crate::platform::keys::Action;

/// Test clips are copied here over SSH (see dev/tv/clips.py).
pub const CLIPS_DIR: &str = "/tmp/mana-clips";
/// Rows shown at once: they end above the message and the help line.
const VISIBLE_ROWS: usize = 6;
/// Local MPEG-TS clips and `.stream` files describing a live HLS stream.
const PLAYABLE_EXTENSIONS: &[&str] = &["ts", STREAM_EXTENSION];

/// The name a clip is shown and played as: its file name without the
/// extension.
pub fn clip_name(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The playable files of the tests (CLIPS_DIR) and those saved on the TV
/// (DATA_DIR), sorted by name.
pub fn clips() -> Vec<PathBuf> {
    let mut clips: Vec<PathBuf> = [Path::new(CLIPS_DIR), Path::new(DATA_DIR)].into_iter().flat_map(playable).collect();
    clips.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    clips
}

/// The playable files in `dir`.
fn playable(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| PLAYABLE_EXTENSIONS.iter().any(|e| ext == *e)))
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    /// The rows: the channels, then the clips.
    fn rows(&self) -> usize {
        self.channels.len() + self.clips.len()
    }

    pub(super) fn handle_home(&mut self, action: Action) -> Outcome {
        match action {
            // The rows stay where they are while «Settings» has the focus.
            Action::Up | Action::Down if self.settings_focused => {}
            Action::Up => self.focus = self.focus.saturating_sub(1),
            Action::Down => self.focus = (self.focus + 1).min(self.rows().saturating_sub(1)),
            Action::Right => self.settings_focused = true,
            Action::Left => self.settings_focused = false,
            Action::Select if self.settings_focused => self.open_setup(),
            Action::Select => self.play_focused(),
            Action::Back => return Outcome::Exit,
        }
        Outcome::Continue
    }

    /// Plays the focused row: a channel, or a clip. Without rows, looks for
    /// clips again.
    pub(super) fn play_focused(&mut self) {
        let channels = self.channels.len();
        if self.focus < channels {
            self.play_channel(self.focus);
            return;
        }
        let Some(path) = self.clips.get(self.focus - channels).cloned() else {
            self.clips = clips();
            return;
        };
        self.start(clip_name(&path), Program::Path(path));
    }

    /// What the home screen says about the list, while it has no channels.
    fn list_status(&self, language: Language) -> Option<String> {
        match self.list_state() {
            ListState::Downloading => Some(text!(language, "home-list-downloading")),
            ListState::Unavailable(reason) => Some(text!(language, "home-list-unavailable", reason = reason.text(language))),
            ListState::NotSet => Some(text!(language, "home-list-not-set")),
            ListState::Tests(_) | ListState::Channels(_) | ListState::Empty => None,
        }
    }

    /// Scrolls the rows shown just enough to show the focus.
    fn scroll_to_focus(&mut self) {
        if self.focus < self.home_first {
            self.home_first = self.focus;
        } else if self.focus >= self.home_first + VISIBLE_ROWS {
            self.home_first = self.focus + 1 - VISIBLE_ROWS;
        }
        self.home_first = self.home_first.min(self.rows().saturating_sub(VISIBLE_ROWS));
    }

    pub(super) fn draw_home(&mut self, r: &mut Renderer, t: &mut Text, s: f32, surface: (i32, i32), language: Language) {
        r.begin(surface.0, surface.1, BACKGROUND);
        self.logos.upload(r);
        t.draw(r, 120.0 * s, 90.0 * s, 72.0 * s, TEXT, "Mana");
        let subtitle = text!(language, "home-subtitle");
        t.draw(r, 120.0 * s, 180.0 * s, 32.0 * s, MUTED, &subtitle);
        self.draw_settings_button(r, t, s, language);

        if let Some(status) = self.list_status(language) {
            t.draw(r, 120.0 * s, 226.0 * s, 24.0 * s, MUTED, &status);
        } else if self.rows() > VISIBLE_ROWS {
            let (focus, rows) = (self.focus + 1, self.rows());
            t.draw(r, 120.0 * s, 226.0 * s, 24.0 * s, MUTED, &text!(language, "home-position", focus = focus, count = rows));
        }
        if self.rows() == 0 {
            let empty = text!(language, "home-empty", clips = CLIPS_DIR, data = DATA_DIR);
            t.draw(r, 120.0 * s, 280.0 * s, 32.0 * s, MUTED, &empty);
        }
        self.scroll_to_focus();
        self.draw_rows(r, t, s, language);
        if let Some(message) = &self.message {
            t.draw(r, 120.0 * s, 900.0 * s, 28.0 * s, ERROR, &message.text(language));
        }
        let footer = text!(language, "home-footer");
        t.draw(r, 120.0 * s, 980.0 * s, 24.0 * s, MUTED, &footer);
    }

    /// «Settings», at the top right.
    fn draw_settings_button(&mut self, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        let (x, y, w, h) = (1460.0 * s, 96.0 * s, 340.0 * s, 72.0 * s);
        r.rect(x, y, w, h, if self.settings_focused { FOCUS } else { ROW });
        self.hits.add(x, y, w, h, Target::Settings);
        t.draw(r, x + 36.0 * s, y + 16.0 * s, 32.0 * s, TEXT, &text!(language, "settings"));
    }

    /// The rows shown: a channel with its logo, dimmed when it cannot be
    /// played, or a clip. Asks for the logos of the channels shown.
    fn draw_rows(&mut self, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        let channels = self.channels.channels();
        let with_logos = channels.iter().any(|c| c.logo.is_some());
        let (first, now) = (self.home_first, clock::now());
        let shown = first..(first + VISIBLE_ROWS).min(self.rows());
        for i in shown.clone() {
            let y = (260.0 + (i - first) as f32 * 96.0) * s;
            let (x, w, h) = (120.0 * s, 900.0 * s, 84.0 * s);
            r.rect(x, y, w, h, if i == self.focus && !self.settings_focused { FOCUS } else { ROW });
            self.hits.add(x, y, w, h, Target::Home(i));
            let Some(channel) = channels.get(i) else {
                t.draw(r, 156.0 * s, y + 20.0 * s, 36.0 * s, TEXT, &clip_name(&self.clips[i - channels.len()]));
                continue;
            };
            if let Some(logo) = &channel.logo {
                draw_logo(r, &mut self.logos, logo, 140.0 * s, y + (84.0 - LOGO_SIZE.1) / 2.0 * s, s);
            }
            let text_x = if with_logos { 260.0 * s } else { 156.0 * s };
            let colour = if channel.play.is_ok() { TEXT } else { MUTED };
            let title_x = match channel_list::starts(channel, now) {
                Some(time) => text_x + t.draw(r, text_x, y + 20.0 * s, 36.0 * s, MUTED, &time) + 20.0 * s,
                None => text_x,
            };
            t.draw(r, title_x, y + 20.0 * s, 36.0 * s, colour, &channel.title(language));
        }
        let logos = shown.filter_map(|i| channels.get(i)?.logo.as_deref());
        self.logos.want(logos);
    }
}
