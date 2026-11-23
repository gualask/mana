//! The channels to choose from, as models the interface shows, each with
//! the stream it plays.

pub mod device;
pub mod folder;
pub mod list;
pub mod m3u;
pub mod search;
pub mod update;

use std::borrow::Cow;
use std::sync::Arc;

use crate::language::Language;
use crate::live::playable::Playable;
use crate::source::{Selection, Source};
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
    /// When an event of a source's catalogue starts, in seconds since 1970.
    pub starts_at: Option<i64>,
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
    /// The catalogue of a source, read when the viewer opens it.
    Folder(Folder),
}

/// A source whose catalogue's items are the channels of a folder of the
/// list.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    /// The source's name in the list: the name of the catalogue's copy.
    pub name: String,
    pub source: Arc<Source>,
    /// The device identifier the source's flows read.
    pub device: String,
}

/// The folder of the copies, in the data directory: the list, what it
/// includes, the catalogues of its folders.
const COPIES: &str = "list";

/// Writes the copy `name` in `dir` whole, through a temporary file: a
/// power-off meanwhile leaves the previous one. Returns whether it was
/// written.
fn save(dir: &std::path::Path, name: &str, text: &str) -> bool {
    let (path, temporary) = (dir.join(name), dir.join(format!("{name}.new")));
    let saved = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&temporary, text)).and_then(|()| std::fs::rename(&temporary, &path));
    if let Err(error) = &saved {
        crate::log::log!("copy {} not saved: {error}", path.display());
    }
    saved.is_ok()
}

/// Puts the events of `channels` by their start, those without one after
/// them in their order: the order of a folder's events, and of the events
/// found.
pub fn by_start(channels: &mut [Channel]) {
    channels.sort_by_key(|channel| (channel.starts_at.is_none(), channel.starts_at));
}

impl Channel {
    /// The name the screens show: the list's, else the placeholder for a
    /// channel without one, in `language`.
    pub fn title(&self, language: Language) -> Cow<'_, str> {
        self.name.as_deref().map_or_else(|| Cow::Owned(text!(language, "channel-unnamed")), Cow::Borrowed)
    }
}
