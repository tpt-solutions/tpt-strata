//! Calendar dates and timestamps, with no dependencies.
//!
//! A `DATE` is a count of days since 1970-01-01 (an `i32`). A `TIMESTAMP` is
//! a count of microseconds since 1970-01-01 00:00:00 with no time zone (an
//! `i64`). Both use the proleptic Gregorian calendar.

/// Days from 1970-01-01 to the given civil date. Howard Hinnant's algorithm.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date for a day count from 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Parse `YYYY-MM-DD`. Returns `None` for anything else, including an
/// impossible day such as `2026-02-30`.
pub fn parse_date(s: &str) -> Option<i32> {
    let (y, m, d) = parse_ymd(s)?;
    i32::try_from(days_from_civil(y, m, d)).ok()
}

/// Parse `YYYY-MM-DD[ HH:MM[:SS[.f{1,6}]]]`, with `T` accepted in place of the
/// space. Returns microseconds since the epoch.
pub fn parse_timestamp(s: &str) -> Option<i64> {
    let (date, time) = match s.find([' ', 'T']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (y, m, d) = parse_ymd(date)?;
    let days = days_from_civil(y, m, d);
    let micros_in_day = match time {
        None => 0,
        Some(t) => parse_hms_micros(t)?,
    };
    days.checked_mul(86_400_000_000)?.checked_add(micros_in_day)
}

fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let mut parts = s.split('-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    if !(y.bytes().chain(m.bytes()).chain(d.bytes())).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (y, m, d): (i64, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    Some((y, m, d))
}

fn parse_hms_micros(t: &str) -> Option<i64> {
    let (hms, frac) = match t.split_once('.') {
        Some((a, f)) => (a, Some(f)),
        None => (t, None),
    };
    let mut parts = hms.split(':');
    let h: i64 = two_digits(parts.next()?)?;
    let mi: i64 = two_digits(parts.next()?)?;
    let sec: i64 = match parts.next() {
        Some(s) => two_digits(s)?,
        None => 0,
    };
    if parts.next().is_some() || h > 23 || mi > 59 || sec > 59 {
        return None;
    }
    let micros = match frac {
        None => 0,
        Some(f) => {
            if f.is_empty() || f.len() > 6 || !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            // Pad to six digits: ".5" is 500 000 microseconds.
            let padded = format!("{f:0<6}");
            padded.parse::<i64>().ok()?
        }
    };
    Some(((h * 60 + mi) * 60 + sec) * 1_000_000 + micros)
}

fn two_digits(s: &str) -> Option<i64> {
    if s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// Format a date as `YYYY-MM-DD`.
pub fn format_date(days: i32) -> String {
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Format a timestamp as `YYYY-MM-DD HH:MM:SS`, with up to six fractional
/// digits when the value has a sub-second part.
pub fn format_timestamp(micros: i64) -> String {
    let days = micros.div_euclid(86_400_000_000);
    let rem = micros.rem_euclid(86_400_000_000);
    let (y, m, d) = civil_from_days(days);
    let secs = rem / 1_000_000;
    let frac = rem % 1_000_000;
    let (h, mi, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let mut out = format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}");
    if frac != 0 {
        let f = format!("{frac:06}");
        out.push('.');
        out.push_str(f.trim_end_matches('0'));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
    }

    #[test]
    fn round_trips_across_centuries() {
        for days in (-800_000..800_000).step_by(397) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
    }

    #[test]
    fn parses_strict_dates_only() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(
            parse_date("2024-02-29").map(format_date).as_deref(),
            Some("2024-02-29")
        );
        assert_eq!(parse_date("2026-02-29"), None, "2026 is not a leap year");
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-1-01"), None);
        assert_eq!(parse_date("2026/01/01"), None);
        assert_eq!(parse_date("2026-01-01 "), None);
    }

    #[test]
    fn parses_timestamps_with_optional_parts() {
        assert_eq!(parse_timestamp("1970-01-01"), Some(0));
        assert_eq!(parse_timestamp("1970-01-01 00:00:01"), Some(1_000_000));
        assert_eq!(parse_timestamp("1970-01-01T00:00:00.5"), Some(500_000));
        assert_eq!(parse_timestamp("1970-01-01 12:30"), Some(45_000_000_000));
        assert_eq!(parse_timestamp("1970-01-01 24:00:00"), None);
        assert_eq!(parse_timestamp("1970-01-01 00:00:00.1234567"), None);
    }

    #[test]
    fn formats_round_trip() {
        assert_eq!(format_date(-1), "1969-12-31");
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00");
        let t = parse_timestamp("2026-10-13 08:05:09.25").unwrap();
        assert_eq!(format_timestamp(t), "2026-10-13 08:05:09.25");
        assert_eq!(format_timestamp(-1), "1969-12-31 23:59:59.999999");
    }
}
