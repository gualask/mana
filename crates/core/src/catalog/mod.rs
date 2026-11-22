//! The channels to choose from, as models the interface shows, each with
//! the source it plays.

pub mod device;
pub mod list;
pub mod m3u;
pub mod update;

use crate::live::source::Source;

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub name: String,
    /// URL or local path of its logo.
    pub logo: Option<String>,
    /// The group the list puts it in, if any.
    pub group: Option<String>,
    /// What it plays, or why it cannot be played: the list gives no stream,
    /// or a broken one.
    pub play: Result<Source, String>,
}
