//! The language the viewer chose in the settings, kept in the data
//! directory the app gives, and the language in use that follows from it.

use std::path::Path;

use super::Language;
use crate::log::log;

/// The file in the data directory that keeps the choice.
pub(super) const FILE: &str = "language";

/// The language the viewer chose in the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// The TV's language.
    #[default]
    Automatic,
    Italian,
    English,
}

impl Choice {
    /// The choices in the settings' order.
    const ORDER: [Self; 3] = [Self::Automatic, Self::Italian, Self::English];

    /// The next one in the settings' order, after the last the first.
    pub fn next(self) -> Self {
        let at = Self::ORDER.iter().position(|&choice| choice == self).unwrap_or(0);
        Self::ORDER[(at + 1) % Self::ORDER.len()]
    }

    /// The previous one in the same order.
    pub fn previous(self) -> Self {
        let at = Self::ORDER.iter().position(|&choice| choice == self).unwrap_or(0);
        Self::ORDER[(at + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// The choice kept in `dir`; Automatic without one. A file that cannot
    /// be read, or that holds an unknown value, counts as Automatic and is
    /// logged.
    pub fn load(dir: &Path) -> Self {
        match std::fs::read_to_string(dir.join(FILE)) {
            Ok(text) => Self::from_key(text.trim()).unwrap_or_else(|| {
                log!("language: unknown choice {:?} in {FILE}, automatic", text.trim());
                Self::Automatic
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Automatic,
            Err(error) => {
                log!("language: {FILE} not read, automatic: {error}");
                Self::Automatic
            }
        }
    }

    /// Keeps the choice in `dir`, for the next starts.
    pub fn save(self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(FILE), self.key())
    }

    fn key(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Italian => "italian",
            Self::English => "english",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ORDER.into_iter().find(|choice| choice.key() == key)
    }

    /// The name the settings show: Automatic in `language`, the languages
    /// each in its own.
    pub fn label(self, language: Language) -> String {
        match self {
            Self::Automatic => crate::text!(language, "language-automatic"),
            Self::Italian => crate::text!(language, "language-italian"),
            Self::English => crate::text!(language, "language-english"),
        }
    }
}

/// Where the language in use comes from, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Chosen in the settings.
    Menu,
    /// The TV's, with its locale code.
    Tv(String),
    /// English, because the TV's could not be read, with the reason.
    Default(String),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Menu => f.write_str("chosen in the settings"),
            Self::Tv(code) => write!(f, "TV language {code}"),
            Self::Default(reason) => write!(f, "default: {reason}"),
        }
    }
}

/// The language in use: the one chosen in the settings, else, with
/// Automatic, the TV's (`tv`, its locale code or why it could not be read;
/// None while not read), else English.
pub fn resolve(choice: Choice, tv: Option<&Result<String, String>>) -> (Language, Origin) {
    match (choice, tv) {
        (Choice::Italian, _) => (Language::Italian, Origin::Menu),
        (Choice::English, _) => (Language::English, Origin::Menu),
        (Choice::Automatic, Some(Ok(code))) => (Language::of_locale(code), Origin::Tv(code.clone())),
        (Choice::Automatic, Some(Err(reason))) => (Language::English, Origin::Default(reason.clone())),
        (Choice::Automatic, None) => (Language::English, Origin::Default("TV language not read".into())),
    }
}
