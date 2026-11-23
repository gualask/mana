//! Platform integration: process environment, SDL window, input mapping
//! and the TV's language.

pub mod clock;
pub mod keys;
pub mod locale;
pub mod pointer;
pub mod sdl;
#[cfg(webos)]
mod starfish;

use sdl::WindowConfig;

use mana_core::playback::OpenBackend;

/// Whether this is the webOS build: `build.rs` decides it, as `cfg(webos)`.
pub const WEBOS: bool = cfg!(webos);

/// Where the TV keeps Mana's files across power-offs: the settings, the
/// channel list's copies, the device identifier, the `.stream` files saved.
/// The persistent partition, outside the app's folder that an install
/// replaces. Elsewhere, a folder for development.
#[cfg(webos)]
pub const DATA_DIR: &str = "/media/developer/mana";
#[cfg(not(webos))]
pub const DATA_DIR: &str = "/tmp/mana-data";

/// Where to look for SDL2 at runtime. The TV always ships LG's SDL 2.0.10.
pub const SDL_CANDIDATES: &[&str] = if WEBOS {
    &["libSDL2-2.0.so.0"]
} else if cfg!(target_os = "macos") {
    &["libSDL2-2.0.0.dylib", "/opt/homebrew/lib/libSDL2-2.0.0.dylib", "/usr/local/lib/libSDL2-2.0.0.dylib"]
} else {
    &["libSDL2-2.0.so.0"]
};

/// Hints set before `SDL_Init`. On webOS the BACK key is delivered to the app
/// instead of being handled by the system, and the Magic Remote's pointer
/// hides after 5 s still, as in moonlight-tv, RetroArch and Stremio Origin:
/// left unset, it stays 5 minutes, and OK comes as its click all along.
pub const SDL_HINTS: &[(&str, &str)] = if WEBOS {
    &[("SDL_WEBOS_ACCESS_POLICY_KEYS_BACK", "true"), ("SDL_WEBOS_CURSOR_SLEEP_TIME", "5000")]
} else {
    &[]
};

pub const WINDOW: WindowConfig = if WEBOS {
    WindowConfig { title: "Mana", width: 1920, height: 1080, fullscreen: true, gles2: true }
} else {
    WindowConfig { title: "Mana", width: 1280, height: 720, fullscreen: false, gles2: false }
};

/// Font loaded at runtime from the system; nothing is bundled.
pub const FONT_CANDIDATES: &[&str] = if WEBOS {
    &["/usr/share/fonts/LG_Smart_UI-Regular.ttf", "/usr/share/fonts/DroidSans.ttf"]
} else {
    &[
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
};

/// Native webOS apps are launched without `EGL_PLATFORM` and `XDG_RUNTIME_DIR`;
/// SDL's Wayland backend needs both. Must run before any thread is spawned.
pub fn prepare_environment() {
    if !WEBOS {
        return;
    }
    let runtime_dir = "/tmp/xdg";
    let _ = std::fs::create_dir_all(runtime_dir);
    // SAFETY: called at the start of `main`, before any other thread exists.
    unsafe {
        if std::env::var_os("EGL_PLATFORM").is_none() {
            std::env::set_var("EGL_PLATFORM", "wayland");
        }
        if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
            std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
        }
    }
}

/// Where playback goes: the TV's Starfish pipeline, loaded on behalf of
/// the app launched (`APPID`). The desktop has no backend yet.
pub fn open_backend() -> OpenBackend {
    #[cfg(webos)]
    {
        const APP_ID_FALLBACK: &str = "dev.mana.app";
        let app_id = std::env::var("APPID").unwrap_or_else(|_| APP_ID_FALLBACK.into());
        std::sync::Arc::new(move || {
            let starfish = starfish::Starfish::open(SDL_CANDIDATES, app_id.clone())?;
            Ok(Box::new(starfish) as Box<dyn mana_core::playback::Backend>)
        })
    }
    #[cfg(not(webos))]
    std::sync::Arc::new(|| Err(mana_core::playback::BackendError::Unavailable))
}

/// Whether segments are decrypted with the CPU's AES instructions: on the
/// TV, when the kernel reports them (`HWCAP2_AES`), as on the B1's
/// Cortex-A73 in AArch32; the core only uses them once told.
pub fn use_aes_instructions() -> bool {
    #[cfg(webos)]
    {
        const AT_HWCAP2: std::ffi::c_ulong = 26;
        const HWCAP2_AES: std::ffi::c_ulong = 1 << 0;
        unsafe extern "C" {
            fn getauxval(kind: std::ffi::c_ulong) -> std::ffi::c_ulong;
        }
        // SAFETY: getauxval only reads the auxiliary vector (glibc 2.16).
        if unsafe { getauxval(AT_HWCAP2) } & HWCAP2_AES != 0 {
            // SAFETY: the kernel reports the instructions.
            unsafe { mana_core::live::hls::assume_aes_instructions() };
            return true;
        }
    }
    false
}

/// On the TV, sends stderr to the log file and logs fatal signals with a
/// backtrace (crash.c): an app started by the system has no stderr to read.
#[cfg(webos)]
pub fn install_crash_handler(log_file: &str) {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn mana_install_crash_handler(fd: std::ffi::c_int);
    }
    // Append mode, as the log's own handle: the log empties the file when
    // it grows too large.
    let Ok(file) = std::fs::OpenOptions::new().append(true).open(log_file) else { return };
    // SAFETY: the descriptor stays open for the life of the process.
    unsafe { mana_install_crash_handler(file.as_raw_fd()) };
    std::mem::forget(file);
}

/// Ends the process at once, with playback still unloading on its thread.
/// On the TV, without the exit handlers: the media libraries' static
/// destructors must not run under that thread.
pub fn exit_now() -> ! {
    #[cfg(webos)]
    {
        unsafe extern "C" {
            fn _exit(status: std::ffi::c_int) -> !;
        }
        // SAFETY: ends the process; the log is written line by line.
        unsafe { _exit(0) }
    }
    #[cfg(not(webos))]
    std::process::exit(0)
}
