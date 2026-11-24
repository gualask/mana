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

use crate::language::{Language, Localized, display_in_english};
use crate::text;

/// A decoder that presents what it is fed. Every call comes from the
/// feeding thread; dropping it unloads the decoder.
pub trait Backend {
    /// Loads the decoder for `formats`; playback starts while feeding.
    fn load(&mut self, formats: &Formats) -> Result<(), BackendError>;

    /// Hands over one access unit to present at `pts_ns`. False when the
    /// decoder's buffer is full and the caller must retry.
    fn feed_video(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, BackendError>;

    /// As `feed_video`, for one audio frame.
    fn feed_audio(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, BackendError>;

    /// Presentation time of the last frame displayed, in nanoseconds.
    fn presented_ns(&self) -> Option<i64>;

    /// Times the decoder reported its input running low.
    fn buffer_low(&self) -> u64;

    /// Pauses presentation and its clock; false when the decoder cannot or
    /// refuses.
    fn pause(&mut self) -> bool;

    /// Resumes presentation after `pause`; false when the decoder refuses.
    fn resume(&mut self) -> bool;

    /// No more data will come: a decoder that holds pictures back, to
    /// decode on several threads, lets them out. Nothing to do for one that
    /// presents all it was fed.
    fn finish(&mut self) {}
}

/// Makes a new backend, on the feeding thread, each time a decoder is
/// loaded: for each opening of a source and each change of formats.
pub type OpenBackend = std::sync::Arc<dyn Fn() -> Result<Box<dyn Backend>, BackendError> + Send + Sync>;

/// Why a backend could not open or failed: the error of the `Backend`
/// port, which each backend fills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// This build has no decoder to play on.
    Unavailable,
    /// The viewing stopped before the decoder opened.
    Stopped,
    /// The decoder could not open, as the system says: a library or a
    /// function missing.
    Open(String),
    /// The decoder's video window was not created.
    NoWindow,
    /// The decoder refused the stream's formats.
    LoadRefused,
    /// The decoder did not finish loading in time.
    LoadTimeout,
    /// The decoder refused to play.
    PlayRefused,
    /// What the decoder reports: an error event, or its answer to data.
    Reported(String),
}

impl Localized for BackendError {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Unavailable => text!(language, "playback-unavailable"),
            Self::Stopped => text!(language, "playback-stopped"),
            Self::Open(detail) => text!(language, "backend-open", detail = detail),
            Self::NoWindow => text!(language, "backend-no-window"),
            Self::LoadRefused => text!(language, "backend-load-refused"),
            Self::LoadTimeout => text!(language, "backend-load-timeout"),
            Self::PlayRefused => text!(language, "backend-play-refused"),
            Self::Reported(detail) => text!(language, "backend-reported", detail = detail),
        }
    }
}

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A video the decoder cannot decode, as FFmpeg describes it; none when
    /// the stream has no video.
    VideoCodec(Option<String>),
    /// No decoder to play on.
    NoDecoder(BackendError),
    Demuxer(DemuxError),
    /// The demuxing thread ended.
    DemuxerGone,
    /// The decoder failed.
    Decoder(BackendError),
    /// The decoder refused data for this long.
    DecoderRefuses(std::time::Duration),
    /// The decoder showed no new frame for this long.
    DecoderFrozen(std::time::Duration),
    /// The decoder refused to resume after a pause.
    DecoderWontResume,
    /// A thread could not start, as the system says.
    Thread(String),
}

impl Error {
    /// Whether opening again cannot help: a video codec the decoder cannot
    /// decode, or no decoder to play on.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::VideoCodec(_) | Self::NoDecoder(_))
    }
}

impl From<BackendError> for Error {
    fn from(error: BackendError) -> Self {
        Self::Decoder(error)
    }
}

impl Localized for Error {
    fn text(&self, language: Language) -> String {
        match self {
            Self::VideoCodec(Some(video)) => text!(language, "playback-video-codec", video = video),
            Self::VideoCodec(None) => text!(language, "playback-no-video"),
            Self::NoDecoder(error) | Self::Decoder(error) => error.text(language),
            Self::Demuxer(error) => error.text(language),
            Self::Thread(detail) => detail.clone(),
            Self::DemuxerGone => text!(language, "playback-demuxer-gone"),
            Self::DecoderRefuses(since) => text!(language, "playback-decoder-refuses", ms = since.as_millis()),
            Self::DecoderFrozen(since) => text!(language, "playback-decoder-frozen", seconds = since.as_secs()),
            Self::DecoderWontResume => text!(language, "playback-decoder-wont-resume"),
        }
    }
}

/// Why the demuxer failed: the step, with what FFmpeg says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DemuxError {
    /// Opening the input.
    Open(String),
    /// Reading the formats of its streams.
    StreamInfo(String),
    /// Turning an MP4's video into Annex B, as the decoder takes it.
    AnnexB(String),
    /// An MP4 whose AAC has no configuration.
    AacConfig,
    /// Reading a packet.
    Read(String),
    /// Going back to the start of a file, to loop it.
    Rewind(String),
}

impl Localized for DemuxError {
    fn text(&self, language: Language) -> String {
        match self {
            Self::Open(detail) => text!(language, "demux-open", detail = detail),
            Self::StreamInfo(detail) => text!(language, "demux-stream-info", detail = detail),
            Self::AnnexB(detail) => text!(language, "demux-annexb", detail = detail),
            Self::AacConfig => text!(language, "demux-aac-config"),
            Self::Read(detail) => text!(language, "demux-read", detail = detail),
            Self::Rewind(detail) => text!(language, "demux-rewind", detail = detail),
        }
    }
}

display_in_english!(Error, BackendError, DemuxError);

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
