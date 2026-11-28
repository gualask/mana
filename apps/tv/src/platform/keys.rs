//! Maps SDL scancodes from a keyboard or the LG remote to UI actions.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
}

const RETURN: i32 = 40;
const ESCAPE: i32 = 41;
const RIGHT: i32 = 79;
const LEFT: i32 = 80;
const DOWN: i32 = 81;
const UP: i32 = 82;
const KP_ENTER: i32 = 88;
const SPACE: i32 = 44;
const PAUSE: i32 = 72;
/// What SDL makes of Linux's KEY_PLAY and KEY_PLAYPAUSE
/// (SDL-webOS's scancodes_linux.h).
const AUDIOPLAY: i32 = 261;
const AC_BACK: i32 = 270;
/// LG remote BACK in webOS SDL builds.
const WEBOS_BACK: i32 = 482;

pub fn action(scancode: i32) -> Option<Action> {
    match scancode {
        UP => Some(Action::Up),
        DOWN => Some(Action::Down),
        LEFT => Some(Action::Left),
        RIGHT => Some(Action::Right),
        RETURN | KP_ENTER => Some(Action::Select),
        ESCAPE | AC_BACK | WEBOS_BACK => Some(Action::Back),
        _ => None,
    }
}

/// Whether the key is the remote's Play or Pause, or the keyboard's space:
/// it pauses and resumes whatever the screen.
pub fn play_pause(scancode: i32) -> bool {
    matches!(scancode, AUDIOPLAY | PAUSE | SPACE)
}
