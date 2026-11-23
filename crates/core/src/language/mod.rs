//! The language of the texts the apps show: English or Italian, the
//! viewer's choice from the settings (kept in the data directory the app
//! gives), else the TV's own, else English. Every message the core hands
//! to a screen is a value that turns into text in either language
//! (`Localized`); its `Display`, for the log, is the English text.
//!
//! The texts themselves are in one Fluent catalog per language, `en.ftl`
//! and `it.ftl` next to this file, built into the binary and shared by the
//! apps; the `text!` macro takes one by its id. The tests check that both
//! catalogs have the same messages with the same variables, and that every
//! `text!` in the workspace names a message that exists, with the variables
//! the message uses.

use std::sync::LazyLock;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};

use crate::log::log;

mod choice;
#[cfg(test)]
mod tests;

pub use choice::{Choice, Origin, resolve};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    English,
    Italian,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::Italian];

    /// The language of a locale code, as `it-IT`, `it_CH.UTF-8` or `en`:
    /// Italian when its language part is `it`, English otherwise.
    pub fn of_locale(code: &str) -> Self {
        let language = code.trim().split(['-', '_', '.', '@']).next().unwrap_or("");
        if language.eq_ignore_ascii_case(Self::Italian.tag()) { Self::Italian } else { Self::English }
    }

    /// The BCP 47 tag, for an HTML page and for the catalog's plural rules.
    pub fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Italian => "it",
        }
    }

    /// The catalog's source.
    fn source(self) -> &'static str {
        match self {
            Self::English => include_str!("en.ftl"),
            Self::Italian => include_str!("it.ftl"),
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::English => "English",
            Self::Italian => "Italian",
        })
    }
}

/// A message that reads in either language.
pub trait Localized {
    fn text(&self, language: Language) -> String;
}

/// Implements `Display` as the English text of a `Localized` type, for the
/// log.
macro_rules! display_in_english {
    ($($type:ty),+ $(,)?) => {$(
        impl std::fmt::Display for $type {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(&$crate::language::Localized::text(self, $crate::language::Language::English))
            }
        }
    )+};
}
pub(crate) use display_in_english;

/// The text of a message of the catalogs in a language, with its
/// variables named as in the message: `text!(language, "id", count = 3)`.
#[macro_export]
macro_rules! text {
    ($language:expr, $id:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::language::message($language, $id, &[$((stringify!($name), $crate::language::Arg::from($value))),*])
    };
}

/// The value of a message's variable: a text, or a number, which plural
/// forms select on.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Text(String),
    Number(f64),
}

impl From<&str> for Arg {
    fn from(text: &str) -> Self {
        Self::Text(text.into())
    }
}

impl From<String> for Arg {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<&String> for Arg {
    fn from(text: &String) -> Self {
        Self::Text(text.clone())
    }
}

macro_rules! number_args {
    ($($type:ty),+) => {$(
        impl From<$type> for Arg {
            #[allow(clippy::cast_precision_loss, clippy::cast_lossless)]
            fn from(number: $type) -> Self {
                Self::Number(number as f64)
            }
        }
        impl From<&$type> for Arg {
            fn from(number: &$type) -> Self {
                Self::from(*number)
            }
        }
    )+};
}
number_args!(u8, u16, u32, u64, u128, usize, i32, i64);

type Catalog = FluentBundle<FluentResource>;

/// The catalogs, each read at its first text.
static ENGLISH: LazyLock<Catalog> = LazyLock::new(|| load(Language::English));
static ITALIAN: LazyLock<Catalog> = LazyLock::new(|| load(Language::Italian));

fn catalog(language: Language) -> &'static Catalog {
    match language {
        Language::English => &ENGLISH,
        Language::Italian => &ITALIAN,
    }
}

/// The catalog of `language`. A catalog with errors keeps the messages it
/// could read; the tests keep both free of errors.
fn load(language: Language) -> Catalog {
    let tag = language.tag().parse().expect("a valid language tag");
    let mut catalog = FluentBundle::new_concurrent(vec![tag]);
    // Without the Unicode isolation marks around variables, which the
    // screens' font does not draw.
    catalog.set_use_isolating(false);
    let resource = FluentResource::try_new(language.source().to_string()).unwrap_or_else(|(resource, errors)| {
        log!("language: errors in the {language} catalog: {errors:?}");
        resource
    });
    if let Err(errors) = catalog.add_resource(resource) {
        log!("language: errors in the {language} catalog: {errors:?}");
    }
    catalog
}

/// The text of message `id` in `language`, through `text!`. A message
/// missing from the catalog gives its id.
pub fn message(language: Language, id: &str, args: &[(&str, Arg)]) -> String {
    let catalog = catalog(language);
    let Some(pattern) = catalog.get_message(id).and_then(|message| message.value()) else {
        log!("language: no message {id} in the {language} catalog");
        return id.into();
    };
    let mut values = FluentArgs::with_capacity(args.len());
    for (name, arg) in args {
        let value = match arg {
            Arg::Text(text) => FluentValue::from(text.as_str()),
            Arg::Number(number) => FluentValue::from(*number),
        };
        values.set(*name, value);
    }
    let mut errors = Vec::new();
    let text = catalog.format_pattern(pattern, Some(&values), &mut errors).into_owned();
    if !errors.is_empty() {
        log!("language: message {id} in {language}: {errors:?}");
    }
    text
}
