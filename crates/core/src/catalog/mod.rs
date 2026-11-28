//! The channels to choose from, as models the interface shows, each with
//! the stream it plays.

pub mod device;
pub mod folder;
pub mod list;
pub mod logos;
pub mod m3u;
pub mod search;
pub mod update;

use std::borrow::Cow;
use std::sync::Arc;

use crate::language::Language;
use logos::Placeholder;
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

/// The name of a file kept for `url`, ending in `suffix`: a hash of the
/// URL (FNV-1a), which may be too long for a name.
fn url_file_name(url: &str, suffix: &str) -> String {
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}{suffix}")
}

/// Writes the copy `name` in `dir` whole (`files::write_whole`). Returns
/// whether it was written.
fn save(dir: &std::path::Path, name: &str, text: &str) -> bool {
    let path = dir.join(name);
    let saved = crate::files::write_whole(&path, text.as_bytes());
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

    /// What the screens show in its logo's box while it has none: a
    /// folder's mark for a folder, else the first letter or digit of its
    /// title, capital.
    pub fn placeholder(&self, language: Language) -> Placeholder {
        if matches!(self.play, Ok(Play::Folder(_))) {
            return Placeholder::Folder;
        }
        let initial = self.title(language).chars().find(|c| c.is_alphanumeric());
        Placeholder::Initial(initial.map(|c| c.to_uppercase().collect()).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_s_placeholder_is_its_initial_or_a_folder_s_mark() {
        let channel = |name: Option<&str>| Channel { name: name.map(str::to_owned), logo: None, group: None, starts_at: None, play: Err(ChannelError::NoStream) };
        let initial = |name: Option<&str>, language| match channel(name).placeholder(language) {
            Placeholder::Initial(initial) => initial,
            Placeholder::Folder => panic!("not a folder"),
        };
        assert_eq!(initial(Some("rai 1"), Language::English), "R");
        assert_eq!(initial(Some("«7 Gold»"), Language::English), "7");
        assert_eq!(initial(Some("équipe"), Language::English), "É");
        assert_eq!(initial(Some("· ·"), Language::English), "");
        let unnamed = channel(None).title(Language::Italian).chars().next().unwrap().to_uppercase().to_string();
        assert_eq!(initial(None, Language::Italian), unnamed);
    }
}
