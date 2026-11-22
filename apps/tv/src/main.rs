//! Mana on the TV: composes the core with SDL, the Starfish backend and the
//! screens, then runs the event loop.

mod autotest;
mod palette;
mod phone;
mod platform;
mod screens;
mod settings;

use std::time::{Duration, Instant};

use autotest::Autotest;
use mana_core::log::{self, debug, log};
use mana_core::session::Session;
use platform::keys::{self, Action};
use platform::pointer::Pointer;
use platform::sdl::{Event, Sdl};
use screens::gfx::Renderer;
use screens::text::Text;
use screens::{App, Outcome};

/// The log on the TV, read over SSH.
const LOG_FILE: &str = "/tmp/mana.log";
/// SDL application-lifecycle event sent when the system terminates the app.
const APP_TERMINATING: u32 = 0x101;
/// Redraw at least this often so the screens stay current.
const IDLE_REDRAW_MS: i32 = 1000;
/// While playing, and while the settings screen waits for the phone, whose
/// URLs and outcomes are taken at each turn.
const PLAYING_REDRAW_MS: i32 = 250;
/// While logos arrive for the channel list, redraw this often to show them:
/// every frame would cost the compositor about a fifth of a core.
const LOADING_REDRAW_MS: i32 = 100;
/// While a viewing waits for its first frame, as the loader needs, for the
/// few seconds it turns.
const LOADER_REDRAW_MS: i32 = screens::LOADER_REDRAW.as_millis() as i32;
/// While animating, events are only polled: the vsync'd swap paces frames.
const ANIMATING_REDRAW_MS: i32 = 0;
/// How long closing waits for playback to end. A native app is killed
/// about a second after SIGTERM, which SDL turns into Quit: unloading
/// Starfish in time leaves nothing for the system to clean up.
const CLOSE_LIMIT: Duration = Duration::from_millis(600);

fn main() {
    platform::prepare_environment();
    log::init(platform::WEBOS.then_some(LOG_FILE));
    #[cfg(webos)]
    platform::install_crash_handler(LOG_FILE);
    // webOS passes launch parameters as a JSON string in argv[1].
    log!("avvio, parametri: {}", std::env::args().nth(1).unwrap_or_default());
    log!("istruzioni AES della CPU: {}", if platform::use_aes_instructions() { "sì" } else { "no" });
    match run() {
        Ok(true) => log!("uscita"),
        Ok(false) => {
            log!("uscita senza attendere la fine della riproduzione");
            platform::exit_now();
        }
        Err(error) => {
            log!("errore: {error}");
            std::process::exit(1);
        }
    }
}

/// Returns whether playback ended before the app did.
fn run() -> Result<bool, String> {
    let sdl = Sdl::open(platform::SDL_CANDIDATES, platform::SDL_HINTS, &platform::WINDOW)?;
    // SAFETY: the SDL GL context is current on this thread for its lifetime.
    let gl = unsafe { glow::Context::from_loader_function_cstr(|name| sdl.gl_proc_address(name)) };
    let mut renderer = Renderer::new(gl, platform::WINDOW.gles2)?;
    let mut text = Text::load(platform::FONT_CANDIDATES)?;
    let mut app = App::new(Session::new(platform::open_backend()));
    let mut autotest = Autotest::start(&mut app);
    let mut surface = sdl.drawable_size();
    log!("GL {} · superficie {surface:?}", renderer.info());
    let mut pointer = Pointer::new(sdl.pixels_per_unit());

    loop {
        let timeout = redraw_timeout(&autotest, &app);
        app.refresh();
        autotest.tick(&mut app);
        log::trim();
        let mut events = sdl.wait_events(timeout).into_iter();
        if events.any(|event| matches!(handle_event(event, &mut app, &mut pointer), Outcome::Exit)) {
            return Ok(app.close(CLOSE_LIMIT));
        }
        surface = sdl.drawable_size();
        pointer.set_scale(sdl.pixels_per_unit());
        let drawing = std::time::Instant::now();
        app.draw(&mut renderer, &mut text, surface);
        let draw = drawing.elapsed();
        sdl.swap();
        autotest.frame_drawn(&app, draw);
    }
}

/// How long to wait for events before the next frame.
fn redraw_timeout(autotest: &Autotest, app: &App) -> i32 {
    if autotest.animating() {
        ANIMATING_REDRAW_MS
    } else if app.waiting_for_video() {
        LOADER_REDRAW_MS
    } else if app.loading_logos() {
        LOADING_REDRAW_MS
    } else if app.is_playing() || app.setup_open() {
        PLAYING_REDRAW_MS
    } else {
        IDLE_REDRAW_MS
    }
}

/// Logs the event and sends keys and the pointer to the app; `Exit` when
/// the app should end.
fn handle_event(event: Event, app: &mut App, pointer: &mut Pointer) -> Outcome {
    let now = Instant::now();
    match event {
        Event::Quit | Event::Lifecycle(APP_TERMINATING) => return Outcome::Exit,
        Event::Key { down: true, repeat, scancode, keycode } => {
            debug!("tasto scancode={scancode} keycode={keycode} repeat={repeat}");
            return key(scancode, app, pointer, now);
        }
        Event::Lifecycle(kind) => debug!("ciclo di vita 0x{kind:x}"),
        Event::Window(id) => debug!("finestra evento {id}"),
        Event::PointerMove { x, y } => {
            let at = pointer.pixels(x, y);
            // A tremor after an arrow key moves nothing.
            if pointer.moved(at, now) {
                app.point(at.0, at.1);
            }
        }
        Event::PointerPress { x, y } => return click(x, y, app, pointer),
        Event::Wheel(notches) if notches != 0 && pointer.wheel(now) => {
            return app.handle(if notches > 0 { Action::Up } else { Action::Down });
        }
        Event::Key { .. } | Event::PointerRelease | Event::Wheel(_) | Event::Other => {}
    }
    Outcome::Continue
}

/// A key of the remote: its action, if any, for the app; an arrow puts the
/// pointer away.
fn key(scancode: i32, app: &mut App, pointer: &mut Pointer, now: Instant) -> Outcome {
    let Some(action) = keys::action(scancode) else { return Outcome::Continue };
    if matches!(action, Action::Up | Action::Down | Action::Left | Action::Right) {
        pointer.put_away(now);
    }
    app.handle(action)
}

/// A click at `(x, y)`, in window units: on the row under the pointer, or
/// OK on the focus while the pointer is away.
fn click(x: i32, y: i32, app: &mut App, pointer: &Pointer) -> Outcome {
    if pointer.is_away() {
        return app.handle(Action::Select);
    }
    let at = pointer.pixels(x, y);
    app.click(at.0, at.1)
}
