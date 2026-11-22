//! Safe wrapper over `shim.c`, which links the FFmpeg that `build.rs`
//! chooses: on the TV the build bundled with the app (tools/webos/ffmpeg.py),
//! on the desktop the one `MANA_FFMPEG` names.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Read as _;
use std::path::Path;

use super::Input;

const READ_EOF: c_int = 0;
const READ_VIDEO: c_int = 1;
const READ_AUDIO: c_int = 2;
const NO_TIMESTAMP: i64 = i64::MIN;
/// FFmpeg's AVERROR(FFERRTAG(...)) codes for the read callback.
const AVERROR_EOF: c_int = -i32::from_le_bytes(*b"EOF ");
const AVERROR_EXIT: c_int = -i32::from_le_bytes(*b"EXIT");

// Mirror MANA_CODEC_* in the shim.
pub const CODEC_H264: i32 = 1;
pub const CODEC_HEVC: i32 = 2;
pub const CODEC_AAC: i32 = 3;
// MPEG audio arrives transcoded to AAC.
pub const CODEC_AC3: i32 = 6;
pub const CODEC_EAC3: i32 = 7;
const NO_AUDIO: i32 = -1;
const NO_VIDEO: i32 = -1;

/// Mirrors `mana_info`.
#[repr(C)]
pub struct Info {
    pub width: i32,
    pub height: i32,
    pub video_codec: i32,
    pub audio_codec: i32,
    video_name: [c_char; 32],
    audio_name: [c_char; 32],
}

impl Info {
    pub fn has_audio(&self) -> bool {
        self.audio_codec != NO_AUDIO
    }

    pub fn describe(&self) -> String {
        format!("{} · {}", self.describe_video(), self.describe_audio())
    }

    pub fn describe_video(&self) -> String {
        if self.video_codec == NO_VIDEO {
            return "nessun video".into();
        }
        format!("{} {}×{}", name(&self.video_name), self.width, self.height)
    }

    pub fn describe_audio(&self) -> String {
        if self.has_audio() { name(&self.audio_name) } else { "nessun audio".into() }
    }
}

fn name(raw: &[c_char; 32]) -> String {
    // SAFETY: the shim writes NUL-terminated names with snprintf.
    unsafe { CStr::from_ptr(raw.as_ptr()) }.to_string_lossy().into_owned()
}

/// Mirrors `mana_packet`.
#[repr(C)]
struct RawPacket {
    data: *const u8,
    size: c_int,
    keyframe: c_int,
    pts_us: i64,
    dts_us: i64,
}

unsafe extern "C" {
    fn mana_media_open(url: *const c_char, info: *mut Info, err: *mut c_char, err_size: c_int) -> *mut c_void;
    fn mana_media_open_stream(
        read: unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int,
        opaque: *mut c_void,
        info: *mut Info,
        err: *mut c_char,
        err_size: c_int,
    ) -> *mut c_void;
    fn mana_media_read(media: *mut c_void, out: *mut RawPacket) -> c_int;
    fn mana_media_rewind(media: *mut c_void) -> c_int;
    fn mana_media_close(media: *mut c_void);
    fn mana_error_text(code: c_int, out: *mut c_char, out_size: c_int);
}

pub enum Read<'a> {
    /// Annex B access unit as stored in the source.
    Video { data: &'a [u8], pts_us: Option<i64>, dts_us: Option<i64>, keyframe: bool },
    /// One AAC frame (ADTS when the source is MPEG-TS).
    Audio { data: &'a [u8], pts_us: Option<i64> },
    Eof,
}

pub struct Media {
    /// Null only after a failed reopen.
    raw: *mut c_void,
    pub info: Info,
    /// Read by the shim through `read_stream`; dropped after the shim closes.
    /// Boxed twice so that the shim holds a thin pointer.
    stream: Option<Box<Box<dyn Input>>>,
}

// SAFETY: the handle is used from one thread at a time (moved into the feeder).
unsafe impl Send for Media {}

impl Media {
    pub fn open(path: &Path) -> Result<Self, String> {
        let url = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: Info is plain data; the shim zeroes and fills it.
        let mut info: Info = unsafe { std::mem::zeroed() };
        let mut err = [0 as c_char; 256];
        // SAFETY: all pointers are valid for the duration of the call.
        let raw = unsafe { mana_media_open(url.as_ptr(), &mut info, err.as_mut_ptr(), err.len() as c_int) };
        if raw.is_null() {
            // SAFETY: the shim NUL-terminates `err` on failure.
            return Err(unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned());
        }
        Ok(Self { raw, info, stream: None })
    }

    /// Demuxes an MPEG-TS or fragmented MP4 byte stream, one section at a
    /// time (see `next_section`).
    pub fn open_stream(stream: Box<dyn Input>) -> Result<Self, String> {
        let mut stream = Box::new(stream);
        let (raw, info) = open_section(&mut stream)?;
        Ok(Self { raw, info, stream: Some(stream) })
    }

    /// See `Input::discontinuity_sequence`; 0 for files.
    pub fn discontinuity_sequence(&self) -> u64 {
        self.stream.as_ref().map_or(0, |s| s.discontinuity_sequence())
    }

    /// After `read` returned end of file: when the stream continues with a
    /// new section, reopens the demuxer on it, with the section's own
    /// streams and formats in `info`, and returns true.
    pub fn next_section(&mut self) -> Result<bool, String> {
        let Some(stream) = self.stream.as_mut() else { return Ok(false) };
        if !stream.next_section() {
            return Ok(false);
        }
        // SAFETY: `raw` is a live handle, closed once and replaced below.
        unsafe { mana_media_close(self.raw) };
        self.raw = std::ptr::null_mut();
        (self.raw, self.info) = open_section(stream)?;
        Ok(true)
    }

    /// The returned data is valid until the next call on this `Media`.
    pub fn read(&mut self) -> Result<Read<'_>, String> {
        let mut packet = RawPacket { data: std::ptr::null(), size: 0, keyframe: 0, pts_us: 0, dts_us: 0 };
        // SAFETY: `raw` is a live handle; `packet` is filled by the shim.
        let kind = unsafe { mana_media_read(self.raw, &mut packet) };
        let timestamp = |ts: i64| (ts != NO_TIMESTAMP).then_some(ts);
        let data = || {
            // SAFETY: the shim guarantees `size` readable bytes at `data`
            // until the next read, which needs `&mut self`.
            unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) }
        };
        match kind {
            READ_VIDEO => Ok(Read::Video {
                data: data(),
                pts_us: timestamp(packet.pts_us),
                dts_us: timestamp(packet.dts_us),
                keyframe: packet.keyframe != 0,
            }),
            READ_AUDIO => Ok(Read::Audio { data: data(), pts_us: timestamp(packet.pts_us) }),
            READ_EOF => Ok(Read::Eof),
            code => Err(error_text(code)),
        }
    }

    pub fn rewind(&mut self) -> Result<(), String> {
        // SAFETY: `raw` is a live handle.
        let code = unsafe { mana_media_rewind(self.raw) };
        if code < 0 { Err(error_text(code)) } else { Ok(()) }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: `raw` came from mana_media_open* and is closed once, before
            // the stream it reads from is dropped.
            unsafe { mana_media_close(self.raw) }
        }
        self.stream.take();
    }
}

/// Opens the shim on the stream's current section.
fn open_section(stream: &mut Box<Box<dyn Input>>) -> Result<(*mut c_void, Info), String> {
    // SAFETY: as in `Media::open`.
    let mut info: Info = unsafe { std::mem::zeroed() };
    let mut err = [0 as c_char; 256];
    let opaque = (&mut **stream as *mut Box<dyn Input>).cast::<c_void>();
    // SAFETY: `opaque` points into the outer box, which stays in the Media
    // and outlives the shim handle (closed first in Drop and on reopening).
    let raw = unsafe { mana_media_open_stream(read_stream, opaque, &mut info, err.as_mut_ptr(), err.len() as c_int) };
    if raw.is_null() {
        // SAFETY: the shim NUL-terminates `err` on failure.
        return Err(unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().into_owned());
    }
    Ok((raw, info))
}

/// AVIO read callback: fills `buf` from the `Input` behind `opaque`.
unsafe extern "C" fn read_stream(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    // SAFETY: `opaque` is the boxed Input registered in `open_section`, and
    // FFmpeg passes a writable buffer of `size` bytes.
    let (stream, buf) =
        unsafe { (&mut *opaque.cast::<Box<dyn Input>>(), std::slice::from_raw_parts_mut(buf, size as usize)) };
    match stream.read(buf) {
        Ok(0) => AVERROR_EOF,
        Ok(n) => n as c_int,
        Err(_) => AVERROR_EXIT,
    }
}

fn error_text(code: c_int) -> String {
    let mut text = [0 as c_char; 128];
    // SAFETY: av_strerror writes a NUL-terminated string into the buffer.
    unsafe {
        mana_error_text(code, text.as_mut_ptr(), text.len() as c_int);
        format!("FFmpeg {code}: {}", CStr::from_ptr(text.as_ptr()).to_string_lossy())
    }
}
