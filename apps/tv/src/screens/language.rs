//! The language of the screens and of the phone's page: resolved once at
//! start, from the choice kept in the data directory, else the TV's, else
//! English; changed from the settings screen at once. A change of the TV's
//! language while the app runs is not followed.

use std::path::Path;

use mana_core::language::{self, Choice, Language};
use mana_core::log::log;

use crate::platform::DATA_DIR;
use crate::platform::locale::Reading;

pub struct Languages {
    /// The language the screens draw in.
    current: Language,
    choice: Choice,
    /// The TV's locale code, or why it could not be read; None while not
    /// read: a choice kept skips the read at start.
    tv: Option<Result<String, String>>,
    /// The read of the TV's language, while it has not answered.
    reading: Option<Reading>,
    /// The read is the one at start, which the first frame waits for.
    at_start: bool,
}

impl Languages {
    /// The choice kept and, with Automatic, the TV's language being read:
    /// until it answers, the screens wait (`ready`). `tests_locale` stands
    /// for the TV's code without asking it; empty, a TV that cannot be read.
    pub fn start(tests_locale: Option<String>) -> Self {
        let choice = Choice::load(Path::new(DATA_DIR));
        let mut languages = Self { current: Language::English, choice, tv: None, reading: None, at_start: false };
        match tests_locale {
            Some(code) if code.is_empty() => languages.tv = Some(Err("the tests' settings give no locale".into())),
            Some(code) => languages.tv = Some(Ok(code)),
            None if choice == Choice::Automatic => {
                languages.reading = Some(Reading::start());
                languages.at_start = true;
            }
            None => {}
        }
        if languages.reading.is_none() {
            languages.resolve();
        }
        languages
    }

    pub fn current(&self) -> Language {
        self.current
    }

    pub fn choice(&self) -> Choice {
        self.choice
    }

    /// Whether the language is known: false only while the TV's is read at
    /// start, for at most `locale::LIMIT`.
    pub fn ready(&self) -> bool {
        !self.at_start
    }

    /// Takes the TV's answer once it came; call at every turn of the event
    /// loop.
    pub fn refresh(&mut self) {
        let Some(answer) = self.reading.as_ref().and_then(Reading::poll) else { return };
        self.reading = None;
        self.at_start = false;
        self.tv = Some(answer);
        if self.choice == Choice::Automatic {
            self.resolve();
        }
    }

    /// The viewer's `choice`, kept at once. Automatic with the TV's language
    /// not read yet reads it: the language in use stays until it answers.
    pub fn choose(&mut self, choice: Choice) {
        self.choice = choice;
        if let Err(error) = choice.save(Path::new(DATA_DIR)) {
            log!("language: choice not saved: {error}");
        }
        if choice == Choice::Automatic && self.tv.is_none() {
            self.reading.get_or_insert_with(Reading::start);
            return;
        }
        self.resolve();
    }

    fn resolve(&mut self) {
        let (language, origin) = language::resolve(self.choice, self.tv.as_ref());
        log!("language: {language}, {origin}");
        self.current = language;
    }
}
