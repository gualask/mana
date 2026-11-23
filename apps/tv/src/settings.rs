//! The TV's settings files: `key = value` lines, changed over SSH without
//! reinstalling. Those kept in the data directory, then those of the tests
//! in /tmp, which override them. Playback tunables are read again at each
//! playback, the rest when the app starts.

use std::path::{Path, PathBuf};

use mana_core::catalog::update::list_url;
use mana_core::log::log;
use mana_core::playback;
use mana_core::session::Quality;
use url::Url;

use crate::autotest;
use crate::platform::DATA_DIR;

/// The settings kept across power-offs, in DATA_DIR.
const KEPT_FILE: &str = "mana.conf";
/// The settings of the tests, gone at the next power-off.
const TESTS_FILE: &str = "/tmp/mana.conf";
/// The key of the channel list's URL, which Settings names too.
pub const LIST_KEY: &str = "list";

/// What the settings file sets.
#[derive(Default)]
pub struct Loaded {
    pub playback: playback::Settings,
    pub tests: autotest::Options,
    /// The quality of live streams with variants when the app starts
    /// (`quality = auto` or the lines, as `720`), then changed by the menu.
    pub quality: Quality,
    /// The URL of the channel list (`list = URL`), instead of the one
    /// chosen from the phone, and the file that sets it.
    pub list: Option<(Url, PathBuf)>,
    /// The TV's locale code in place of the one the TV gives, without
    /// asking it (`locale = it-IT`); empty, a TV whose language cannot be
    /// read. Only in the tests' file.
    pub locale: Option<String>,
    /// The log with the lines that repeat while playing (`debug = true`),
    /// to debug.
    pub debug: bool,
}

/// The settings of both files, those of the tests last.
pub fn load() -> Loaded {
    let mut loaded = Loaded::default();
    read(&Path::new(DATA_DIR).join(KEPT_FILE), &mut loaded, false);
    read(Path::new(TESTS_FILE), &mut loaded, true);
    loaded
}

/// Applies to `loaded` any valid line of `path`, the tests' file if
/// `for_tests`; other lines are logged.
fn read(path: &Path, loaded: &mut Loaded, for_tests: bool) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let Loaded { playback, tests, quality, list, locale, debug } = loaded;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let applied = match key {
            "lead_ms" => value.parse().ok().map(|v| playback.lead_ms = v),
            "audio_offset_ms" => value.parse().ok().map(|v| playback.audio_offset_ms = v),
            "loop" => value.parse().ok().map(|v| playback.loop_playback = v),
            "freeze_test_s" => value.parse().ok().map(|v| playback.freeze_test_s = v),
            "zap_count" => value.parse().ok().map(|v| tests.zap_count = v),
            "zap_interval_ms" => value.parse().ok().map(|v| tests.zap_interval_ms = v),
            "ui_test" => value.parse().ok().map(|v| tests.ui_test = v),
            "quality" => value.parse().ok().map(|q| *quality = q),
            "debug" => value.parse().ok().map(|v| *debug = v),
            LIST_KEY => list_url(value)
                .inspect_err(|e| log!("list in {}: {e}", path.display()))
                .ok()
                .map(|url| *list = Some((url, path.to_path_buf()))),
            "locale" if for_tests => {
                *locale = Some(value.to_string());
                Some(())
            }
            _ => None,
        };
        if applied.is_none() {
            // The key only: a value may carry a token, as a list's URL.
            log!("setting ignored in {}: {key}", path.display());
        }
    }
}
