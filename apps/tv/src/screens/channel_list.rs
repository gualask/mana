//! The channel list drawn over the playing video, the same channels as the
//! home screen, moved with the remote while playback goes on; thousands of
//! made-up ones for the interface test.

use std::borrow::Cow;

use mana_core::catalog::logos::{self, Placeholder, room_for_logos};
use mana_core::catalog::{Channel, ChannelError, Play};
use mana_core::language::{Language, Localized};
use mana_core::text;

use super::gfx::{Color, Renderer};
use super::hits::{Hits, Target};
use super::logos::Logos;
use super::text::Text;
use super::{BACKGROUND as PANEL, ERROR, FOCUS, MUTED, OK, TEXT};
use mana_core::clock;

const VISIBLE_ROWS: usize = 12;
/// Rows moved by Left/Right.
pub const PAGE: isize = 10;
/// Rows beyond the visible ones whose logos are fetched ahead, each way.
const PREFETCH_ROWS: usize = 2 * PAGE as usize;
/// Made-up genres, as data: not translated.
const GENRES: [&str; 6] = ["News", "Sport", "Cinema", "Music", "Kids", "Documentaries"];

const LOGO_BOX: Color = Color::hex(0x252b3b);
/// A logo's box in a row, in 1080p pixels.
pub const LOGO_SIZE: (f32, f32) = logos::BOX;

pub struct ChannelList {
    channels: Vec<Channel>,
    focus: usize,
    /// The first visible row: the keys keep the focus in the middle, the
    /// pointer moves it within the rows shown, which stay put under it.
    first: usize,
    /// Why the focused channel did not play, until the focus moves.
    notice: Option<ChannelError>,
    /// The name of the folder these are the events of, if they are.
    folder: Option<String>,
    /// What the screens say about the folder, under the rows.
    status: Option<String>,
}

impl ChannelList {
    /// `count` made-up channels, without logos or a stream.
    pub fn synthetic(count: usize) -> Self {
        let channels = (0..count)
            .map(|i| Channel {
                name: Some(format!("{:04}  {} {} HD", i + 1, GENRES[i % GENRES.len()], i / 6 + 1)),
                logo: None,
                group: None,
                starts_at: None,
                play: Err(ChannelError::NoStream),
            })
            .collect();
        Self::new(channels)
    }

    pub fn new(channels: Vec<Channel>) -> Self {
        Self { channels, focus: 0, first: 0, notice: None, folder: None, status: None }
    }

    /// The events of the folder `name`.
    pub fn folder(name: &str, channels: Vec<Channel>) -> Self {
        Self { folder: Some(name.to_string()), ..Self::new(channels) }
    }

    /// Says `status` under the rows, while no notice is shown.
    pub fn set_status(&mut self, status: Option<String>) {
        self.status = status;
    }

    /// Shows `channels` instead, the focus kept on the channel of the same
    /// name if it is still there, else on the first. Returns where the
    /// channel at `row` is now, by the same rule, for a focus kept
    /// elsewhere (the home screen's); None when `row` was not a channel.
    pub fn replace(&mut self, channels: Vec<Channel>, row: usize) -> Option<usize> {
        let name = |list: &Self, i: usize| list.channels.get(i).map(|c| c.name.clone());
        let (focused, followed) = (name(self, self.focus), name(self, row));
        self.channels = channels;
        self.notice = None;
        let now = |name: Option<String>| name.and_then(|name| self.position(&name)).unwrap_or(0);
        let (followed, focus) = (followed.map(now), now(focused.flatten()));
        self.set_focus(focus);
        followed
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    pub fn into_channels(self) -> Vec<Channel> {
        self.channels
    }

    /// Shows why the focused channel does not play, until the focus moves.
    pub fn notice(&mut self, reason: ChannelError) {
        self.notice = Some(reason);
    }

    pub fn len(&self) -> usize {
        self.channels.len()
    }

    pub fn set_focus(&mut self, index: usize) {
        self.focus = index.min(self.channels.len().saturating_sub(1));
        self.centre();
    }

    /// Moves the focus to the visible row `index`, under the pointer,
    /// without scrolling.
    pub fn point(&mut self, index: usize) {
        if (self.first..self.first + VISIBLE_ROWS).contains(&index) && index < self.channels.len() && index != self.focus {
            self.focus = index;
            self.notice = None;
        }
    }

    pub fn focused(&self) -> Option<&Channel> {
        self.channels.get(self.focus)
    }

    /// Where the channel named `name` is in the list.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.channels.iter().position(|c| c.name.as_deref() == Some(name))
    }

    pub fn with_logos(&self) -> usize {
        self.channels.iter().filter(|c| c.logo.is_some()).count()
    }

    /// Moves the focus by `delta` rows, stopping at either end.
    pub fn step(&mut self, delta: isize) {
        let last = self.channels.len().saturating_sub(1) as isize;
        self.focus = (self.focus as isize + delta).clamp(0, last) as usize;
        self.notice = None;
        self.centre();
    }

    /// Scrolls the focus to the middle of the visible rows.
    fn centre(&mut self) {
        self.first = self.focus.saturating_sub(VISIBLE_ROWS / 2).min(self.channels.len().saturating_sub(VISIBLE_ROWS));
    }

    /// Draws the list as a translucent panel on the left of the screen; `s`
    /// scales the 1080p layout to the surface. Asks for the logos of the
    /// visible rows, then of the rows around them; records the rows in `hits`.
    /// A channel that cannot be played is dimmed; the reason it did not,
    /// once asked, shows under the rows, else the folder's status. The
    /// header names the folder, if these are its events.
    pub fn draw(&self, r: &mut Renderer, t: &mut Text, logos: &mut Logos, s: f32, hits: &mut Hits, language: Language) {
        let (x, y, w, row) = (60.0 * s, 120.0 * s, 720.0 * s, 64.0 * s);
        let below = self.below(language);
        let below_height = if below.is_some() { 56.0 } else { 0.0 };
        r.rect(x, y, w, (VISIBLE_ROWS as f32 * 64.0 + 120.0 + below_height) * s, PANEL.with_alpha(0.85));
        t.draw(r, x + 30.0 * s, y + 24.0 * s, 32.0 * s, MUTED, &self.header(language));
        let (first, now) = (self.first, clock::now());
        let with_logos = room_for_logos(&self.channels);
        let text_x = if with_logos { x + 136.0 * s } else { x + 36.0 * s };
        for (i, channel) in self.channels.iter().enumerate().skip(first).take(VISIBLE_ROWS) {
            let top = y + 90.0 * s + (i - first) as f32 * row;
            // The row's box: the focus colours it, the pointer finds it.
            let (bx, bw, bh) = (x + 16.0 * s, w - 32.0 * s, row - 6.0 * s);
            hits.add(bx, top, bw, bh, Target::Channel(i));
            if i == self.focus {
                r.rect(bx, top, bw, bh, FOCUS);
            }
            if with_logos {
                draw_logo(r, t, logos, channel, x + 28.0 * s, top + 5.0 * s, s, language);
            }
            let colour = if channel.play.is_ok() { TEXT } else { MUTED };
            let title_x = match starts(channel, now) {
                Some(time) => text_x + t.draw(r, text_x, top + 12.0 * s, 32.0 * s, time_colour(channel, now), &time) + 20.0 * s,
                None => text_x,
            };
            t.draw(r, title_x, top + 12.0 * s, 32.0 * s, colour, &label(channel, language));
        }
        if let Some((text, colour)) = below {
            let top = y + 90.0 * s + VISIBLE_ROWS as f32 * row + 10.0 * s;
            t.draw(r, x + 30.0 * s, top, 26.0 * s, colour, &text);
        }
        if with_logos {
            self.want_logos(logos);
        }
    }

    /// The header: the folder's name, if these are its events, and where
    /// the focus is.
    fn header(&self, language: Language) -> String {
        let (focus, count) = (self.focus + 1, self.channels.len());
        match &self.folder {
            Some(folder) => text!(language, "channels-folder-header", folder = folder, focus = focus, count = count),
            None => text!(language, "channels-header", focus = focus, count = count),
        }
    }

    /// What shows under the rows, with its colour: why the focused channel
    /// did not play, else the folder's status.
    fn below(&self, language: Language) -> Option<(String, Color)> {
        let notice = self.notice.as_ref().map(|notice| (notice.text(language), ERROR));
        notice.or_else(|| self.status.clone().map(|status| (status, MUTED)))
    }

    /// Asks for the logos of the visible rows, then of the rows around them.
    fn want_logos(&self, logos: &mut Logos) {
        let visible = self.first..(self.first + VISIBLE_ROWS).min(self.channels.len());
        let around = self.first.saturating_sub(PREFETCH_ROWS)..(visible.end + PREFETCH_ROWS).min(self.channels.len());
        let order = visible.clone().chain(around.filter(|i| !visible.contains(i)));
        logos.want(order.filter_map(|i| self.channels[i].logo.as_deref()));
    }
}

/// The logo of `channel` in its box at `(x, y)`; while it has none, or it
/// is not loaded, the box with a folder for a folder, else the channel's
/// initial, so that every row shows one.
#[allow(clippy::too_many_arguments)]
pub fn draw_logo(r: &mut Renderer, t: &mut Text, logos: &mut Logos, channel: &Channel, x: f32, y: f32, s: f32, language: Language) {
    let (w, h) = (LOGO_SIZE.0 * s, LOGO_SIZE.1 * s);
    if channel.logo.as_deref().is_some_and(|url| logos.draw(r, url, x, y, w, h)) {
        return;
    }
    r.rect(x, y, w, h, LOGO_BOX);
    match channel.placeholder(language) {
        Placeholder::Folder => draw_folder(r, x + w / 2.0, y + h / 2.0, s),
        Placeholder::Initial(initial) => {
            let px = 28.0 * s;
            let width = t.width(r, px, &initial);
            t.draw(r, x + (w - width) / 2.0, y + (h - px) / 2.0, px, MUTED, &initial);
        }
    }
}

/// A folder centred on `(cx, cy)`: its tab, then its body, in 1080p pixels
/// scaled by `s`.
fn draw_folder(r: &mut Renderer, cx: f32, cy: f32, s: f32) {
    let (w, h) = (34.0 * s, 22.0 * s);
    let (x, y) = (cx - w / 2.0, cy - h / 2.0 + 2.0 * s);
    r.rect(x, y - 5.0 * s, 14.0 * s, 6.0 * s, MUTED);
    r.rect(x, y, w, h, MUTED);
}

/// The name a row shows: the channel's title, with a mark when it is a
/// folder.
pub(super) fn label(channel: &Channel, language: Language) -> Cow<'_, str> {
    match &channel.play {
        Ok(Play::Folder(_)) => Cow::Owned(format!("{}  ›", channel.title(language))),
        _ => channel.title(language),
    }
}

/// When an event starts, in the TV's local time: `20:30` today, else with
/// the day, `24/11 20:30`; None for a channel.
pub(super) fn starts(channel: &Channel, now: i64) -> Option<String> {
    Some(clock::time(channel.starts_at?, now))
}

/// The colour of an event's time: that of a live stream once it started.
pub(super) fn time_colour(channel: &Channel, now: i64) -> Color {
    if channel.starts_at.is_some_and(|at| at <= now) { OK } else { MUTED }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[&str]) -> Vec<Channel> {
        let channel = |name: &&str| Channel { name: Some(name.to_string()), logo: None, group: None, starts_at: None, play: Err(ChannelError::NoStream) };
        names.iter().map(channel).collect()
    }

    #[test]
    fn a_replacement_keeps_the_focus_and_a_row_followed_on_the_same_names() {
        let mut list = ChannelList::new(named(&["A", "B", "C"]));
        list.set_focus(2);
        assert_eq!(list.replace(named(&["C", "X", "B"]), 1), Some(2), "B, followed");
        assert_eq!(list.focused().and_then(|c| c.name.as_deref()), Some("C"));
        assert_eq!(list.replace(named(&["Y"]), 0), Some(0), "gone: the first");
        assert_eq!(list.replace(named(&["Y"]), 5), None, "not a channel");
    }
}
