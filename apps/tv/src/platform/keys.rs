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
