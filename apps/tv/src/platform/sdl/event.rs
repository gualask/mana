//! SDL events, decoded by offset because LG's system SDL (webOS "broken
//! ABI") inserts a `Uint32 inputSource` after `windowID` in keyboard and
//! text events, shifting every later field.

const EVENT_QUIT: u32 = 0x100;
const EVENT_APP_FIRST: u32 = 0x101;
const EVENT_APP_LAST: u32 = 0x106;
const EVENT_WINDOW: u32 = 0x200;
const EVENT_KEY_DOWN: u32 = 0x300;
const EVENT_KEY_UP: u32 = 0x301;
const EVENT_MOUSE_MOTION: u32 = 0x400;
const EVENT_MOUSE_BUTTON_DOWN: u32 = 0x401;
const EVENT_MOUSE_BUTTON_UP: u32 = 0x402;
const EVENT_MOUSE_WHEEL: u32 = 0x403;
/// Offsets of `x` and `y` in `SDL_MouseMotionEvent` and
/// `SDL_MouseButtonEvent`, and of `y` in `SDL_MouseWheelEvent`: LG's SDL
/// has them as upstream SDL 2 (PlxNative's shell-sdl/src/inject.rs, the
/// window units Stremio Origin reads), unlike its keyboard events.
const MOUSE_X_OFFSET: usize = 20;
const MOUSE_Y_OFFSET: usize = 24;
const WHEEL_Y_OFFSET: usize = 20;

/// Offset of `state` in `SDL_KeyboardEvent`: 12 in upstream SDL, 16 in LG's
/// system SDL where `inputSource` is present.
const KEY_STATE_OFFSET: usize = if crate::platform::WEBOS { 16 } else { 12 };

/// Large enough for `SDL_Event` (56 bytes upstream) with room for vendor growth.
#[repr(C, align(8))]
pub(super) struct RawEvent(pub(super) [u8; 128]);

impl RawEvent {
    fn u8_at(&self, offset: usize) -> u8 {
        self.0[offset]
    }

    fn u32_at(&self, offset: usize) -> u32 {
        u32::from_ne_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }

    fn i32_at(&self, offset: usize) -> i32 {
        i32::from_ne_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Quit,
    Key { down: bool, repeat: bool, scancode: i32, keycode: i32 },
    /// `SDL_WindowEventID`, e.g. 3 = exposed, 6 = resized, 12/13 = focus.
    Window(u8),
    /// SDL application lifecycle events (terminating, background, foreground).
    Lifecycle(u32),
    /// The Magic Remote's pointer moving to `(x, y)`, in window units.
    PointerMove { x: i32, y: i32 },
    /// A press of the Magic Remote's OK while its pointer shows: a click
    /// at `(x, y)`, with no key alongside.
    PointerPress { x: i32, y: i32 },
    /// The click released.
    PointerRelease,
    /// The Magic Remote's wheel, in notches: positive away from the viewer.
    Wheel(i32),
    Other,
}

impl Event {
    pub(super) fn decode(raw: &RawEvent) -> Self {
        let kind = raw.u32_at(0);
        match kind {
            EVENT_QUIT => Event::Quit,
            EVENT_APP_FIRST..=EVENT_APP_LAST => Event::Lifecycle(kind),
            EVENT_WINDOW => Event::Window(raw.u8_at(12)),
            EVENT_KEY_DOWN | EVENT_KEY_UP => Event::Key {
                down: kind == EVENT_KEY_DOWN,
                repeat: raw.u8_at(KEY_STATE_OFFSET + 1) != 0,
                scancode: raw.i32_at(KEY_STATE_OFFSET + 4),
                keycode: raw.i32_at(KEY_STATE_OFFSET + 8),
            },
            EVENT_MOUSE_MOTION => {
                Event::PointerMove { x: raw.i32_at(MOUSE_X_OFFSET), y: raw.i32_at(MOUSE_Y_OFFSET) }
            }
            EVENT_MOUSE_BUTTON_DOWN => {
                Event::PointerPress { x: raw.i32_at(MOUSE_X_OFFSET), y: raw.i32_at(MOUSE_Y_OFFSET) }
            }
            EVENT_MOUSE_BUTTON_UP => Event::PointerRelease,
            EVENT_MOUSE_WHEEL => Event::Wheel(raw.i32_at(WHEEL_Y_OFFSET)),
            _ => Event::Other,
        }
    }
}
