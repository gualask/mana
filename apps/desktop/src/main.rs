//! Mana on the desktop, an alpha: the channel list of a URL, its folders
//! and signals, played through the core's viewing session by a backend
//! that decodes in software (backend/). The list's URL is given once with
//! `--list URL` and kept, as the phone sets it on the TV.
//!
//! Keys: Enter opens the list and chooses; Esc over a video shows the list
//! where it was left, then goes back a level, then hides it; ↑ ↓ move by a
//! row, ← → by a page.

mod app;
mod awake;
mod backend;
mod view;

use std::path::PathBuf;

use mana_core::language::Language;
use rune::native_surface::synthetic;
use rune::{AppSpec, Chrome, KeyEvent, Tx, Waker};
use url::Url;

use app::App;

/// The window opens on a test picture until a video plays.
const PICTURE: (u32, u32) = (320, 180);

fn main() {
    let spec = AppSpec { setup: Some(setup), poll: Some(poll), on_key: Some(on_key), ..AppSpec::new("Mana", (1280.0, 720.0), start, view::view) };
    // A standard titled window: under a full-size titlebar, full screen put
    // two of its buttons off the screen.
    rune::App::new(spec).chrome(Chrome { full_size_content: false, ..Chrome::DEFAULT }).run();
}

fn start() -> App {
    let args = rune::extra_args();
    let list = args.iter().position(|a| a == "--list").and_then(|at| args.get(at + 1)).and_then(|url| Url::parse(url).ok());
    let app = App::new(&data_dir(), list, language());
    let _ = backend::push(&app.surface, synthetic(PICTURE.0, PICTURE.1, 0), 0);
    app
}

fn setup(app: &mut App, waker: Waker) {
    app.surface.set_waker(waker);
}

fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
    app.poll(tx)
}

fn on_key(app: &mut App, event: &KeyEvent, _: &mut Tx) -> bool {
    app.on_key(event)
}

/// Where the list, its copies and the device identifier are kept, as each
/// system wants an app's data.
fn data_dir() -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Mana")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Mana")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("mana")
    }
}

/// Italian when the system's locale is, else English.
fn language() -> Language {
    Language::environment_locale().map_or(Language::English, |code| Language::of_locale(&code))
}
