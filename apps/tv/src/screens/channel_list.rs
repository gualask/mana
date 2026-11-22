//! The channel list drawn over the playing video, the same channels as the
//! home screen, moved with the remote while playback goes on; thousands of
//! made-up ones for the interface test.

use mana_core::catalog::Channel;

use super::gfx::{Color, Renderer};
use super::hits::{Hits, Target};
use super::logos::Logos;
use super::text::Text;
use super::{BACKGROUND as PANEL, ERROR, FOCUS, MUTED, TEXT};

const VISIBLE_ROWS: usize = 12;
/// Rows moved by Left/Right.
pub const PAGE: isize = 10;
/// Rows beyond the visible ones whose logos are fetched ahead, each way.
const PREFETCH_ROWS: usize = 2 * PAGE as usize;
const GENRES: [&str; 6] = ["Notizie", "Sport", "Cinema", "Musica", "Ragazzi", "Documentari"];

const LOGO_BOX: Color = Color::hex(0x252b3b);
/// A logo's box in a row, in 1080p pixels.
pub const LOGO_SIZE: (f32, f32) = (92.0, 48.0);
/// Why a made-up channel does not play.
const SYNTHETIC: &str = "canale finto della prova dell'interfaccia";

pub struct ChannelList {
    channels: Vec<Channel>,
    focus: usize,
    /// The first visible row: the keys keep the focus in the middle, the
    /// pointer moves it within the rows shown, which stay put under it.
    first: usize,
    /// Why the focused channel did not play, until the focus moves.
    notice: Option<String>,
}

impl ChannelList {
    /// `count` made-up channels, without logos.
    pub fn synthetic(count: usize) -> Self {
        let channels = (0..count)
            .map(|i| Channel {
                name: format!("{:04}  {} {} HD", i + 1, GENRES[i % GENRES.len()], i / 6 + 1),
                logo: None,
                group: None,
                play: Err(SYNTHETIC.into()),
            })
            .collect();
        Self::new(channels)
    }

    pub fn new(channels: Vec<Channel>) -> Self {
        Self { channels, focus: 0, first: 0, notice: None }
    }

    /// Shows `channels` instead, the focus kept on the channel of the same
    /// name if it is still there, else on the first.
    pub fn replace(&mut self, channels: Vec<Channel>) {
        let focused = self.focused().map(|c| c.name.clone());
        self.channels = channels;
        self.notice = None;
        self.set_focus(focused.and_then(|name| self.position(&name)).unwrap_or(0));
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Shows why the focused channel does not play, until the focus moves.
    pub fn notice(&mut self, reason: String) {
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
        self.channels.iter().position(|c| c.name == name)
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
    /// once asked, shows under the rows.
    pub fn draw(&self, r: &mut Renderer, t: &mut Text, logos: &mut Logos, s: f32, hits: &mut Hits) {
        let (x, y, w, row) = (60.0 * s, 120.0 * s, 720.0 * s, 64.0 * s);
        let notice_height = if self.notice.is_some() { 56.0 } else { 0.0 };
        r.rect(x, y, w, (VISIBLE_ROWS as f32 * 64.0 + 120.0 + notice_height) * s, PANEL.with_alpha(0.85));
        let header = format!("Canali · {} di {}", self.focus + 1, self.channels.len());
        t.draw(r, x + 30.0 * s, y + 24.0 * s, 32.0 * s, MUTED, &header);
        let first = self.first;
        let with_logos = self.channels.iter().any(|c| c.logo.is_some());
        let text_x = if with_logos { x + 136.0 * s } else { x + 36.0 * s };
        for (i, channel) in self.channels.iter().enumerate().skip(first).take(VISIBLE_ROWS) {
            let top = y + 90.0 * s + (i - first) as f32 * row;
            // The row's box: the focus colours it, the pointer finds it.
            let (bx, bw, bh) = (x + 16.0 * s, w - 32.0 * s, row - 6.0 * s);
            hits.add(bx, top, bw, bh, Target::Channel(i));
            if i == self.focus {
                r.rect(bx, top, bw, bh, FOCUS);
            }
            if let Some(logo) = &channel.logo {
                draw_logo(r, logos, logo, x + 28.0 * s, top + 5.0 * s, s);
            }
            let colour = if channel.play.is_ok() { TEXT } else { MUTED };
            t.draw(r, text_x, top + 12.0 * s, 32.0 * s, colour, &channel.name);
        }
        if let Some(notice) = &self.notice {
            let top = y + 90.0 * s + VISIBLE_ROWS as f32 * row + 10.0 * s;
            t.draw(r, x + 30.0 * s, top, 26.0 * s, ERROR, notice);
        }
        if with_logos {
            self.want_logos(logos);
        }
    }

    /// Asks for the logos of the visible rows, then of the rows around them.
    fn want_logos(&self, logos: &mut Logos) {
        let visible = self.first..(self.first + VISIBLE_ROWS).min(self.channels.len());
        let around = self.first.saturating_sub(PREFETCH_ROWS)..(visible.end + PREFETCH_ROWS).min(self.channels.len());
        let order = visible.clone().chain(around.filter(|i| !visible.contains(i)));
        logos.want(order.filter_map(|i| self.channels[i].logo.as_deref()));
    }
}

/// The logo of `url` in its box at `(x, y)`, or the empty box until it is
/// loaded.
pub fn draw_logo(r: &mut Renderer, logos: &mut Logos, url: &str, x: f32, y: f32, s: f32) {
    let (w, h) = (LOGO_SIZE.0 * s, LOGO_SIZE.1 * s);
    if !logos.draw(r, url, x, y, w, h) {
        r.rect(x, y, w, h, LOGO_BOX);
    }
}
