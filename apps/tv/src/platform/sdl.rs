//! Minimal SDL2 binding loaded at runtime: only the functions the app uses
//! are bound; events are decoded in event.rs.

use std::ffi::{CStr, CString, c_char, c_int, c_void};

use libloading::Library;

mod event;

use event::RawEvent;
pub use event::Event;

type RawWindow = *mut c_void;
type RawContext = *mut c_void;

const INIT_VIDEO: u32 = 0x0000_0020;
const WINDOW_FULLSCREEN: u32 = 0x0000_0001;
const WINDOW_OPENGL: u32 = 0x0000_0002;
const WINDOW_RESIZABLE: u32 = 0x0000_0020;
const WINDOW_ALLOW_HIGHDPI: u32 = 0x0000_2000;
const WINDOWPOS_CENTERED: c_int = 0x2FFF_0000;

const GL_ALPHA_SIZE: c_int = 3;
const GL_DOUBLEBUFFER: c_int = 5;
const GL_CONTEXT_MAJOR_VERSION: c_int = 17;
const GL_CONTEXT_MINOR_VERSION: c_int = 18;
const GL_CONTEXT_PROFILE_MASK: c_int = 21;
const GL_CONTEXT_PROFILE_ES: c_int = 0x0004;


struct Api {
    init: unsafe extern "C" fn(u32) -> c_int,
    quit: unsafe extern "C" fn(),
    get_error: unsafe extern "C" fn() -> *const c_char,
    set_hint: unsafe extern "C" fn(*const c_char, *const c_char) -> c_int,
    gl_set_attribute: unsafe extern "C" fn(c_int, c_int) -> c_int,
    create_window: unsafe extern "C" fn(*const c_char, c_int, c_int, c_int, c_int, u32) -> RawWindow,
    destroy_window: unsafe extern "C" fn(RawWindow),
    gl_create_context: unsafe extern "C" fn(RawWindow) -> RawContext,
    gl_delete_context: unsafe extern "C" fn(RawContext),
    gl_get_proc_address: unsafe extern "C" fn(*const c_char) -> *const c_void,
    gl_set_swap_interval: unsafe extern "C" fn(c_int) -> c_int,
    gl_swap_window: unsafe extern "C" fn(RawWindow),
    gl_get_drawable_size: unsafe extern "C" fn(RawWindow, *mut c_int, *mut c_int),
    get_window_size: unsafe extern "C" fn(RawWindow, *mut c_int, *mut c_int),
    wait_event_timeout: unsafe extern "C" fn(*mut RawEvent, c_int) -> c_int,
    poll_event: unsafe extern "C" fn(*mut RawEvent) -> c_int,
}

impl Api {
    fn load(lib: &Library) -> Result<Self, String> {
        // SAFETY: each symbol is looked up by its exact SDL2 name and cast to
        // the C signature documented in SDL2's headers (stable since 2.0.0).
        unsafe {
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes())
                        .map_err(|e| format!("simbolo SDL mancante {}: {e}", $name))?
                };
            }
            Ok(Self {
                init: sym!("SDL_Init"),
                quit: sym!("SDL_Quit"),
                get_error: sym!("SDL_GetError"),
                set_hint: sym!("SDL_SetHint"),
                gl_set_attribute: sym!("SDL_GL_SetAttribute"),
                create_window: sym!("SDL_CreateWindow"),
                destroy_window: sym!("SDL_DestroyWindow"),
                gl_create_context: sym!("SDL_GL_CreateContext"),
                gl_delete_context: sym!("SDL_GL_DeleteContext"),
                gl_get_proc_address: sym!("SDL_GL_GetProcAddress"),
                gl_set_swap_interval: sym!("SDL_GL_SetSwapInterval"),
                gl_swap_window: sym!("SDL_GL_SwapWindow"),
                gl_get_drawable_size: sym!("SDL_GL_GetDrawableSize"),
                get_window_size: sym!("SDL_GetWindowSize"),
                wait_event_timeout: sym!("SDL_WaitEventTimeout"),
                poll_event: sym!("SDL_PollEvent"),
            })
        }
    }
}

/// How the window is created on each platform.
pub struct WindowConfig {
    pub title: &'static str,
    pub width: i32,
    pub height: i32,
    pub fullscreen: bool,
    /// Request an OpenGL ES 2.0 context (TV). Otherwise the platform default.
    pub gles2: bool,
}

/// An SDL library with one window and its GL context.
pub struct Sdl {
    api: Api,
    window: RawWindow,
    context: RawContext,
    _lib: Library,
}

impl Sdl {
    /// Loads SDL from the first candidate path that opens, then creates the
    /// window and the GL context. `hints` are applied before `SDL_Init`.
    pub fn open(candidates: &[&str], hints: &[(&str, &str)], config: &WindowConfig) -> Result<Self, String> {
        let lib = open_first(candidates)?;
        let api = Api::load(&lib)?;
        let sdl = |result: bool, what: &str| -> Result<(), String> {
            if result { Ok(()) } else { Err(format!("{what}: {}", last_error(&api))) }
        };
        // SAFETY: calls follow SDL's documented initialization order; all
        // pointers passed are valid NUL-terminated strings or null checks.
        unsafe {
            for (name, value) in hints {
                let name = CString::new(*name).unwrap();
                let value = CString::new(*value).unwrap();
                (api.set_hint)(name.as_ptr(), value.as_ptr());
            }
            sdl((api.init)(INIT_VIDEO) == 0, "SDL_Init")?;
            (api.gl_set_attribute)(GL_DOUBLEBUFFER, 1);
            if config.gles2 {
                (api.gl_set_attribute)(GL_CONTEXT_PROFILE_MASK, GL_CONTEXT_PROFILE_ES);
                (api.gl_set_attribute)(GL_CONTEXT_MAJOR_VERSION, 2);
                (api.gl_set_attribute)(GL_CONTEXT_MINOR_VERSION, 0);
                (api.gl_set_attribute)(GL_ALPHA_SIZE, 8);
            }
            let mut flags = WINDOW_OPENGL | WINDOW_ALLOW_HIGHDPI;
            flags |= if config.fullscreen { WINDOW_FULLSCREEN } else { WINDOW_RESIZABLE };
            let title = CString::new(config.title).unwrap();
            let window = (api.create_window)(
                title.as_ptr(), WINDOWPOS_CENTERED, WINDOWPOS_CENTERED, config.width, config.height, flags,
            );
            if window.is_null() {
                let error = format!("SDL_CreateWindow: {}", last_error(&api));
                (api.quit)();
                return Err(error);
            }
            let context = (api.gl_create_context)(window);
            if context.is_null() {
                let error = format!("SDL_GL_CreateContext: {}", last_error(&api));
                (api.destroy_window)(window);
                (api.quit)();
                return Err(error);
            }
            (api.gl_set_swap_interval)(1);
            Ok(Self { api, window, context, _lib: lib })
        }
    }

    pub fn gl_proc_address(&self, name: &CStr) -> *const c_void {
        // SAFETY: a context is current on this thread and `name` is NUL-terminated.
        unsafe { (self.api.gl_get_proc_address)(name.as_ptr()) }
    }

    pub fn drawable_size(&self) -> (i32, i32) {
        let (mut w, mut h) = (0, 0);
        // SAFETY: the window is alive for the lifetime of `self`.
        unsafe { (self.api.gl_get_drawable_size)(self.window, &mut w, &mut h) };
        (w, h)
    }

    /// Drawable pixels per window unit, in which the pointer comes: 1 on
    /// the TV, 2 on a Retina display.
    pub fn pixels_per_unit(&self) -> (f32, f32) {
        let (mut w, mut h) = (0, 0);
        // SAFETY: the window is alive for the lifetime of `self`.
        unsafe { (self.api.get_window_size)(self.window, &mut w, &mut h) };
        let (dw, dh) = self.drawable_size();
        if w <= 0 || h <= 0 { (1.0, 1.0) } else { (dw as f32 / w as f32, dh as f32 / h as f32) }
    }

    pub fn swap(&self) {
        // SAFETY: the window is alive for the lifetime of `self`.
        unsafe { (self.api.gl_swap_window)(self.window) }
    }

    /// Waits up to `timeout_ms` for the first event, then drains the queue.
    pub fn wait_events(&self, timeout_ms: i32) -> Vec<Event> {
        let mut events = Vec::new();
        let mut raw = RawEvent([0; 128]);
        // SAFETY: `raw` is larger than any SDL_Event SDL can write.
        unsafe {
            if (self.api.wait_event_timeout)(&mut raw, timeout_ms) == 1 {
                events.push(Event::decode(&raw));
                while (self.api.poll_event)(&mut raw) == 1 {
                    events.push(Event::decode(&raw));
                }
            }
        }
        events
    }
}

impl Drop for Sdl {
    fn drop(&mut self) {
        // SAFETY: context and window were created by this instance and are
        // destroyed once, before SDL_Quit.
        unsafe {
            (self.api.gl_delete_context)(self.context);
            (self.api.destroy_window)(self.window);
            (self.api.quit)();
        }
    }
}

fn open_first(candidates: &[&str]) -> Result<Library, String> {
    let mut errors = Vec::new();
    for path in candidates {
        // SAFETY: loading SDL runs its library constructors, which have no
        // preconditions.
        match unsafe { Library::new(*path) } {
            Ok(lib) => return Ok(lib),
            Err(e) => errors.push(format!("{path}: {e}")),
        }
    }
    Err(format!("SDL non trovata ({})", errors.join("; ")))
}

fn last_error(api: &Api) -> String {
    // SAFETY: SDL_GetError always returns a valid, possibly empty, C string.
    unsafe { CStr::from_ptr((api.get_error)()).to_string_lossy().into_owned() }
}
