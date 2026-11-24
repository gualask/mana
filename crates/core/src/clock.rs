//! The local time, for the times of events: what the system's time zone
//! says (`clock.c`, as Rust's standard library gives no local time), and
//! how the screens write an event's time.

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

/// The instant `at`, in seconds since 1970, in local time: `20:30` on the
/// day of `now`, else with the day, `24/11 20:30`.
pub fn time(at: i64, now: i64) -> String {
    let at = local(at);
    let (day, today) = (at.div_euclid(86_400), local(now).div_euclid(86_400));
    let date = crate::source::date::rfc3339(at);
    let time = &date[11..16];
    if day == today { time.to_string() } else { format!("{}/{} {time}", &date[8..10], &date[5..7]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_today_reads_as_its_time_and_another_day_with_the_day() {
        let at = crate::source::date::instant("2026-11-24T12:00:00Z").unwrap();
        let offset = local(at) - at;
        let today = time(at, at + 60);
        assert_eq!(today.len(), 5, "{today}");
        assert_eq!(today, crate::source::date::rfc3339(at + offset)[11..16]);
        let tomorrow = time(at + 2 * 86_400, at);
        assert!(tomorrow.contains('/') && tomorrow.ends_with(&today), "{tomorrow}");
    }
}
