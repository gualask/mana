//! The search: an on-screen keyboard, used with the arrows or the pointer,
//! and what it finds as the text grows, among the list's channels and the
//! events of the folders' saved copies, read when the search opens: no
//! request goes to the network.

use mana_core::catalog::search::search;
use mana_core::catalog::{Channel, Play, by_start};
use mana_core::language::{Language, Localized};
use mana_core::text;

use super::channel_list::{self, time_colour};
use super::gfx::Renderer;
use super::hits::{Hits, Target};
use super::text::Text;
use super::App;
use super::{ERROR, FOCUS, MUTED, ROW, TEXT};
use mana_core::clock;
use crate::platform::keys::Action;

/// The keys that type, in the keyboard's order; then space and delete.
const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const SPACE: usize = 36;
const DELETE: usize = 37;
const COLUMNS: usize = 7;
/// The last row: the last digit, then space and delete, three columns each.
const LAST_ROW: usize = 5;
/// Results shown at once; the list scrolls past them.
const RESULT_ROWS: usize = 11;
/// Characters of a result's title shown, so that the name of its source
/// fits after it.
const TITLE_CHARACTERS: usize = 44;

/// What the search asks the app to do.
pub(super) enum Done {
    Close,
    Play(Box<Channel>),
}

pub(super) struct Search {
    query: String,
    /// What the search looks in.
    pool: Vec<Channel>,
    found: Vec<Channel>,
    key: usize,
    /// The focus is on the results, not on the keyboard.
    on_results: bool,
    result: usize,
    /// The first result shown.
    first: usize,
    /// Why the result chosen does not play, until the focus moves.
    notice: Option<String>,
}

impl Search {
    /// A search among `pool`.
    pub(super) fn new(pool: Vec<Channel>) -> Self {
        Self { query: String::new(), pool, found: Vec::new(), key: 0, on_results: false, result: 0, first: 0, notice: None }
    }

    /// The arrows move on the keyboard, and from its right edge to the
    /// results and back; OK types, or plays the result; Back closes.
    pub(super) fn handle(&mut self, action: Action) -> Option<Done> {
        self.notice = None;
        match action {
            Action::Back => return Some(Done::Close),
            Action::Select if self.on_results => return self.found.get(self.result).cloned().map(|channel| Done::Play(Box::new(channel))),
            Action::Select => self.press(self.key),
            Action::Up | Action::Down if self.on_results => {
                let delta: isize = if matches!(action, Action::Up) { -1 } else { 1 };
                self.point_result((self.result as isize + delta).max(0) as usize);
            }
            Action::Left if self.on_results => self.on_results = false,
            Action::Right if self.on_results => {}
            Action::Up => self.move_key(0, -1),
            Action::Down => self.move_key(0, 1),
            Action::Left => self.move_key(-1, 0),
            Action::Right => self.move_key(1, 0),
        }
        None
    }

    /// Says why the result chosen does not play.
    pub(super) fn notice(&mut self, reason: String) {
        self.notice = Some(reason);
    }

    /// The key `index` takes the focus, under the pointer.
    pub(super) fn point_key(&mut self, index: usize) {
        self.on_results = false;
        self.key = index.min(DELETE);
    }

    /// The result `index` takes the focus, the list scrolled to show it.
    pub(super) fn point_result(&mut self, index: usize) {
        if self.found.is_empty() {
            return;
        }
        self.on_results = true;
        self.result = index.min(self.found.len() - 1);
        if self.result < self.first {
            self.first = self.result;
        } else if self.result >= self.first + RESULT_ROWS {
            self.first = self.result + 1 - RESULT_ROWS;
        }
    }

    fn press(&mut self, key: usize) {
        match key {
            DELETE => drop(self.query.pop()),
            SPACE if self.query.is_empty() || self.query.ends_with(' ') => {}
            SPACE => self.query.push(' '),
            letter => self.query.extend(LETTERS.chars().nth(letter)),
        }
        self.found = search(&self.pool, &self.query);
        (self.result, self.first) = (0, 0);
    }

    /// Moves the focus on the keyboard by `dx` columns or `dy` rows; past
    /// the right edge, to the results.
    fn move_key(&mut self, dx: isize, dy: isize) {
        let (column, row, span) = place(self.key);
        let column = if dx > 0 { column as isize + span as isize } else { column as isize + dx };
        let row = row as isize + dy;
        if column >= COLUMNS as isize && !self.found.is_empty() {
            self.point_result(self.result);
            return;
        }
        if let Some(key) = key_at(column, row) {
            self.key = key;
        }
    }

    /// Draws the search on the whole screen; `s` scales the 1080p layout.
    /// Records the keys and the results in `hits`.
    pub(super) fn draw(&self, r: &mut Renderer, t: &mut Text, s: f32, hits: &mut Hits, language: Language) {
        t.draw(r, 120.0 * s, 70.0 * s, 56.0 * s, TEXT, &text!(language, "search-title"));
        r.rect(120.0 * s, 160.0 * s, 776.0 * s, 72.0 * s, ROW);
        t.draw(r, 140.0 * s, 172.0 * s, 40.0 * s, TEXT, &format!("{}_", self.query));
        for key in 0..=DELETE {
            let (column, row, span) = place(key);
            let (x, y) = ((120.0 + column as f32 * 112.0) * s, (260.0 + row as f32 * 96.0) * s);
            let (w, h) = ((span as f32 * 112.0 - 8.0) * s, 88.0 * s);
            r.rect(x, y, w, h, if key == self.key && !self.on_results { FOCUS } else { ROW });
            hits.add(x, y, w, h, Target::Key(key));
            let px = if span > 1 { 30.0 } else { 40.0 };
            t.draw(r, x + 24.0 * s, y + (88.0 - px * 1.3) / 2.0 * s, px * s, TEXT, &key_label(key, language));
        }
        self.draw_results(r, t, s, hits, language);
        let footer = text!(language, "search-footer");
        t.draw(r, 120.0 * s, 980.0 * s, 24.0 * s, MUTED, &footer);
    }

    fn draw_results(&self, r: &mut Renderer, t: &mut Text, s: f32, hits: &mut Hits, language: Language) {
        let (x, w, row) = (980.0 * s, 820.0 * s, 64.0 * s);
        t.draw(r, x, 176.0 * s, 28.0 * s, MUTED, &self.header(language));
        let now = clock::now();
        for (i, channel) in self.found.iter().enumerate().skip(self.first).take(RESULT_ROWS) {
            let top = (260.0 + (i - self.first) as f32 * 64.0) * s;
            let (h, focused) = (row - 6.0 * s, self.on_results && i == self.result);
            r.rect(x, top, w, h, if focused { FOCUS } else { ROW });
            hits.add(x, top, w, h, Target::Found(i));
            let mut at = x + 20.0 * s;
            if let Some(time) = channel_list::starts(channel, now) {
                at += t.draw(r, at, top + 12.0 * s, 30.0 * s, time_colour(channel, now), &time) + 16.0 * s;
            }
            let title: String = channel.title(language).chars().take(TITLE_CHARACTERS).collect();
            let colour = if channel.play.is_ok() { TEXT } else { MUTED };
            at += t.draw(r, at, top + 12.0 * s, 30.0 * s, colour, &title);
            if let Ok(Play::Source(selection)) = &channel.play {
                t.draw(r, at + 16.0 * s, top + 16.0 * s, 24.0 * s, MUTED, &selection.source.name);
            }
        }
        if let Some(notice) = &self.notice {
            t.draw(r, x, (260.0 + RESULT_ROWS as f32 * 64.0 + 8.0) * s, 26.0 * s, ERROR, notice);
        }
    }

    /// Above the results: how many were found, or what to type.
    fn header(&self, language: Language) -> String {
        if self.query.trim().is_empty() {
            text!(language, "search-hint")
        } else {
            text!(language, "search-found", count = self.found.len())
        }
    }
}

impl App {
    /// Opens the search among the list's channels and the events of the
    /// folders' saved copies: the channels first, then the events by time.
    pub(super) fn open_search(&mut self) {
        let list = self.list_channels();
        let mut pool: Vec<Channel> = list.iter().filter(|c| !matches!(c.play, Ok(Play::Folder(_)))).cloned().collect();
        let saved = list.iter().filter_map(|c| match &c.play {
            Ok(Play::Folder(folder)) => self.folders.saved(folder),
            _ => None,
        });
        let mut events: Vec<Channel> = saved.flatten().collect();
        by_start(&mut events);
        pool.extend(events);
        self.search = Some(Search::new(pool));
    }

    /// Sends a key to the search: it closes, or plays a result, saying why
    /// one does not play.
    pub(super) fn handle_search(&mut self, action: Action) {
        let Some(search) = &mut self.search else { return };
        let channel = match search.handle(action) {
            Some(Done::Close) => {
                self.search = None;
                return;
            }
            Some(Done::Play(channel)) => channel,
            None => return,
        };
        let Err(reason) = self.play(*channel) else { return };
        let notice = reason.text(self.languages.current());
        if let Some(search) = &mut self.search {
            search.notice(notice);
        }
    }
}

/// What `key` shows.
fn key_label(key: usize, language: Language) -> String {
    match key {
        SPACE => text!(language, "search-space"),
        DELETE => text!(language, "search-delete"),
        letter => LETTERS.chars().nth(letter).map(String::from).unwrap_or_default(),
    }
}

/// Where `key` is: its column, its row and how many columns it takes.
fn place(key: usize) -> (usize, usize, usize) {
    match key {
        SPACE => (1, LAST_ROW, 3),
        DELETE => (4, LAST_ROW, 3),
        letter => (letter % COLUMNS, letter / COLUMNS, 1),
    }
}

/// The key at `column` and `row`, if there is one.
fn key_at(column: isize, row: isize) -> Option<usize> {
    let (column, row) = (usize::try_from(column).ok()?, usize::try_from(row).ok()?);
    match (column, row) {
        (COLUMNS.., _) | (_, 6..) => None,
        (1..=3, LAST_ROW) => Some(SPACE),
        (4.., LAST_ROW) => Some(DELETE),
        (column, row) => Some(row * COLUMNS + column),
    }
}

#[cfg(test)]
mod tests {
    use mana_core::catalog::ChannelError;

    use super::*;

    fn pool() -> Vec<Channel> {
        let channel = |name: &str| Channel { name: Some(name.into()), logo: None, group: None, starts_at: None, play: Err(ChannelError::NoStream) };
        vec![channel("Monza - Fiorentina"), channel("Fiorentina U19 - Roma U19"), channel("Rai 1")]
    }

    fn typed(search: &mut Search, keys: &str) {
        for c in keys.chars() {
            let key = if c == ' ' { SPACE } else { LETTERS.find(c).unwrap() };
            search.point_key(key);
            search.handle(Action::Select);
        }
    }

    #[test]
    fn typing_narrows_the_results_and_delete_widens_them() {
        let mut search = Search::new(pool());
        typed(&mut search, "FIOR");
        assert_eq!(search.found.len(), 2);
        typed(&mut search, " ROMA");
        assert_eq!(search.found.len(), 1);
        for _ in 0..5 {
            search.point_key(DELETE);
            search.handle(Action::Select);
        }
        assert_eq!((search.query.as_str(), search.found.len()), ("FIOR", 2));
    }

    #[test]
    fn the_arrows_cross_the_keyboard_and_reach_the_results_from_its_right_edge() {
        let mut search = Search::new(pool());
        typed(&mut search, "RAI");
        search.point_key(0);
        search.handle(Action::Up);
        search.handle(Action::Left);
        assert_eq!(search.key, 0, "stops at the edges");
        (0..5).for_each(|_| drop(search.handle(Action::Down)));
        assert_eq!(search.key, 35, "the last digit");
        search.handle(Action::Right);
        assert_eq!(search.key, SPACE);
        search.handle(Action::Right);
        assert_eq!(search.key, DELETE);
        search.handle(Action::Up);
        assert_eq!(search.key, 32, "above delete's first column");
        search.point_key(DELETE);
        search.handle(Action::Right);
        assert!(search.on_results);
        assert!(matches!(search.handle(Action::Select), Some(Done::Play(channel)) if channel.name.as_deref() == Some("Rai 1")));
        search.handle(Action::Left);
        assert!(!search.on_results);
        assert!(matches!(search.handle(Action::Back), Some(Done::Close)));
    }
}
