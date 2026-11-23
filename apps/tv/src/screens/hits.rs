//! The screens' side of the Magic Remote's pointer: where the rows were
//! drawn, recorded as they are drawn so that the pointer finds the row
//! under it (as Stremio Origin's hit areas: the drawing is the only
//! layout), and what pointing at or clicking a row does.

use super::setup::Row;
use super::{App, Outcome};
use crate::platform::keys::Action;

/// What a row on screen stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A row of the home screen: a channel, then the clips.
    Home(usize),
    /// The home screen's «Settings».
    Settings,
    /// The settings screen's «Update list».
    Refresh,
    /// The settings screen's «Language».
    Language,
    Channel(usize),
    Quality(usize),
}

/// The rows of the last frame drawn, in drawable pixels.
#[derive(Default)]
pub struct Hits(Vec<(Area, Target)>);

#[derive(Clone, Copy)]
struct Area {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Hits {
    /// Forgets the rows: the screen changed, or is about to be drawn again.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add(&mut self, x: f32, y: f32, w: f32, h: f32, target: Target) {
        self.0.push((Area { x, y, w, h }, target));
    }

    /// The row at `(x, y)`, the last drawn when rows overlap.
    pub fn at(&self, x: f32, y: f32) -> Option<Target> {
        self.0
            .iter()
            .rev()
            .find(|(a, _)| x >= a.x && x < a.x + a.w && y >= a.y && y < a.y + a.h)
            .map(|&(_, target)| target)
    }
}

impl App {
    /// Whether the pointer has nothing to point at: over the video, with no
    /// list or menu open, where a click is OK.
    fn over_video(&self) -> bool {
        self.session.is_playing() && !self.channels_open && self.quality_menu.is_none()
    }

    /// The pointer at `(x, y)`, in drawable pixels: the row under it takes
    /// the focus, as it would with the arrow keys.
    pub fn point(&mut self, x: f32, y: f32) {
        let Some(target) = self.hits.at(x, y) else { return };
        match target {
            Target::Home(i) => {
                self.focus = i;
                self.settings_focused = false;
            }
            Target::Settings => self.settings_focused = true,
            Target::Refresh => self.point_setup(Row::Refresh),
            Target::Language => self.point_setup(Row::Language),
            Target::Channel(i) => self.channels.point(i),
            Target::Quality(i) => {
                if let Some(menu) = &mut self.quality_menu {
                    menu.set_focus(i);
                }
            }
        }
    }

    /// A click at `(x, y)`: OK on the row under the pointer, or over the
    /// video; nothing elsewhere.
    pub fn click(&mut self, x: f32, y: f32) -> Outcome {
        if self.over_video() {
            return self.handle(Action::Select);
        }
        if self.hits.at(x, y).is_none() {
            return Outcome::Continue;
        }
        self.point(x, y);
        self.handle(Action::Select)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_row_under_a_point_and_nothing_between_rows() {
        let mut hits = Hits::default();
        hits.add(100.0, 200.0, 300.0, 60.0, Target::Channel(4));
        hits.add(100.0, 264.0, 300.0, 60.0, Target::Channel(5));
        assert_eq!(hits.at(150.0, 230.0), Some(Target::Channel(4)));
        assert_eq!(hits.at(399.0, 264.0), Some(Target::Channel(5)));
        assert_eq!(hits.at(150.0, 262.0), None);
        assert_eq!(hits.at(400.0, 230.0), None);
        hits.clear();
        assert_eq!(hits.at(150.0, 230.0), None);
    }
}
