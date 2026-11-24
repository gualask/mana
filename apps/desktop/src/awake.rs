//! Keeps the display awake while a video plays, as mpv and VLC do on macOS
//! with an IOKit assertion against idle display sleep; elsewhere nothing,
//! for now.

/// Holds the display awake while it lives.
pub struct Awake {
    #[cfg(target_os = "macos")]
    id: u32,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};

    pub type CfString = *const c_void;
    /// kCFStringEncodingUTF8.
    pub const UTF8: u32 = 0x0800_0100;
    /// kIOPMAssertionLevelOn.
    pub const LEVEL_ON: u32 = 255;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        pub fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> CfString;
        pub fn CFRelease(object: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        pub fn IOPMAssertionCreateWithName(kind: CfString, level: u32, name: CfString, id: *mut u32) -> i32;
        pub fn IOPMAssertionRelease(id: u32) -> i32;
    }

    pub fn text(text: &std::ffi::CStr) -> CfString {
        // SAFETY: a NUL-terminated UTF-8 string, copied by CoreFoundation.
        unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) }
    }
}

impl Awake {
    /// The display kept awake; None when the system refuses.
    #[cfg(target_os = "macos")]
    pub fn hold() -> Option<Self> {
        let (kind, name) = (macos::text(c"PreventUserIdleDisplaySleep"), macos::text(c"Mana is playing a video"));
        if kind.is_null() || name.is_null() {
            return None;
        }
        let mut id = 0;
        // SAFETY: two valid CFStrings, released once the assertion is made.
        let made = unsafe {
            let made = macos::IOPMAssertionCreateWithName(kind, macos::LEVEL_ON, name, &mut id);
            macos::CFRelease(kind);
            macos::CFRelease(name);
            made
        };
        (made == 0).then_some(Self { id })
    }

    #[cfg(not(target_os = "macos"))]
    pub fn hold() -> Option<Self> {
        None
    }
}

#[cfg(target_os = "macos")]
impl Drop for Awake {
    fn drop(&mut self) {
        // SAFETY: an assertion this value made, released once.
        unsafe { macos::IOPMAssertionRelease(self.id) };
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    fn listed() -> bool {
        let output = std::process::Command::new("pmset").args(["-g", "assertions"]).output().unwrap();
        String::from_utf8_lossy(&output.stdout).contains("Mana is playing a video")
    }

    #[test]
    fn the_display_stays_awake_while_held() {
        let awake = Awake::hold().expect("the assertion made");
        assert!(listed());
        drop(awake);
        assert!(!listed());
    }
}
