//! Why a playlist could not be read, and how the screens say it.

use crate::language::{Language, Localized, display_in_english};
use crate::text;

/// Why a playlist could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a playlist, or a damaged one: a later download may do better.
    Invalid(Invalid),
    /// HLS features this client does not have.
    Unsupported(Unsupported),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    NotPlaylist,
    NotUtf8,
    MapWithoutUri,
    KeyWithoutUri,
    Sequence(String),
    Duration(String),
    Iv(String),
    /// A URL that does not resolve, as the url crate says.
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    ByteRange,
    MapByteRange,
    Dash,
    /// An encryption method other than AES-128, as written.
    Encryption(String),
    /// An answer that is neither a playlist nor MPEG-TS.
    OtherFormat,
}

impl From<url::ParseError> for Error {
    fn from(error: url::ParseError) -> Self {
        Self::Invalid(Invalid::Url(error.to_string()))
    }
}

impl Localized for Error {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Invalid(invalid) => match invalid {
                Invalid::NotPlaylist => text!(language, "playlist-invalid"),
                Invalid::NotUtf8 => text!(language, "playlist-not-utf8"),
                Invalid::MapWithoutUri => text!(language, "playlist-map-without-uri"),
                Invalid::KeyWithoutUri => text!(language, "playlist-key-without-uri"),
                Invalid::Sequence(value) => text!(language, "playlist-sequence", value = value),
                Invalid::Duration(value) => text!(language, "playlist-duration", value = value),
                Invalid::Iv(value) => text!(language, "playlist-iv", value = value),
                Invalid::Url(detail) => detail.clone(),
            },
            Self::Unsupported(unsupported) => match unsupported {
                Unsupported::ByteRange => text!(language, "playlist-byte-range"),
                Unsupported::MapByteRange => text!(language, "playlist-map-byte-range"),
                Unsupported::Dash => text!(language, "playlist-dash"),
                Unsupported::Encryption(method) => text!(language, "playlist-encryption", method = method),
                Unsupported::OtherFormat => text!(language, "playlist-other-format"),
            },
        }
    }
}

display_in_english!(Error);
