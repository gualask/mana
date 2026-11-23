//! Where the channels come from (the tests' lists, the saved copy, the
//! downloads of the list) and how a download replaces those shown.

use std::path::Path;

use mana_core::catalog::update::{self, Updater};
use mana_core::catalog::{Channel, m3u};
use mana_core::log::log;
use url::Url;

use super::App;
use super::channel_list::ChannelList;
use crate::platform::DATA_DIR;

/// The state of the channel list, as the screens say it.
pub(super) enum ListState<'a> {
    /// The tests' list or the made-up channels, never downloaded.
    Tests(usize),
    Channels(usize),
    /// No channels yet, and a download asked has not ended.
    Downloading,
    /// No channels, and why the last download failed.
    Unavailable(&'a update::Failure),
    /// No URL in the settings, and none chosen from the phone.
    NotSet,
    /// A list downloaded without channels.
    Empty,
}

/// Made-up channels for the interface test, when there is no other list.
const TEST_CHANNELS: usize = 5000;
/// The channel list of the tests, copied over SSH (dev/tv/channels.py):
/// when present, it replaces the list.
pub(super) const CHANNELS_FILE: &str = "/tmp/mana-channels.m3u";

impl App {
    /// Hands the URL the phone sent to the updater, and takes each list
    /// downloaded, once it arrives, in place of the one shown; call at every
    /// turn of the event loop.
    pub fn refresh(&mut self) {
        self.languages.refresh();
        self.take_phone_request();
        while let Some(update) = self.updater.as_mut().and_then(Updater::poll) {
            self.answer_phone(&update);
            match update.outcome {
                update::Outcome::Channels { channels, .. } => {
                    self.list_failure = None;
                    self.replace_channels(channels);
                }
                update::Outcome::Failed(reason) => self.list_failure = Some(reason),
            }
        }
        self.sync_page();
    }

    /// Shows `channels` instead, on both screens, each focus kept on the
    /// channel of the same name, or on the same clip.
    fn replace_channels(&mut self, channels: Vec<Channel>) {
        let before = self.channels.len();
        let focused = self.channels.channels().get(self.focus).map(|c| c.name.clone());
        self.channels.replace(channels);
        self.focus = match focused {
            Some(name) => name.and_then(|name| self.channels.position(&name)).unwrap_or(0),
            // The first clip was focused only because no channel had come
            // yet: the screen opens on the first channel.
            None if before == 0 && self.focus == 0 => 0,
            None => self.focus - before + self.channels.len(),
        };
    }

    /// The list's state: its channels, else a download still running, which
    /// is newer than the last failure, else that failure.
    pub(super) fn list_state(&self) -> ListState<'_> {
        let count = self.channels.len();
        let Some(updater) = &self.updater else { return ListState::Tests(count) };
        match &self.list_failure {
            _ if count > 0 => ListState::Channels(count),
            _ if updater.downloading() => ListState::Downloading,
            Some(reason) => ListState::Unavailable(reason),
            None if updater.url().is_none() => ListState::NotSet,
            None => ListState::Empty,
        }
    }
}

/// The channels to show first, and the updater that brings the list
/// downloaded from `list` (else from the URL chosen from the phone): the
/// tests' list instead when copied, and the made-up channels for the
/// interface test when there is no other.
pub(super) fn channels(list: Option<Url>, ui_test: bool) -> (ChannelList, Option<Updater>) {
    if let Some(channels) = m3u::load(Path::new(CHANNELS_FILE)) {
        log!("test list {CHANNELS_FILE}: {} channels", channels.len());
        return (ChannelList::new(channels), None);
    }
    let (saved, updater) = Updater::start(Path::new(DATA_DIR), list);
    match saved {
        Some(channels) => (ChannelList::new(channels), Some(updater)),
        None if ui_test => (ChannelList::synthetic(TEST_CHANNELS), None),
        None => (ChannelList::new(Vec::new()), Some(updater)),
    }
}
