//! From a source to a byte stream: the `.stream` file, the recipe that
//! resolves it, and the HLS client that downloads it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub mod hls;
pub mod recipe;
pub mod source;

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
            _ => Err(format!("qualità non valida: {text}")),
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

/// Why an opening of a live source stopped itself. Live only reports it:
/// whether to open the source again is the session's to decide.
#[derive(Clone, Debug, PartialEq)]
pub enum Interruption {
    /// The server refused the URLs' token: only a new resolution gives new
    /// ones.
    Refused(String),
    /// The playlist stood still, or its numbering went back.
    Stuck(String),
    /// The stream uses HLS features this client does not have.
    Unsupported(String),
    /// A downloader gave up for another reason.
    Failed(String),
}

impl std::fmt::Display for Interruption {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Refused(reason) | Self::Stuck(reason) | Self::Failed(reason) => {
                f.write_str(reason)
            }
            Self::Unsupported(reason) => write!(f, "non supportato: {reason}"),
        }
    }
}

/// What a live source shows the screens.
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
