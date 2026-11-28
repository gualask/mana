//! The panel over the video: the information on what is playing, closed at
//! first, shown while a viewing starts or is paused and toggled with up or
//! down. It only reads the counters the viewing keeps anyway.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use mana_core::language::{Language, Localized};
use mana_core::playback::{AudioCodec, Formats, VideoCodec};
use mana_core::session::{Quality, State, Stats};
use mana_core::text;

use super::gfx::{Color, Renderer};
use super::text::Text;
use super::{BACKGROUND, ERROR, MUTED, TEXT, loader, quality_menu};

/// How long the information shows on its own once a viewing's first frame
/// shows.
const SHOWN_AT_START: Duration = Duration::from_secs(5);

pub struct InfoPanel {
    /// Opened with up or down, until closed the same way.
    open: bool,
    /// Shown on its own until then, after a viewing started: held while it
    /// waits for its first frame.
    until: Option<Instant>,
    /// When the viewing started.
    since: Instant,
}

impl Default for InfoPanel {
    fn default() -> Self {
        Self { open: false, until: None, since: Instant::now() }
    }
}

impl InfoPanel {
    /// A viewing starts: the information shows until its first frame and
    /// SHOWN_AT_START after.
    pub fn started(&mut self) {
        self.since = Instant::now();
        self.until = Some(self.since + SHOWN_AT_START);
    }

    /// Up or down: closes the panel shown, else opens it.
    pub fn toggle(&mut self) {
        let shown = self.shown();
        self.open = !shown;
        self.until = None;
    }

    /// How long the viewing has been going.
    pub fn watched(&self) -> Duration {
        self.since.elapsed()
    }

    fn shown(&self) -> bool {
        self.open || self.until.is_some_and(|until| Instant::now() < until)
    }

    /// A band across the bottom: the channel and, right, its state and the
    /// time watched; below, the formats and, right, the quality, each as a
    /// small grey label before its value.
    pub fn draw(&mut self, r: &mut Renderer, t: &mut Text, s: f32, stats: &Stats, language: Language) {
        self.hold_while_waiting(stats);
        if !self.shown() && stats.state != State::Paused {
            return;
        }
        let top = 928.0 * s;
        r.rect(40.0 * s, top, 1840.0 * s, 112.0 * s, BACKGROUND.with_alpha(0.8));
        draw_top_line(r, t, s, top, stats, self.since.elapsed(), language);
        draw_bottom_line(r, t, s, top + 68.0 * s, stats, language);
    }

    /// While the viewing waits for its first frame, the panel shown at its
    /// start stays, until SHOWN_AT_START after.
    fn hold_while_waiting(&mut self, stats: &Stats) {
        if let Some(until) = &mut self.until
            && loader::waiting(stats)
        {
            *until = (*until).max(Instant::now() + SHOWN_AT_START);
        }
    }
}

/// The band's edges, in design pixels.
const LEFT: f32 = 72.0;
const RIGHT: f32 = 1848.0;

/// The channel and, right, its state and the time watched, on the band's
/// top `top`.
fn draw_top_line(r: &mut Renderer, t: &mut Text, s: f32, top: f32, stats: &Stats, watched: Duration, language: Language) {
    t.draw(r, LEFT * s, top + 14.0 * s, 38.0 * s, TEXT, &stats.name);
    let (state, failed) = state(&stats.state, language);
    let watched_left = draw_right(r, t, RIGHT * s, top + 22.0 * s, 26.0 * s, TEXT, &clock(watched));
    draw_right(r, t, watched_left - GAP * s, top + 22.0 * s, 26.0 * s, if failed { ERROR } else { MUTED }, &state);
}

/// The formats and, right, the quality, on the line at `y`.
fn draw_bottom_line(r: &mut Renderer, t: &mut Text, s: f32, y: f32, stats: &Stats, language: Language) {
    let mut x = LEFT * s;
    for field in formats(stats.playback.formats.as_ref(), language) {
        x = field.draw(r, t, x, y, s) + GAP * s;
    }
    if let Some(field) = quality(stats, language) {
        let width = field.width(r, t, s);
        field.draw(r, t, RIGHT * s - width, y, s);
    }
}

/// Space between the fields of a line, and after a label.
const GAP: f32 = 40.0;
const LABEL_GAP: f32 = 10.0;
const LABEL_PX: f32 = 19.0;
const VALUE_PX: f32 = 24.0;

/// A label, then its value and, in grey, what goes with it.
#[derive(Debug, PartialEq)]
struct Field {
    label: Cow<'static, str>,
    value: String,
    aside: Option<String>,
}

impl Field {
    fn new(label: &'static str, value: String) -> Self {
        Self { label: Cow::Borrowed(label), value, aside: None }
    }

    /// Draws the field from `x`, on the line starting at `y`; returns where
    /// it ends.
    fn draw(&self, r: &mut Renderer, t: &mut Text, x: f32, y: f32, s: f32) -> f32 {
        let (pieces, width) = self.pieces(r, t, s);
        for (offset, text, px, color) in pieces {
            // A smaller label's top lower, so that it sits on the value's
            // baseline.
            let lower = (VALUE_PX - px) * 0.8 * s;
            t.draw(r, x + offset, y + lower, px * s, color, text);
        }
        x + width
    }

    fn width(&self, r: &mut Renderer, t: &mut Text, s: f32) -> f32 {
        self.pieces(r, t, s).1
    }

    /// The label, the value and the aside, each with its offset from the
    /// field's start, its size and its colour; and the field's width.
    fn pieces(&self, r: &mut Renderer, t: &mut Text, s: f32) -> (Vec<(f32, &str, f32, Color)>, f32) {
        let mut pieces = vec![(&*self.label, LABEL_PX, MUTED), (self.value.as_str(), VALUE_PX, TEXT)];
        pieces.extend(self.aside.as_deref().map(|aside| (aside, VALUE_PX, MUTED)));
        let (mut placed, mut end) = (Vec::new(), 0.0);
        for (i, (text, px, color)) in pieces.into_iter().enumerate() {
            let offset = if i == 0 { 0.0 } else { end + LABEL_GAP * s };
            end = offset + t.width(r, px * s, text);
            placed.push((offset, text, px, color));
        }
        (placed, end)
    }
}

/// `text` ending at `right`; returns where it starts.
fn draw_right(r: &mut Renderer, t: &mut Text, right: f32, y: f32, px: f32, color: Color, text: &str) -> f32 {
    let start = right - t.width(r, px, text);
    t.draw(r, start, y, px, color, text);
    start
}

/// The viewing's state as the panel says it, and whether it ended in an
/// error.
fn state(state: &State, language: Language) -> (String, bool) {
    (state.text(language), matches!(state, State::Failed(_)))
}

/// The video and audio the decoder plays, once known.
fn formats(formats: Option<&Formats>, language: Language) -> Vec<Field> {
    let Some(formats) = formats else { return Vec::new() };
    let video = match formats.video {
        VideoCodec::H264 => "H.264",
        VideoCodec::H265 => "HEVC",
    };
    let audio = match formats.audio {
        Some(AudioCodec::Aac) => "AAC".into(),
        Some(AudioCodec::Ac3) => "AC-3".into(),
        Some(AudioCodec::Eac3) => "E-AC-3".into(),
        None => text!(language, "info-no-audio"),
    };
    vec![
        Field::new("VIDEO", format!("{video} {}×{}", formats.width, formats.height)),
        Field::new("AUDIO", audio),
    ]
}

/// The quality asked for and, in grey, the heights offered, when the
/// stream offers more than one.
fn quality(stats: &Stats, language: Language) -> Option<Field> {
    let heights = &stats.live.heights;
    if heights.len() < 2 {
        return None;
    }
    let offered: Vec<String> = heights.iter().rev().map(|&h| quality_menu::label(Quality::Lines(h))).collect();
    let label = text!(language, "info-quality");
    Some(Field { label: Cow::Owned(label), value: quality_menu::label(stats.quality), aside: Some(offered.join(" ")) })
}

/// `elapsed` as minutes and seconds, with the hours past the first.
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mana_core::playback;
    use mana_core::session::Failure;

    #[test]
    fn the_fields_name_the_formats_and_the_quality_only_with_variants() {
        let mut stats = Stats::default();
        let it = Language::Italian;
        assert!(formats(None, it).is_empty());
        let played = Formats { video: VideoCodec::H264, width: 1920, height: 1080, audio: Some(AudioCodec::Aac) };
        assert_eq!(
            formats(Some(&played), it),
            [Field::new("VIDEO", "H.264 1920×1080".into()), Field::new("AUDIO", "AAC".into())]
        );
        let silent = Formats { audio: None, ..played };
        assert_eq!(formats(Some(&silent), it)[1].value, "nessuno");
        assert_eq!(formats(Some(&silent), Language::English)[1].value, "none");
        stats.live.heights = vec![1080];
        assert_eq!(quality(&stats, it), None);
        stats.live.heights = vec![360, 720, 1080];
        stats.quality = Quality::Auto;
        let field = quality(&stats, it).unwrap();
        assert_eq!((field.label.as_ref(), field.value.as_str(), field.aside.as_deref()), ("QUALITÀ", "Auto", Some("1080p 720p 360p")));
        assert_eq!(quality(&stats, Language::English).unwrap().label, "QUALITY");
        stats.quality = Quality::Lines(720);
        assert_eq!(quality(&stats, it).unwrap().value, "720p");
    }

    #[test]
    fn the_state_reads_as_a_word_and_an_error_as_itself() {
        let it = Language::Italian;
        assert_eq!(state(&State::Playing, it), ("In riproduzione".into(), false));
        assert_eq!(state(&State::Stopped, Language::English), ("Stopped".into(), false));
        let failed = State::Failed(Failure::Thread("Resource temporarily unavailable".into()));
        assert_eq!(state(&failed, it), ("Errore: Resource temporarily unavailable".into(), true));
        let codec = State::Failed(Failure::Playback(playback::Error::VideoCodec(Some("mpeg2video 720×576".into()))));
        assert_eq!(state(&codec, it).0, "Errore: codec video non supportato: mpeg2video 720×576");
        assert_eq!(state(&codec, Language::English).0, "Error: unsupported video codec: mpeg2video 720×576");
        let no_video = State::Failed(Failure::Playback(playback::Error::VideoCodec(None)));
        assert_eq!(state(&no_video, it).0, "Errore: il flusso non ha video");
        assert_eq!(clock(Duration::from_secs(75)), "1:15");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }

    #[test]
    fn up_or_down_closes_the_panel_shown_at_start_and_opens_it_again() {
        let mut panel = InfoPanel::default();
        assert!(!panel.shown());
        panel.started();
        assert!(panel.shown());
        panel.toggle();
        assert!(!panel.shown());
        panel.toggle();
        assert!(panel.shown());
    }
}
