//! The TV's playback backend: LG Starfish media pipeline (`libplayerAPIs`,
//! webOS 5+), driven as a `BUFFERSTREAM`: we feed elementary streams, the TV
//! decodes, synchronizes and presents them on an SDL exported video window.
//!
//! The C++ class is called through its mangled symbols, loaded at runtime.
//! Interface facts (payloads, event codes) come from webosbrew's
//! webos-userland headers and from observing NDL DirectMedia on the TV: NDL's
//! own `DIRECTMEDIA-ES-PLAYER` transport shows a green picture for H.264 on
//! the B1. Every call must come from the same thread.

mod events;

use std::ffi::{CStr, CString, c_char, c_int, c_longlong, c_void};
use std::time::{Duration, Instant};

use libloading::Library;

use mana_core::log::{debug, log};
use mana_core::playback::{AudioCodec, Backend, Formats, VideoCodec};
use events::{SHARED, Shared, on_event, wait_for};

const LIBRARY: &str = "libplayerAPIs.so.1";

const ES_VIDEO: u8 = 1;
const ES_AUDIO: u8 = 2;

const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const UNLOAD_TIMEOUT: Duration = Duration::from_secs(3);
/// Room for the C++ object; its real size is not published (> 80 bytes).
const OBJECT_WORDS: usize = 512;
/// SDL exported window type for video.
const EXPORTED_WINDOW_VIDEO: c_int = 0;
const SCREEN: SdlRect = SdlRect { x: 0, y: 0, w: 1920, h: 1080 };

/// `std::string` under libstdc++'s C++11 ABI on 32-bit ARM.
#[repr(C)]
struct StdString {
    data: *mut c_char,
    len: usize,
    local: [u8; 16],
}

#[repr(C)]
struct SdlRect {
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
}

type Callback = unsafe extern "C" fn(c_int, c_longlong, *const c_char);

struct Api {
    construct: unsafe extern "C" fn(*mut c_void, *const c_char),
    destruct: unsafe extern "C" fn(*mut c_void),
    load: unsafe extern "C" fn(*mut c_void, *const c_char, Callback) -> bool,
    play: unsafe extern "C" fn(*mut c_void) -> bool,
    pause: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
    unload: unsafe extern "C" fn(*mut c_void) -> bool,
    /// Returns `std::string` by value: under the ARM Itanium C++ ABI the
    /// result pointer comes first, then `this`.
    feed: unsafe extern "C" fn(*mut StdString, *mut c_void, *const c_char),
    delete: unsafe extern "C" fn(*mut c_void),
    create_window: unsafe extern "C" fn(c_int) -> *const c_char,
    set_window: unsafe extern "C" fn(*const c_char, *const SdlRect, *const SdlRect) -> bool,
    destroy_window: unsafe extern "C" fn(*const c_char),
}

pub struct Starfish {
    api: Api,
    /// The C++ object, destructed once unloading is confirmed: an object left
    /// alive keeps its hardware decoder, and the B1 has only two.
    object: Box<[u64; OBJECT_WORDS]>,
    window: CString,
    app_id: String,
    /// When Load was accepted.
    loaded: Option<Instant>,
    playing: bool,
    _player: Library,
    _sdl: Library,
}

impl Starfish {
    /// Loads the library, creates the video window and the pipeline object.
    /// `sdl` lists where the SDL library the app runs on is found; the
    /// pipeline is loaded on behalf of `app_id`.
    pub fn open(sdl: &[&str], app_id: String) -> Result<Self, String> {
        // SAFETY: firmware libraries with no load-time preconditions; SDL is
        // already loaded by the platform layer, this only takes a reference.
        let player = unsafe { Library::new(LIBRARY) }.map_err(|e| format!("{LIBRARY}: {e}"))?;
        let sdl = sdl
            .iter()
            // SAFETY: as above.
            .find_map(|path| unsafe { Library::new(*path) }.ok())
            .ok_or("SDL non trovata")?;
        // SAFETY: symbols are cast to the signatures of StarfishMediaAPIs.h
        // (webos-userland) and LG's SDL webOS extensions.
        let api = unsafe {
            macro_rules! sym {
                ($lib:expr, $name:literal) => {
                    *$lib.get(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $name))?
                };
            }
            Api {
                construct: sym!(player, "_ZN17StarfishMediaAPIsC1EPKc"),
                destruct: sym!(player, "_ZN17StarfishMediaAPIsD1Ev"),
                load: sym!(player, "_ZN17StarfishMediaAPIs4LoadEPKcPFvixS1_E"),
                play: sym!(player, "_ZN17StarfishMediaAPIs4PlayEv"),
                pause: player.get(b"_ZN17StarfishMediaAPIs5PauseEv\0").ok().map(|symbol| *symbol),
                unload: sym!(player, "_ZN17StarfishMediaAPIs6UnloadEv"),
                feed: sym!(player, "_ZN17StarfishMediaAPIs4FeedB5cxx11EPKc"),
                delete: sym!(player, "_ZdlPv"),
                create_window: sym!(sdl, "SDL_webOSCreateExportedWindow"),
                set_window: sym!(sdl, "SDL_webOSSetExportedWindow"),
                destroy_window: sym!(sdl, "SDL_webOSDestroyExportedWindow"),
            }
        };
        // SAFETY: plain C calls; the id is a NUL-terminated string owned by SDL.
        let window = unsafe {
            let id = (api.create_window)(EXPORTED_WINDOW_VIDEO);
            if id.is_null() {
                return Err("finestra video esportata non creata".into());
            }
            let id = CStr::from_ptr(id).to_owned();
            (api.set_window)(id.as_ptr(), &SCREEN, &SCREEN);
            id
        };
        let mut object = Box::new([0u64; OBJECT_WORDS]);
        // SAFETY: `object` is zeroed, 8-byte aligned and larger than the class.
        unsafe { (api.construct)(object.as_mut_ptr().cast(), std::ptr::null()) };
        *SHARED.lock().unwrap() = Shared::default();
        Ok(Self { api, object, window, app_id, loaded: None, playing: false, _player: player, _sdl: sdl })
    }

    /// With audio, loading completes only once data has arrived, so feeding
    /// starts right after Load and Play follows the completion event.
    fn play_when_loaded(&mut self) -> Result<(), String> {
        let Some(loaded_at) = self.loaded.filter(|_| !self.playing) else { return Ok(()) };
        let completed = {
            let mut shared = SHARED.lock().unwrap();
            if let Some(error) = shared.error.take() {
                return Err(format!("Starfish: {error}"));
            }
            shared.load_completed
        };
        if !completed {
            return if loaded_at.elapsed() > LOAD_TIMEOUT {
                Err("Starfish: caricamento non completato".into())
            } else {
                Ok(())
            };
        }
        // SAFETY: same thread as every other call.
        if !unsafe { (self.api.play)(self.this()) } {
            return Err("Starfish Play rifiutato".into());
        }
        self.playing = true;
        log!("Starfish Play dopo {} ms", loaded_at.elapsed().as_millis());
        Ok(())
    }

    fn feed(&mut self, data: &[u8], pts_ns: i64, es: u8) -> Result<bool, String> {
        self.play_when_loaded()?;
        let payload = format!(
            r#"{{"bufferAddr":"{:#x}","bufferSize":{},"pts":{pts_ns},"esData":{es}}}"#,
            data.as_ptr() as usize,
            data.len()
        );
        let payload = CString::new(payload).map_err(|e| e.to_string())?;
        let mut result = StdString { data: std::ptr::null_mut(), len: 0, local: [0; 16] };
        // SAFETY: the pipeline copies the buffer before returning and
        // constructs `result` in place; we free it below as libstdc++ would.
        let reply = unsafe {
            (self.api.feed)(&mut result, self.this(), payload.as_ptr());
            let bytes = std::slice::from_raw_parts(result.data as *const u8, result.len);
            let reply = String::from_utf8_lossy(bytes).into_owned();
            if result.data.cast::<u8>() != result.local.as_mut_ptr() {
                (self.api.delete)(result.data.cast());
            }
            reply
        };
        // The reply is JSON such as {"returnValue":"Ok"}.
        if reply.contains(r#""Ok""#) {
            Ok(true)
        } else if reply.contains("BufferFull") {
            Ok(false)
        } else {
            Err(format!("Starfish Feed: {reply}"))
        }
    }

    fn this(&self) -> *mut c_void {
        self.object.as_ptr() as *mut c_void
    }
}

impl Backend for Starfish {
    fn load(&mut self, formats: &Formats) -> Result<(), String> {
        let payload = load_payload(formats, &self.app_id, &self.window.to_string_lossy());
        debug!("Starfish Load: {payload}");
        let payload = CString::new(payload).map_err(|e| e.to_string())?;
        // SAFETY: same thread for every call; the payload outlives the call.
        if !unsafe { (self.api.load)(self.this(), payload.as_ptr(), on_event) } {
            return Err("Starfish Load rifiutato".into());
        }
        self.loaded = Some(Instant::now());
        Ok(())
    }

    fn feed_video(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String> {
        self.feed(data, pts_ns, ES_VIDEO)
    }

    fn feed_audio(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String> {
        self.feed(data, pts_ns, ES_AUDIO)
    }

    fn presented_ns(&self) -> Option<i64> {
        SHARED.lock().unwrap().presented_ns
    }

    fn buffer_low(&self) -> u64 {
        SHARED.lock().unwrap().buffer_low
    }

    fn pause(&mut self) -> bool {
        // SAFETY: same thread as every other call.
        self.playing && self.api.pause.is_some_and(|pause| unsafe { pause(self.this()) })
    }

    fn resume(&mut self) -> bool {
        // SAFETY: same thread as every other call.
        self.playing && unsafe { (self.api.play)(self.this()) }
    }
}

/// The Load payload of a BUFFERSTREAM pipeline for `formats`, in the window
/// `window` of the app `app_id`.
fn load_payload(formats: &Formats, app_id: &str, window: &str) -> String {
    let codec = match formats.video {
        VideoCodec::H264 => "H264",
        VideoCodec::H265 => "H265",
    };
    // Names from the pipeline's own codec table (libpf). MPEG audio is
    // listed there too, but the pipeline fails on it as a stream ("MPEG":
    // crash, "MP3": playing error): the demuxer transcodes it to AAC.
    let audio_codec = match formats.audio {
        Some(AudioCodec::Aac) => r#","audio":"AAC""#,
        Some(AudioCodec::Ac3) => r#","audio":"AC3""#,
        // "EAC3" is accepted but its audio never reaches the decoder.
        Some(AudioCodec::Eac3) => r#","audio":"AC3 PLUS""#,
        None => "",
    };
    format!(
        concat!(
            r#"{{"args":[{{"mediaTransportType":"BUFFERSTREAM","option":{{"#,
            r#""appId":"{app_id}","windowId":"{window}","#,
            r#""externalStreamingInfo":{{"contents":{{"codec":{{"video":"{codec}"{audio_codec}}},"#,
            r#""esInfo":{{"ptsToDecode":0,"videoWidth":{width},"videoHeight":{height},"pauseAtDecodeTime":false}}}}}},"#,
            r#""adaptiveStreaming":{{"maxWidth":1920,"maxHeight":1080,"maxFrameRate":60}}}}}}]}}"#,
        ),
        app_id = app_id,
        window = window,
        codec = codec,
        audio_codec = audio_codec,
        width = formats.width,
        height = formats.height,
    )
}


impl Drop for Starfish {
    fn drop(&mut self) {
        // SAFETY: same thread as load; the window outlives the pipeline. The
        // object is destructed only once unloading is confirmed: until then
        // the pipeline may still call back into it.
        unsafe {
            let unloaded = self.loaded.is_none() || {
                (self.api.unload)(self.this());
                wait_for(|s| s.unload_completed, UNLOAD_TIMEOUT)
            };
            if unloaded {
                (self.api.destruct)(self.this());
            } else {
                log!("Starfish: scaricamento non confermato, oggetto lasciato in memoria");
                std::mem::forget(std::mem::replace(&mut self.object, Box::new([0; OBJECT_WORDS])));
            }
            (self.api.destroy_window)(self.window.as_ptr());
        }
    }
}
