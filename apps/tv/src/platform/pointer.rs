//! The Magic Remote's pointer, as Stremio Origin and the TV's own screens
//! treat it: an arrow key puts it away, and a hand's tremor right after
//! does not bring it back; away, its clicks are OK on the row with the
//! focus. The wheel moves one row per notch, at most every WHEEL_INTERVAL.
//! The TV shows and hides its arrow itself, after 5 s still
//! (SDL_WEBOS_CURSOR_SLEEP_TIME), over the video too: hidden with
//! `SDL_ShowCursor`, it sends no motion at all until shown again, and on
//! the B1 it once did not come back when shown again.

use std::time::{Duration, Instant};

/// After an arrow key, motion this soon is the hand pressing it.
const ARROW_WINDOW: Duration = Duration::from_millis(350);
/// Then the pointer comes back once it moves this far, in drawable pixels,
/// from where it rested...
const TREMOR_DISTANCE: f32 = 32.0;
/// ...a rest being this long without motion.
const TREMOR_PAUSE: Duration = Duration::from_millis(300);
/// A notch of the wheel can send several events.
const WHEEL_INTERVAL: Duration = Duration::from_millis(70);

pub struct Pointer {
    /// Drawable pixels per window unit, the unit SDL gives positions in.
    scale: (f32, f32),
    /// Put away, and when.
    put_away: Option<Instant>,
    /// Where the pointer rested since, and when it last moved.
    rest: Option<((f32, f32), Instant)>,
    wheel_at: Option<Instant>,
}

impl Pointer {
    pub fn new(scale: (f32, f32)) -> Self {
        Self { scale, put_away: None, rest: None, wheel_at: None }
    }

    /// The window was resized, or moved to another display.
    pub fn set_scale(&mut self, scale: (f32, f32)) {
        self.scale = scale;
    }

    /// A position, from window units to the drawable pixels the screens
    /// are drawn in.
    pub fn pixels(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 * self.scale.0, y as f32 * self.scale.1)
    }

    /// An arrow key: the pointer goes away until it really moves.
    pub fn put_away(&mut self, now: Instant) {
        self.put_away = Some(now);
        self.rest = None;
    }

    /// The pointer moved to `at`: whether it counts, rather than being a
    /// tremor after an arrow key. Counting brings it back.
    pub fn moved(&mut self, at: (f32, f32), now: Instant) -> bool {
        let Some(put_away) = self.put_away else { return true };
        if now.duration_since(put_away) < ARROW_WINDOW {
            return false;
        }
        let rest = match self.rest {
            Some((rest, moved)) if now.duration_since(moved) <= TREMOR_PAUSE => rest,
            _ => at,
        };
        self.rest = Some((rest, now));
        if (at.0 - rest.0).hypot(at.1 - rest.1) < TREMOR_DISTANCE {
            return false;
        }
        self.put_away = None;
        self.rest = None;
        true
    }

    /// Whether the pointer is away: its clicks are OK on the focus.
    pub fn is_away(&self) -> bool {
        self.put_away.is_some()
    }

    /// Whether a wheel event at `now` moves a row.
    pub fn wheel(&mut self, now: Instant) -> bool {
        if self.wheel_at.is_some_and(|at| now.duration_since(at) < WHEEL_INTERVAL) {
            return false;
        }
        self.wheel_at = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_an_arrow_key_only_a_real_move_brings_the_pointer_back() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut pointer = Pointer::new((2.0, 2.0));
        assert_eq!(pointer.pixels(250, 250), (500.0, 500.0));
        assert!(pointer.moved((500.0, 500.0), at(0)));
        pointer.put_away(at(100));
        assert!(pointer.is_away());
        // The press itself, then a tremble around where the hand rests.
        assert!(!pointer.moved((560.0, 500.0), at(300)));
        assert!(!pointer.moved((570.0, 500.0), at(500)));
        assert!(!pointer.moved((590.0, 510.0), at(600)));
        // A real move from there brings it back.
        assert!(pointer.moved((640.0, 500.0), at(700)));
        assert!(!pointer.is_away());
        assert!(pointer.moved((641.0, 500.0), at(710)));
    }

    #[test]
    fn a_notch_of_the_wheel_moves_one_row() {
        let start = Instant::now();
        let mut pointer = Pointer::new((1.0, 1.0));
        assert!(pointer.wheel(start));
        assert!(!pointer.wheel(start + Duration::from_millis(30)));
        assert!(pointer.wheel(start + Duration::from_millis(80)));
    }
}
