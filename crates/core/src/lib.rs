//! What the TV and desktop apps share, with no interface and no platform:
//! the catalog, live sources down to a byte stream, playback from a byte
//! stream to the decoder each app provides, and the viewing session that
//! ties them together.

mod adts;
pub mod catalog;
pub mod language;
pub mod live;
pub mod log;
pub mod net;
pub mod playback;
pub mod session;
