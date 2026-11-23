//! The folders of the list: a source's catalogue, opened from its row on
//! the home screen or in the list over the video. Its events take the
//! place of the list's channels on both screens until Back on the home
//! screen; the copy shows at once, the source's answer replaces it.

use mana_core::catalog::folder::Update;
use mana_core::catalog::{Channel, Folder, Play};
use mana_core::language::{Language, Localized};
use mana_core::source::run::Failure;
use mana_core::text;

use super::App;
use super::channel_list::ChannelList;

/// The folder open, and the list it was opened from.
pub(super) struct OpenFolder {
    pub folder: Folder,
    /// The list's channels, shown again when the folder closes.
    list: Vec<Channel>,
    /// Why the source did not answer, the last time it was asked.
    failure: Option<Failure>,
}

impl App {
    /// Shows the events of `folder` in place of the list's channels: those
    /// of its copy at once, if any, while its source is asked when the copy
    /// is old.
    pub(super) fn open_folder(&mut self, folder: Folder) {
        let copy = self.folders.open(&folder).unwrap_or_default();
        let list = std::mem::replace(&mut self.channels, ChannelList::folder(&folder.source.name, copy));
        self.folder = Some(OpenFolder { folder, list: list.into_channels(), failure: None });
        self.focus = 0;
        self.home_first = 0;
    }

    /// Shows the list's channels again, the focus on the folder's row.
    pub(super) fn close_folder(&mut self) {
        let Some(open) = self.folder.take() else { return };
        self.channels = ChannelList::new(open.list);
        self.focus = folder_row(self.channels.channels(), &open.folder.name).unwrap_or(0);
        self.channels.set_focus(self.focus);
        self.home_first = 0;
    }

    /// The list's channels kept while a folder is open, shown again when
    /// it closes; None when no folder is open.
    pub(super) fn list_behind_folder(&mut self) -> Option<&mut Vec<Channel>> {
        self.folder.as_mut().map(|open| &mut open.list)
    }

    /// Asks the open folder's source again, if its events are old: called
    /// as the list over the video or the home screen opens.
    pub(super) fn refresh_folder(&mut self) {
        if let Some(open) = &self.folder {
            self.folders.refresh(&open.folder);
        }
    }

    /// Takes the answers of the sources: the open folder's events replace
    /// those shown, the focus kept on the same event.
    pub(super) fn take_catalogues(&mut self) {
        while let Some(Update { name, outcome }) = self.folders.poll() {
            let Some(open) = &mut self.folder else { continue };
            if open.folder.name != name {
                continue;
            }
            match outcome {
                Ok(channels) => {
                    open.failure = None;
                    self.focus = self.channels.replace(channels, self.focus).unwrap_or(0);
                }
                Err(failure) => open.failure = Some(failure),
            }
        }
    }

    /// The list's channels, also while a folder shows its events.
    pub(super) fn list_channels(&self) -> &[Channel] {
        match &self.folder {
            Some(open) => &open.list,
            None => self.channels.channels(),
        }
    }

    /// The open folder's name, as the list names it.
    pub(super) fn folder_title(&self) -> Option<&str> {
        self.folder.as_ref().map(|open| open.folder.source.name.as_str())
    }

    /// What the screens say about the open folder: its source being asked
    /// while there are no events, why it did not answer, or that it has no
    /// events now.
    pub(super) fn folder_status(&self, language: Language) -> Option<String> {
        let open = self.folder.as_ref()?;
        let (empty, asking) = (self.channels.len() == 0, self.folders.asking(&open.folder.name));
        match &open.failure {
            _ if empty && asking => Some(text!(language, "folder-loading")),
            Some(failure) if empty => Some(text!(language, "folder-unavailable", reason = failure.text(language))),
            Some(failure) => Some(text!(language, "folder-saved", reason = failure.text(language))),
            None if empty => Some(text!(language, "folder-empty")),
            None => None,
        }
    }
}

/// The row of the folder `name` among `channels`.
fn folder_row(channels: &[Channel], name: &str) -> Option<usize> {
    channels.iter().position(|c| matches!(&c.play, Ok(Play::Folder(folder)) if folder.name == name))
}
