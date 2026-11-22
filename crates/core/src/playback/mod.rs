//! From a byte stream to a decoder: FFmpeg demuxes local files or live
//! streams, the timeline makes their timestamps continuous, and the feeder
//! hands packets to a decoder at the pace it presents them. The decoder is
//! the client's: Starfish on the TV, another backend on the desktop.

pub mod demux;
#[cfg(test)]
pub mod fake;
pub mod feeder;
pub mod ffmpeg;
pub mod reader;
mod settings;
mod silence;
pub mod timeline;

pub use demux::{AudioCodec, Formats, VideoCodec};
pub use settings::Settings;

/// A decoder that presents what it is fed. Every call comes from the
/// feeding thread; dropping it unloads the decoder.
pub trait Backend {
    /// Loads the decoder for `formats`; playback starts while feeding.
    fn load(&mut self, formats: &Formats) -> Result<(), String>;

    /// Hands over one access unit to present at `pts_ns`. False when the
    /// decoder's buffer is full and the caller must retry.
    fn feed_video(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String>;

    /// As `feed_video`, for one audio frame.
    fn feed_audio(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String>;

    /// Presentation time of the last frame displayed, in nanoseconds.
    fn presented_ns(&self) -> Option<i64>;

    /// Times the decoder reported its input running low.
    fn buffer_low(&self) -> u64;

    /// Pauses presentation and its clock; false when the decoder cannot or
    /// refuses.
    fn pause(&mut self) -> bool;

    /// Resumes presentation after `pause`; false when the decoder refuses.
    fn resume(&mut self) -> bool;
}

/// Makes a new backend, on the feeding thread, each time a decoder is
/// loaded: for each opening of a source and each change of formats.
pub type OpenBackend = std::sync::Arc<dyn Fn() -> Result<Box<dyn Backend>, String> + Send + Sync>;

/// A byte stream cut into sections, read on the demuxing thread. At the
/// end of a section streams and formats may change: the demuxer starts
/// afresh on the next one.
pub trait Input: std::io::Read + Send {
    /// After reading end of file: whether it was the end of a section, with
    /// another one now readable.
    fn next_section(&mut self) -> bool;

    /// Identifies the timeline of the section being read: timestamps of two
    /// inputs compare only when this is the same.
    fn discontinuity_sequence(&self) -> u64;
}

/// Why playing an opening of a source failed.
#[derive(Debug, PartialEq)]
pub enum Error {
    /// Opening again cannot help: a video codec the decoder cannot decode,
    /// or no decoder to play on.
    Unsupported(String),
    /// The demuxer or the decoder failed.
    Failed(String),
}

impl From<String> for Error {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) | Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// Where the feeding is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// No decoder loaded yet.
    #[default]
    Opening,
    Playing,
    /// Playing out the end of a file.
    Draining,
}

/// What the feeding shows the screens: its phase, the formats playing and
/// the last frame shown.
#[derive(Clone, Default, Debug)]
pub struct Stats {
    pub phase: Phase,
    /// The formats of the section playing.
    pub formats: Option<Formats>,
    /// Timeline position of the last frame the decoder displayed.
    pub presented_ms: Option<i64>,
}
