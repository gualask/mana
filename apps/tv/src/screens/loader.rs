//! The loader over the video while a viewing waits for its first frame:
//! dots on a circle at the centre of the screen, lit one after another.

use std::time::Duration;

use mana_core::session::Stats;

use super::TEXT;
use super::gfx::Renderer;

const DOTS: usize = 12;
/// How long each dot stays the brightest.
const STEP: Duration = Duration::from_millis(80);
/// How often the screen is drawn while the loader turns: twice per step,
/// so that no step is skipped.
pub const REDRAW: Duration = Duration::from_millis(STEP.as_millis() as u64 / 2);
/// The circle's radius and the dots' side, in design pixels.
const RADIUS: f32 = 40.0;
const DOT: f32 = 10.0;
/// The dimmest dot, at the end of the trail.
const DIMMEST: f32 = 0.15;

/// Whether the viewing waits for its first frame: it did not end, and the
/// decoder shows nothing yet.
pub fn waiting(stats: &Stats) -> bool {
    stats.playback.presented_ms.is_none() && !stats.state.ended()
}

/// The loader, `elapsed` after the viewing started, on a surface of
/// `surface` drawable pixels.
pub fn draw(r: &mut Renderer, s: f32, surface: (i32, i32), elapsed: Duration) {
    let (cx, cy) = (surface.0 as f32 / 2.0, surface.1 as f32 / 2.0);
    for (x, y, alpha) in dots(elapsed) {
        let side = DOT * s;
        r.rect(cx + x * s - side / 2.0, cy + y * s - side / 2.0, side, side, TEXT.with_alpha(alpha));
    }
}

/// Each dot's offset from the centre, clockwise from the top, and its
/// opacity: the brightest the one `elapsed` has reached, the others fading
/// behind it.
fn dots(elapsed: Duration) -> impl Iterator<Item = (f32, f32, f32)> {
    let head = (elapsed.as_millis() / STEP.as_millis()) as usize % DOTS;
    (0..DOTS).map(move |i| {
        let angle = i as f32 / DOTS as f32 * std::f32::consts::TAU;
        let behind = (head + DOTS - i) % DOTS;
        let alpha = 1.0 - (1.0 - DIMMEST) * behind as f32 / (DOTS - 1) as f32;
        (RADIUS * angle.sin(), -RADIUS * angle.cos(), alpha)
    })
}

#[cfg(test)]
mod tests {
    use mana_core::session::State;

    use super::*;

    #[test]
    fn the_brightest_dot_goes_round_and_the_others_fade_behind_it() {
        let at_start: Vec<_> = dots(Duration::ZERO).collect();
        assert_eq!(at_start.len(), DOTS);
        let (x, y, alpha) = at_start[0];
        assert!(x.abs() < 1e-3 && (y + RADIUS).abs() < 1e-3 && alpha == 1.0);
        assert!((at_start[1].2 - DIMMEST).abs() < 1e-6);
        let later: Vec<_> = dots(STEP * 3).collect();
        assert_eq!(later[3].2, 1.0);
        assert!(later[2].2 > later[1].2 && later[1].2 > later[0].2);
    }

    #[test]
    fn it_waits_until_a_frame_shows_or_the_viewing_ends() {
        let mut stats = Stats::default();
        assert!(waiting(&stats));
        stats.state = State::Playing;
        assert!(waiting(&stats));
        stats.playback.presented_ms = Some(0);
        assert!(!waiting(&stats));
        stats.playback.presented_ms = None;
        stats.state = State::Failed("rete assente".into());
        assert!(!waiting(&stats));
    }
}
