//! The channels to choose from, as models the interface shows, each with
//! the stream it plays.

pub mod device;
pub mod list;
pub mod m3u;
pub mod update;

use std::borrow::Cow;

use crate::language::Language;
use crate::live::playable::Playable;
use crate::source::Selection;
use crate::text;
pub use list::ChannelError;

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// None when the list gives none: the screens show a placeholder.
    pub name: Option<String>,
    /// URL or local path of its logo.
    pub logo: Option<String>,
    /// The group the list puts it in, if any.
    pub group: Option<String>,
    /// What it plays, or why it cannot be played: the list gives no stream,
    /// or a broken one.
    pub play: Result<Play, ChannelError>,
}

/// What a channel plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Play {
    /// A stream at a URL the list gives.
    Stream(Playable),
    /// An item of a source, which the session resolves when it opens.
    Source(Selection),
}

impl Channel {
    /// The name the screens show: the list's, else the placeholder for a
    /// channel without one, in `language`.
    pub fn title(&self, language: Language) -> Cow<'_, str> {
        self.name.as_deref().map_or_else(|| Cow::Owned(text!(language, "channel-unnamed")), Cow::Borrowed)
    }
}
