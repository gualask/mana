//! A minimal HLS client for live streams, which also plays a continuous
//! MPEG-TS stream served in place of a playlist.

mod continuous;
mod decrypt;
mod download;
mod downloader;
mod ladder;
mod packed_audio;
mod playlist;
mod progress;
mod segment;
mod stream;
mod ts;

#[cfg(target_arch = "arm")]
pub use decrypt::assume_aes_instructions;
pub use download::Live;
pub use stream::Stop;
