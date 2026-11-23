//! The dates of EXT-X-PROGRAM-DATE-TIME.

/// An RFC 3339 date and time (RFC 8216 §4.3.2.6), `2026-11-21T10:00:00.5Z`
/// or with an offset as `+01:00`, in seconds since the Unix epoch.
pub(super) fn parse_date(text: &str) -> Option<f64> {
    let (day, time) = text.trim().split_once(['T', 't'])?;
    let mut day = day.splitn(3, '-').map(|n| n.parse::<i64>().ok());
    let (year, month, date) = (day.next()??, day.next()??, day.next()??);
    let (clock, offset) = split_offset(time)?;
    let mut clock = clock.splitn(3, ':');
    let (hour, minute) = (clock.next()?.parse::<i64>().ok()?, clock.next()?.parse::<i64>().ok()?);
    let second = clock.next()?.parse::<f64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&date) || hour > 23 || minute > 59 || !(0.0..61.0).contains(&second) {
        return None;
    }
    let days = days_since_epoch(year, month, date);
    Some((days * 86_400 + hour * 3_600 + minute * 60) as f64 + second - offset)
}

/// The clock of an RFC 3339 time and its offset from UTC, in seconds: `Z`,
/// or `+01:00`, `-0030`.
fn split_offset(time: &str) -> Option<(&str, f64)> {
    if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        return Some((clock, 0.0));
    }
    let at = time.rfind(['+', '-'])?;
    let digits = time[at + 1..].replace(':', "");
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let offset = (digits[..2].parse::<f64>().ok()? * 60.0 + digits[2..].parse::<f64>().ok()?) * 60.0;
    Some((&time[..at], if time[at..].starts_with('-') { -offset } else { offset }))
}

/// Days from 1970-01-01 in the proleptic Gregorian calendar (H. Hinnant).
fn days_since_epoch(year: i64, month: i64, date: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year.rem_euclid(400);
    let of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + date - 1;
    era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468
}
