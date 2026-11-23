//! From a stream to play to a byte stream: the stream at a known URL (a
//! `.stream` file, a channel's URL, a source's resolution) and the HLS
//! client that downloads it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use crate::language::{Language, Localized, display_in_english};
use crate::net;
use crate::text;

pub mod hls;
pub mod playable;

/// The quality the viewer asks for, among the variants of a master
/// playlist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// Chosen on the bandwidth measured.
    #[default]
    Auto,
    /// The best variant at most this many lines high, kept whatever the
    /// bandwidth; the lightest when all are higher. Without a RESOLUTION
    /// on any variant, as Auto.
    Lines(u32),
}

impl std::fmt::Display for Quality {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Lines(lines) => write!(f, "{lines}p"),
        }
    }
}

/// `auto`, or the lines as `720` or `720p`.
impl std::str::FromStr for Quality {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if text == "auto" {
            return Ok(Self::Auto);
        }
        match text.strip_suffix('p').unwrap_or(text).parse() {
            Ok(lines) if lines > 0 => Ok(Self::Lines(lines)),
            _ => Err(format!("invalid quality: {text}")),
        }
    }
}

/// The viewer's choice, shared by the session that sets it and the
/// downloads that follow it, which apply a change from the next segment.
#[derive(Clone, Default)]
pub struct QualityChoice(Arc<AtomicU32>);

impl QualityChoice {
    pub fn get(&self) -> Quality {
        match self.0.load(Ordering::Relaxed) {
            0 => Quality::Auto,
            lines => Quality::Lines(lines),
        }
    }

    pub fn set(&self, quality: Quality) {
        let lines = match quality {
            Quality::Auto => 0,
            Quality::Lines(lines) => lines.max(1),
        };
        self.0.store(lines, Ordering::Relaxed);
    }
}

/// Why an opening of a live stream stopped itself. Live only reports it:
/// whether to open the stream again is the session's to decide.
#[derive(Clone, Debug, PartialEq)]
pub enum Interruption {
    /// The server refused the URLs' token: only a new resolution gives new
    /// ones.
    Refused(Cause),
    /// The playlist stood still, or its numbering went back.
    Stuck(Stall),
    /// The stream uses HLS features this client does not have.
    Unsupported(Cause),
    /// A downloader gave up for another reason.
    Failed(Cause),
}

impl Localized for Interruption {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Refused(cause) | Self::Failed(cause) => cause.text(language),
            Self::Stuck(stall) => stall.text(language),
            Self::Unsupported(cause) => text!(language, "live-unsupported", cause = cause.text(language)),
        }
    }
}

/// What made a download of a live stream fail.
#[derive(Clone, Debug, PartialEq)]
pub enum Cause {
    Network(net::Error),
    Playlist(hls::playlist::Error),
    Playable(playable::Error),
    /// A segment that does not decrypt.
    Decryption(hls::decrypt::Error),
    /// An AES-128 key that is not 16 bytes long.
    KeyLength,
    PackedAudio(hls::packed_audio::Error),
    /// The HTTP client could not be made, as TLS or the system says.
    Client(String),
}

impl Localized for Cause {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Network(error) => error.text(language),
            Self::Playlist(error) => error.text(language),
            Self::Playable(error) => error.text(language),
            Self::Decryption(hls::decrypt::Error::Length) => text!(language, "live-decryption-length"),
            Self::Decryption(hls::decrypt::Error::Padding) => text!(language, "live-decryption-padding"),
            Self::KeyLength => text!(language, "live-key-length"),
            Self::PackedAudio(error) => error.text(language),
            Self::Client(detail) => detail.clone(),
        }
    }
}

impl From<net::Error> for Cause {
    fn from(error: net::Error) -> Self {
        Self::Network(error)
    }
}

impl From<hls::playlist::Error> for Cause {
    fn from(error: hls::playlist::Error) -> Self {
        Self::Playlist(error)
    }
}

/// A playlist that stood still: for how long, its last segment and the one
/// expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stall {
    pub idle: Duration,
    pub last: u64,
    pub expected: u64,
}

impl Localized for Stall {
    fn text(&self, language: Language) -> String {
        let Self { idle, last, expected } = self;
        text!(language, "live-stall", seconds = idle.as_secs(), last = last, expected = expected)
    }
}

display_in_english!(Interruption, Cause, Stall);

/// What a live stream shows the screens.
#[derive(Clone, Default, Debug)]
pub struct Stats {
    /// The heights of its variants, lowest first, for choosing a quality.
    pub heights: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quality_reads_as_auto_or_lines_and_is_shared() {
        assert_eq!("auto".parse(), Ok(Quality::Auto));
        assert_eq!("720".parse(), Ok(Quality::Lines(720)));
        assert_eq!("576p".parse(), Ok(Quality::Lines(576)));
        assert!("0".parse::<Quality>().is_err() && "alta".parse::<Quality>().is_err());
        let choice = QualityChoice::default();
        let shared = choice.clone();
        choice.set(Quality::Lines(720));
        assert_eq!(shared.get(), Quality::Lines(720));
        assert_eq!(Quality::Lines(720).to_string(), "720p");
    }
}
