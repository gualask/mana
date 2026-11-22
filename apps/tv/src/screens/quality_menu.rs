//! The quality menu over the playing video: «Auto» and the heights the
//! live stream offers, highest first, chosen with the remote.

use mana_core::session::Quality;

use super::gfx::Renderer;
use super::hits::{Hits, Target};
use super::text::Text;
use super::{BACKGROUND as PANEL, FOCUS, MUTED, TEXT};

/// The menu's position among the options; they come from the viewing's
/// stats at each step, since a channel change can change them.
pub struct QualityMenu {
    focus: usize,
}

impl QualityMenu {
    /// A menu on the option `current`, among those for `heights`.
    pub fn new(heights: &[u32], current: Quality) -> Self {
        let focus = options(heights).iter().position(|&q| q == current).unwrap_or(0);
        Self { focus }
    }

    /// Moves the focus by `delta` options, stopping at either end.
    pub fn step(&mut self, heights: &[u32], delta: isize) {
        let last = options(heights).len() as isize - 1;
        self.focus = (self.focus.min(last as usize) as isize + delta).clamp(0, last) as usize;
    }

    /// Moves the focus to the option `index`, under the pointer.
    pub fn set_focus(&mut self, index: usize) {
        self.focus = index;
    }

    /// The option under the focus.
    pub fn focused(&self, heights: &[u32]) -> Quality {
        let options = options(heights);
        options[self.focus.min(options.len() - 1)]
    }

    /// Draws the menu as a translucent panel on the right of the screen,
    /// marking the option in use; `s` scales the 1080p layout to the surface.
    /// Records the options in `hits`.
    pub fn draw(&self, r: &mut Renderer, t: &mut Text, s: f32, heights: &[u32], current: Quality, hits: &mut Hits) {
        let options = options(heights);
        let focus = self.focus.min(options.len() - 1);
        let (x, y, w, row) = (1440.0 * s, 120.0 * s, 420.0 * s, 64.0 * s);
        r.rect(x, y, w, (options.len() as f32 * 64.0 + 120.0) * s, PANEL.with_alpha(0.85));
        t.draw(r, x + 30.0 * s, y + 24.0 * s, 32.0 * s, MUTED, "Qualità");
        for (i, &option) in options.iter().enumerate() {
            let top = y + 90.0 * s + i as f32 * row;
            // The row's box: the focus colours it, the pointer finds it.
            let (bx, bw, bh) = (x + 16.0 * s, w - 32.0 * s, row - 6.0 * s);
            hits.add(bx, top, bw, bh, Target::Quality(i));
            if i == focus {
                r.rect(bx, top, bw, bh, FOCUS);
            }
            let width = t.draw(r, x + 36.0 * s, top + 12.0 * s, 32.0 * s, TEXT, &label(option));
            let in_use_color = if i == focus { TEXT } else { MUTED };
            if option == current {
                t.draw(r, x + 56.0 * s + width, top + 16.0 * s, 26.0 * s, in_use_color, "in uso");
            }
        }
    }
}

/// How `quality` reads on the screens: «Auto», or the lines as «720p».
pub(super) fn label(quality: Quality) -> String {
    match quality {
        Quality::Auto => "Auto".into(),
        Quality::Lines(_) => quality.to_string(),
    }
}

/// «Auto», then `heights` from the highest.
fn options(heights: &[u32]) -> Vec<Quality> {
    std::iter::once(Quality::Auto).chain(heights.iter().rev().map(|&h| Quality::Lines(h))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_auto_then_the_heights_from_the_highest_and_opens_on_the_one_in_use() {
        let heights = [360, 720, 1080];
        assert_eq!(options(&heights), [Quality::Auto, Quality::Lines(1080), Quality::Lines(720), Quality::Lines(360)]);
        let mut menu = QualityMenu::new(&heights, Quality::Lines(720));
        assert_eq!(menu.focused(&heights), Quality::Lines(720));
        menu.step(&heights, 5);
        assert_eq!(menu.focused(&heights), Quality::Lines(360));
        menu.step(&heights, -9);
        assert_eq!(menu.focused(&heights), Quality::Auto);
        // A choice no longer offered, as after a channel change, opens on Auto.
        assert_eq!(QualityMenu::new(&[576], Quality::Lines(720)).focused(&[576]), Quality::Auto);
    }

    #[test]
    fn keeps_the_focus_within_fewer_heights() {
        let mut menu = QualityMenu::new(&[360, 720, 1080], Quality::Lines(360));
        assert_eq!(menu.focused(&[720]), Quality::Lines(720));
        menu.step(&[720], -1);
        assert_eq!(menu.focused(&[720]), Quality::Auto);
    }
}
