//! The TV's local time, for the times of events.

use std::time::{SystemTime, UNIX_EPOCH};

unsafe extern "C" {
    fn mana_utc_offset(seconds: i64) -> std::ffi::c_long;
}

/// `seconds` since 1970 in local time, as seconds since 1970 of the same
/// wall clock in UTC.
pub fn local(seconds: i64) -> i64 {
    // SAFETY: the function only reads its argument and the time zone.
    seconds + i64::from(unsafe { mana_utc_offset(seconds) })
}

/// Now, in seconds since 1970.
pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
