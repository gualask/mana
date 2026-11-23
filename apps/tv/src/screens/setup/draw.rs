//! The drawing of the settings screen: the list's URL and state, the rows,
//! and the page's QR code.

use mana_core::catalog::update::Updater;
use mana_core::language::{Language, Localized};
use mana_core::text;
use url::Url;

use super::super::gfx::Renderer;
use super::super::hits::Target;
use super::super::list_updates::ListState;
use super::super::text::Text;
use super::super::{App, BACKGROUND, ERROR, FOCUS, MUTED, OK, ROW, TEXT};
use super::{Row, Setup};
use crate::settings::LIST_KEY;

/// Longest texts shown, in characters: longer ones are shortened in the
/// middle, so that they stay left of the QR code.
const MAX_URL_CHARS: usize = 56;
const MAX_NOTICE_CHARS: usize = 64;

impl Setup {
    /// The page's QR code and address, or why there is none.
    fn draw_page(&self, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        match (&self.server, &self.qr) {
            (Ok(server), Some(qr)) => {
                let side = qr.draw(r, 1160.0 * s, 200.0 * s, 560.0 * s);
                let below = 200.0 * s + side;
                let scan = text!(language, "settings-scan");
                t.draw(r, 1160.0 * s, below + 30.0 * s, 26.0 * s, MUTED, &scan);
                t.draw(r, 1160.0 * s, below + 76.0 * s, 32.0 * s, TEXT, server.address());
            }
            (Ok(server), None) => {
                t.draw(r, 1160.0 * s, 240.0 * s, 32.0 * s, TEXT, server.address());
            }
            (Err(reason), _) => {
                t.draw(r, 1160.0 * s, 240.0 * s, 32.0 * s, ERROR, &reason.text(language));
            }
        }
    }
}

impl App {
    /// The list's state, as the settings screen says it.
    fn list_summary(&self, language: Language) -> String {
        match self.list_state() {
            ListState::Tests(count) => text!(language, "settings-test-list", count = count),
            ListState::Channels(count) => text!(language, "settings-channels", count = count),
            ListState::Downloading => text!(language, "settings-list-downloading"),
            ListState::Unavailable(reason) => text!(language, "settings-list-unavailable", reason = reason.text(language)),
            ListState::NotSet | ListState::Empty => text!(language, "settings-no-channels"),
        }
    }

    pub(in crate::screens) fn draw_setup(&mut self, r: &mut Renderer, t: &mut Text, s: f32, surface: (i32, i32), language: Language) {
        r.begin(surface.0, surface.1, BACKGROUND);
        t.draw(r, 120.0 * s, 90.0 * s, 72.0 * s, TEXT, &text!(language, "settings"));
        self.draw_refresh_button(r, t, s, language);
        self.draw_language_row(r, t, s, language);
        let Some(setup) = &self.setup else { return };
        self.draw_list_state(setup, r, t, s, language);
        setup.draw_page(r, t, s, language);
        let footer = text!(language, "settings-footer");
        t.draw(r, 120.0 * s, 980.0 * s, 24.0 * s, MUTED, &footer);
    }

    /// «Update list», dimmed while a download is waited for.
    fn draw_refresh_button(&mut self, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        let Some(setup) = &self.setup else { return };
        let busy = setup.waiting.is_some();
        let focused = setup.focus == Row::Refresh;
        let (x, y, w, h) = (120.0 * s, Row::Refresh.top() * s, 560.0 * s, 72.0 * s);
        r.rect(x, y, w, h, if focused && !busy { FOCUS } else { ROW });
        self.hits.add(x, y, w, h, Target::Refresh);
        let label = if busy { text!(language, "settings-updating") } else { text!(language, "settings-update") };
        t.draw(r, x + 36.0 * s, y + 16.0 * s, 32.0 * s, if busy { MUTED } else { TEXT }, &label);
    }

    /// «Language» and the choice, which OK, left and right change.
    fn draw_language_row(&mut self, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        let Some(setup) = &self.setup else { return };
        let (x, y, w, h) = (120.0 * s, Row::Language.top() * s, 560.0 * s, 72.0 * s);
        r.rect(x, y, w, h, if setup.focus == Row::Language { FOCUS } else { ROW });
        self.hits.add(x, y, w, h, Target::Language);
        let label = format!("{}: ‹ {} ›", text!(language, "settings-language"), self.languages.choice().label(language));
        t.draw(r, x + 36.0 * s, y + 16.0 * s, 32.0 * s, TEXT, &label);
    }

    /// The list's URL and state, whether a settings file wins over the
    /// phone, and what became of the last URL sent.
    fn draw_list_state(&self, setup: &Setup, r: &mut Renderer, t: &mut Text, s: f32, language: Language) {
        t.draw(r, 120.0 * s, 240.0 * s, 40.0 * s, TEXT, &text!(language, "settings-channel-list"));
        let url = self.updater.as_ref().and_then(Updater::url).map(Url::as_str);
        let not_set = text!(language, "settings-list-not-set");
        let shown = url.unwrap_or(&not_set);
        t.draw(r, 120.0 * s, 310.0 * s, 28.0 * s, if url.is_some() { TEXT } else { MUTED }, &shortened(shown, MAX_URL_CHARS));
        t.draw(r, 120.0 * s, 360.0 * s, 28.0 * s, MUTED, &shortened(&self.list_summary(language), MAX_NOTICE_CHARS));
        if let Some(file) = &self.list_file {
            let text = text!(language, "settings-list-from-file", key = LIST_KEY, file = file.display().to_string());
            t.draw(r, 120.0 * s, 430.0 * s, 24.0 * s, ERROR, &shortened(&text, MAX_NOTICE_CHARS));
        }
        if let Some(notice) = &setup.notice {
            let colour = if notice.taken() { OK } else { ERROR };
            t.draw(r, 120.0 * s, 500.0 * s, 28.0 * s, colour, &shortened(&notice.text(language), MAX_NOTICE_CHARS));
        }
    }
}

/// `text` within `max` characters: its start and end around «…».
pub(super) fn shortened(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let end = max / 4;
    let start = max - end - 1;
    let head: String = text.chars().take(start).collect();
    let tail: String = text.chars().skip(count - end).collect();
    format!("{head}…{tail}")
}
