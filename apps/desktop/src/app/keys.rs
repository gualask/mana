//! What the keys do: the arrows move in the list, Enter chooses (a folder
//! opens, a channel or a signal plays, an event with several signals shows
//! them), Esc shows the list over the video where it was left, then goes
//! back a level, then hides it.

use mana_core::catalog::{Channel, Folder, Play};
use mana_core::playback::Settings;
use mana_core::session::{Candidates, Program};
use rune::{Key, KeyEvent};

use super::levels::rows;
use super::{Action, App, Kind, Level, Note};

/// Rows moved by Left and Right.
const PAGE: isize = 10;

impl App {
    pub fn on_key(&mut self, event: &KeyEvent) -> bool {
        match &event.key {
            Key::Enter if !self.list_open => {
                self.list_open = true;
                self.refresh_folder();
            }
            Key::Enter => self.choose(),
            Key::Escape => match escape(self.list_open, self.levels.len(), self.session.is_playing()) {
                Escape::Show => {
                    self.list_open = true;
                    self.refresh_folder();
                }
                Escape::Up => {
                    self.levels.pop();
                    self.version += 1;
                }
                Escape::Hide => self.list_open = false,
                Escape::Nothing => return false,
            },
            Key::Up if self.list_open => self.step(-1),
            Key::Down if self.list_open => self.step(1),
            Key::Left if self.list_open => self.step(-PAGE),
            Key::Right if self.list_open => self.step(PAGE),
            _ => return false,
        }
        true
    }

    fn step(&mut self, by: isize) {
        let level = self.top_mut();
        level.focus = level.focus.saturating_add_signed(by).min(level.rows.len().saturating_sub(1));
    }

    /// OK on the focused row: a folder opens, a channel or a signal plays,
    /// an event with several signals shows them.
    fn choose(&mut self) {
        let level = self.top();
        let Some(row) = level.rows.get(level.focus) else { return };
        let name = row.title.clone();
        match &row.action {
            // A signal plays under the name of its event, as on the TV.
            Action::Signal(selection) => {
                let event = match &level.kind {
                    Kind::Signals(event) => event.clone(),
                    _ => name,
                };
                self.play(event, Program::Source(selection.clone()));
            }
            Action::Channel(channel) => match channel.play.clone() {
                Err(error) => self.note = Note::Unplayable(name, error),
                Ok(Play::Folder(folder)) => self.open_folder(name, folder),
                Ok(Play::Stream(playable)) => self.play(name, Program::Live(playable)),
                Ok(Play::Source(selection)) if selection.source.sources.is_some() => {
                    let asking = Candidates::ask(&selection);
                    self.asking = Some((name, selection, asking));
                    self.note = Note::SignalsAsking;
                }
                Ok(Play::Source(selection)) => self.play(name, Program::Source(selection)),
            },
        }
    }

    fn open_folder(&mut self, title: String, folder: Folder) {
        let copy = self.folders.open(&folder);
        let rows = copy.as_deref().map(|channels| rows(channels, &folder.name, self.language)).unwrap_or_default();
        self.note = if rows.is_empty() { Note::FolderLoading } else { Note::None };
        self.levels.push(Level { kind: Kind::Folder(folder.name), title, rows, focus: 0 });
        self.version += 1;
    }

    /// Asks the open folder's source again, if its events are old: as the
    /// list opens over the video.
    fn refresh_folder(&mut self) {
        let Some(name) = self.levels.iter().rev().find_map(|l| match &l.kind {
            Kind::Folder(name) => Some(name.clone()),
            _ => None,
        }) else {
            return;
        };
        let folder = self.levels[0].rows.iter().find_map(|row| match &row.action {
            Action::Channel(Channel { play: Ok(Play::Folder(folder)), .. }) if folder.name == name => Some(folder.clone()),
            _ => None,
        });
        if let Some(folder) = folder {
            self.folders.refresh(&folder);
        }
    }

    pub(super) fn play(&mut self, name: String, program: Program) {
        match self.session.play(name, program, Settings::default()) {
            Ok(()) => {
                self.list_open = false;
                self.note = Note::None;
            }
            Err(failure) => self.note = Note::NotStarted(failure),
        }
    }
}

/// What Esc does.
#[derive(Debug, PartialEq)]
enum Escape {
    /// Shows the list over the video, at the level it was left.
    Show,
    /// Goes back a level: signals to folder, folder to list.
    Up,
    /// Hides the list over the video.
    Hide,
    Nothing,
}

/// Esc with the list open or not, `depth` levels deep, a video playing or
/// not: the hidden list shows where it was left, the open one goes back a
/// level, then hides over a video.
fn escape(list_open: bool, depth: usize, playing: bool) -> Escape {
    match (list_open, depth, playing) {
        (false, _, true) => Escape::Show,
        (false, _, false) => Escape::Nothing,
        (true, 2.., _) => Escape::Up,
        (true, _, true) => Escape::Hide,
        (true, _, false) => Escape::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_shows_the_hidden_list_where_it_was_then_goes_back_a_level_then_hides_it() {
        // Watching an event chosen among its signals, three levels deep.
        assert_eq!(escape(false, 3, true), Escape::Show);
        assert_eq!(escape(true, 3, true), Escape::Up);
        assert_eq!(escape(true, 2, true), Escape::Up);
        assert_eq!(escape(true, 1, true), Escape::Hide);
        // Nothing plays: the list stays, at its first level.
        assert_eq!(escape(true, 1, false), Escape::Nothing);
        assert_eq!(escape(true, 2, false), Escape::Up);
    }
}
