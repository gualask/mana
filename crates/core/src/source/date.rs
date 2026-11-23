//! Dates as sources write them, made instants: the `date` operation, and
//! the reading of its RFC 3339 result for whoever shows it.

/// The seconds an offset as `+08:00`, `-05:30` or `Z` adds to UTC.
pub fn offset(zone: &str) -> Option<i64> {
    if zone == "Z" {
        return Some(0);
    }
    let (sign, rest) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => return None,
    };
    let (hours, minutes) = rest.split_once(':')?;
    let (hours, minutes): (i64, i64) = (number(hours, 2)?, number(minutes, 2)?);
    (hours < 24 && minutes < 60).then_some(sign * (hours * 3600 + minutes * 60))
}

/// Whether `format` can be read: `unix`, `unix_ms`, or text with `%Y`,
/// `%m`, `%d` and optionally `%H`, `%M`, `%S`.
pub fn check_format(format: &str) -> Result<(), String> {
    if format == "unix" || format == "unix_ms" {
        return Ok(());
    }
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('Y' | 'm' | 'd' | 'H' | 'M' | 'S' | '%') => {}
                other => return Err(format!("`%{}` is not one of %Y %m %d %H %M %S", other.map_or(String::new(), String::from))),
            }
        }
    }
    if ["%Y", "%m", "%d"].iter().all(|field| format.contains(field)) { Ok(()) } else { Err("`%Y`, `%m` and `%d` are needed".into()) }
}

/// The instant `text` gives in `format` at `offset` seconds from UTC, in
/// seconds since 1970.
pub fn parse(text: &str, format: &str, offset: i64) -> Result<i64, String> {
    let text = text.trim();
    let unreadable = || format!("`{text}` does not read as `{format}`");
    match format {
        "unix" => return text.parse().map_err(|_| unreadable()),
        "unix_ms" => return text.parse::<i64>().map(|ms| ms.div_euclid(1000)).map_err(|_| unreadable()),
        _ => {}
    }
    let (mut fields, mut rest, mut spec) = ([0i64, 1, 1, 0, 0, 0], text, format.chars());
    while let Some(c) = spec.next() {
        let (index, width) = match (c, c == '%') {
            (_, true) => match spec.next() {
                Some('Y') => (0, 4),
                Some('m') => (1, 2),
                Some('d') => (2, 2),
                Some('H') => (3, 2),
                Some('M') => (4, 2),
                Some('S') => (5, 2),
                _ => {
                    rest = rest.strip_prefix('%').ok_or_else(unreadable)?;
                    continue;
                }
            },
            (literal, false) => {
                rest = rest.strip_prefix(literal).ok_or_else(unreadable)?;
                continue;
            }
        };
        let digits = rest.get(..width).ok_or_else(unreadable)?;
        fields[index] = number(digits, width).ok_or_else(unreadable)?;
        rest = &rest[width..];
    }
    let [year, month, day, hour, minute, second] = fields;
    if !rest.is_empty() || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return Err(unreadable());
    }
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// `seconds` since 1970 as RFC 3339 in UTC: `2026-11-23T12:30:00Z`.
pub fn rfc3339(seconds: i64) -> String {
    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", time / 3600, time / 60 % 60, time % 60)
}

/// The instant of an RFC 3339 date in UTC, as `rfc3339` writes it.
pub fn instant(text: &str) -> Option<i64> {
    parse(text, "%Y-%m-%dT%H:%M:%SZ", 0).ok()
}

/// Exactly `width` ASCII digits.
fn number(text: &str, width: usize) -> Option<i64> {
    (text.len() == width && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

/// Days since 1970-01-01 of a date of the proleptic Gregorian calendar
/// (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = if month < 10 { month + 3 } else { month - 9 };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_in_a_zone_of_its_own_is_an_instant_in_utc() {
        let at = parse("2026-11-23 20:30:00", "%Y-%m-%d %H:%M:%S", offset("+08:00").unwrap()).unwrap();
        assert_eq!(rfc3339(at), "2026-11-23T12:30:00Z");
        assert_eq!(instant("2026-11-23T12:30:00Z"), Some(at));
        assert_eq!(rfc3339(parse("01/03/2024", "%d/%m/%Y", 0).unwrap()), "2024-03-01T00:00:00Z");
        assert_eq!(rfc3339(parse("1795449600", "unix", 0).unwrap()), "2026-11-23T16:00:00Z");
        assert_eq!(parse("1795449600123", "unix_ms", 0).unwrap(), 1_795_449_600);
        assert_eq!(rfc3339(parse("2026-01-01 03:00", "%Y-%m-%d %H:%M", offset("+05:30").unwrap()).unwrap()), "2025-12-31T21:30:00Z");
    }

    #[test]
    fn what_does_not_fit_is_refused() {
        assert!(parse("2026-13-01", "%Y-%m-%d", 0).is_err());
        assert!(parse("2026-11-23 extra", "%Y-%m-%d", 0).is_err());
        assert!(parse("26-11-23", "%Y-%m-%d", 0).is_err());
        assert_eq!(offset("08:00"), None);
        assert_eq!(offset("-05:30"), Some(-19_800));
        assert!(check_format("%Y-%m-%d %H:%M").is_ok());
        assert!(check_format("%H:%M").is_err());
        assert!(check_format("%Y-%m-%d %Z").is_err());
    }
}
