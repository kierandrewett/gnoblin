//! The `--since` and `--until` times: an RFC 3339 instant or a duration back from now.
//!
//! The grammar, so a script can rely on it:
//!
//! - an RFC 3339 timestamp with a time and an offset, such as `2026-11-06T10:00:00Z` or
//!   `2026-11-06T10:00:00.250+01:00`. A space for the `T`, or no offset, is refused;
//! - a duration: one or more digits and one unit, `s` (seconds), `m` (minutes), `h`
//!   (hours) or `d` (days), counted back from the time the command runs. `10m` is ten
//!   minutes ago. Compound forms such as `1h30m` are refused.
//!
//! Nothing here reads the clock: the caller passes `now`, so the tests fix it.

use super::iso8601_utc;

/// One `--since` or `--until` value, before it is resolved against the current time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimeBound {
    /// A fixed instant, in milliseconds since the epoch.
    At(i64),
    /// A span back from now, in milliseconds.
    Ago(i64),
}

impl TimeBound {
    /// The instant this bound names, given the current time in milliseconds since the epoch.
    pub(crate) fn resolve(self, now_ms: i64) -> Result<i64, String> {
        match self {
            Self::At(instant) => Ok(instant),
            Self::Ago(span) => now_ms
                .checked_sub(span)
                // A result below zero is before 1970; `checked_sub` only catches overflow.
                .filter(|instant| *instant >= 0)
                .ok_or_else(|| "the duration reaches before the epoch".to_owned()),
        }
    }
}

/// Parses one `--since` or `--until` value. The error names the value and the two forms.
pub(crate) fn parse_bound(text: &str) -> Result<TimeBound, String> {
    if let Some(span) = parse_duration_ms(text)? {
        return Ok(TimeBound::Ago(span));
    }
    parse_rfc3339_ms(text).map(TimeBound::At).ok_or_else(|| {
        format!(
            "\"{text}\" is neither an RFC 3339 timestamp such as 2026-11-06T10:00:00Z nor a duration such as 10m, 2h or 1d"
        )
    })
}

/// Resolves `--since` and `--until` against `now_ms`. Returns the two bounds in milliseconds,
/// and refuses a `--since` later than `--until`, since no record can then match.
pub(crate) fn resolve(
    since: Option<TimeBound>,
    until: Option<TimeBound>,
    now_ms: i64,
) -> Result<(Option<i64>, Option<i64>), String> {
    let since_ms = since
        .map(|bound| bound.resolve(now_ms).map_err(|error| format!("--since: {error}")))
        .transpose()?;
    let until_ms = until
        .map(|bound| bound.resolve(now_ms).map_err(|error| format!("--until: {error}")))
        .transpose()?;
    if let (Some(start), Some(end)) = (since_ms, until_ms)
        && start > end
    {
        return Err(format!(
            "--since {} is after --until {}, so no record can match",
            iso8601_utc(start),
            iso8601_utc(end),
        ));
    }
    Ok((since_ms, until_ms))
}

/// The milliseconds in a duration such as `10m`. `Ok(None)` when the text is not a duration at
/// all, so the caller tries a timestamp next; an error when it is a duration that cannot be
/// counted, or a bare number with no unit.
fn parse_duration_ms(text: &str) -> Result<Option<i64>, String> {
    if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("\"{text}\" needs a unit: s, m, h or d, as in {text}m"));
    }
    let Some(unit) = text.chars().last() else {
        return Ok(None);
    };
    let digits = &text[..text.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(None);
    }
    let unit_ms: i64 = match unit {
        's' => 1_000,
        'm' => 60_000,
        'h' => 3_600_000,
        'd' => 86_400_000,
        _ => return Ok(None),
    };
    let count: i64 = digits
        .parse()
        .map_err(|_| format!("\"{text}\" is too large to count back from now"))?;
    count
        .checked_mul(unit_ms)
        .map(Some)
        .ok_or_else(|| format!("\"{text}\" is too large to count back from now"))
}

/// Milliseconds since the epoch of an RFC 3339 timestamp, or `None` when it is not one.
/// Fractional seconds are kept to the millisecond; digits past the third are dropped.
fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = digits(&bytes[0..4])?;
    let month = digits(&bytes[5..7])?;
    let day = digits(&bytes[8..10])?;
    let hour = digits(&bytes[11..13])?;
    let minute = digits(&bytes[14..16])?;
    let second = digits(&bytes[17..19])?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut rest = &bytes[19..];
    let mut millis = 0;
    if let Some((b'.', after_dot)) = rest.split_first() {
        let fraction = after_dot.iter().take_while(|byte| byte.is_ascii_digit()).count();
        if fraction == 0 {
            return None;
        }
        for position in 0..3 {
            let digit = after_dot.get(position).map_or(0, |byte| i64::from(byte - b'0'));
            millis = millis * 10 + digit;
        }
        rest = &after_dot[fraction..];
    }
    let offset_minutes = match rest {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let hours = digits(&[*h1, *h2])?;
            let minutes = digits(&[*m1, *m2])?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let total = hours * 60 + minutes;
            if *sign == b'+' { total } else { -total }
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some((seconds - offset_minutes * 60) * 1_000 + millis)
}

/// The value of a run of ASCII digits, or `None` when any byte is not a digit.
fn digits(bytes: &[u8]) -> Option<i64> {
    bytes.iter().try_fold(0_i64, |value, byte| {
        byte.is_ascii_digit().then(|| value * 10 + i64::from(byte - b'0'))
    })
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// The number of days in `month` (1 to 12) of `year`.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to the given date. Howard Hinnant's `days_from_civil`, the inverse of
/// the one that formats timestamps in `logs.rs`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    #[test]
    fn an_rfc3339_instant_is_milliseconds_since_the_epoch() {
        assert_eq!(parse_bound("1970-01-01T00:00:00Z"), Ok(TimeBound::At(0)));
        assert_eq!(parse_bound("2023-11-14T22:13:20Z"), Ok(TimeBound::At(NOW)));
        assert_eq!(parse_bound("2023-11-14T22:13:20.123Z"), Ok(TimeBound::At(NOW + 123)));
        assert_eq!(parse_bound("2023-11-14t22:13:20z"), Ok(TimeBound::At(NOW)));
    }

    #[test]
    fn an_offset_moves_the_instant_to_utc() {
        assert_eq!(parse_bound("2023-11-15T00:13:20+02:00"), Ok(TimeBound::At(NOW)));
        assert_eq!(parse_bound("2023-11-14T20:13:20-02:00"), Ok(TimeBound::At(NOW)));
        assert_eq!(parse_bound("2023-11-14T22:13:20.9999Z"), Ok(TimeBound::At(NOW + 999)));
    }

    #[test]
    fn leap_days_are_checked_by_the_calendar() {
        assert_eq!(
            parse_bound("2024-02-29T00:00:00Z"),
            Ok(TimeBound::At(1_709_164_800_000))
        );
        assert!(parse_bound("2023-02-29T00:00:00Z").is_err());
        assert!(parse_bound("2000-02-29T00:00:00Z").is_ok());
        assert!(parse_bound("1900-02-29T00:00:00Z").is_err());
    }

    #[test]
    fn out_of_range_timestamp_fields_are_refused() {
        for text in [
            "2023-13-01T00:00:00Z",
            "2023-00-01T00:00:00Z",
            "2023-04-31T00:00:00Z",
            "2023-11-14T24:00:00Z",
            "2023-11-14T23:60:00Z",
            "2023-11-14T23:59:60Z",
            "2023-11-14T22:13:20+24:00",
            "2023-11-14T22:13:20+02:60",
        ] {
            assert!(parse_bound(text).is_err(), "{text} should be refused");
        }
    }

    #[test]
    fn malformed_timestamps_are_refused_and_named() {
        for text in [
            "2023-11-14",
            "2023-11-14 22:13:20Z",
            "2023-11-14T22:13:20",
            "2023-11-14T22:13:20.Z",
            "2023-11-14T22:13:20+0200",
            "2023-11-14T22:13:20 Z",
            "yesterday",
            "",
        ] {
            let error = parse_bound(text).expect_err(text);
            assert!(error.contains(&format!("\"{text}\"")), "{error}");
        }
    }

    #[test]
    fn durations_count_back_from_now_in_each_unit() {
        assert_eq!(parse_bound("0s"), Ok(TimeBound::Ago(0)));
        assert_eq!(parse_bound("30s"), Ok(TimeBound::Ago(30_000)));
        assert_eq!(parse_bound("10m"), Ok(TimeBound::Ago(600_000)));
        assert_eq!(parse_bound("2h"), Ok(TimeBound::Ago(7_200_000)));
        assert_eq!(parse_bound("1d"), Ok(TimeBound::Ago(86_400_000)));
        assert_eq!(TimeBound::Ago(600_000).resolve(NOW), Ok(NOW - 600_000));
    }

    #[test]
    fn a_bare_number_needs_a_unit() {
        let error = parse_bound("10").expect_err("no unit");
        assert!(error.contains("needs a unit"), "{error}");
        assert!(error.contains("10m"), "{error}");
    }

    #[test]
    fn compound_signed_and_unknown_units_are_refused() {
        for text in ["1h30m", "-10m", "+5m", "10 m", "10w", "10M", "10ms", "m", "d"] {
            assert!(parse_bound(text).is_err(), "{text} should be refused");
        }
    }

    #[test]
    fn a_duration_that_overflows_is_refused_not_wrapped() {
        let error = parse_bound("99999999999999999999d").expect_err("overflow");
        assert!(error.contains("too large"), "{error}");
        assert!(parse_bound("9999999999999999d").is_err());
    }

    #[test]
    fn a_duration_before_the_epoch_is_refused_when_resolved() {
        let error = resolve(Some(TimeBound::Ago(NOW + 1)), None, NOW).expect_err("before epoch");
        assert!(error.starts_with("--since: "), "{error}");
        assert!(error.contains("epoch"), "{error}");
    }

    #[test]
    fn resolve_mixes_absolute_and_relative_bounds() {
        assert_eq!(
            resolve(Some(TimeBound::Ago(600_000)), Some(TimeBound::At(NOW)), NOW),
            Ok((Some(NOW - 600_000), Some(NOW)))
        );
        assert_eq!(resolve(None, None, NOW), Ok((None, None)));
        assert_eq!(resolve(None, Some(TimeBound::Ago(0)), NOW), Ok((None, Some(NOW))));
    }

    #[test]
    fn since_after_until_is_refused_and_names_both_flags() {
        let error = resolve(Some(TimeBound::At(NOW)), Some(TimeBound::At(NOW - 1)), NOW).expect_err("inverted");
        assert!(error.contains("--since"), "{error}");
        assert!(error.contains("--until"), "{error}");
        assert!(error.contains("2023-11-14T22:13:20.000Z"), "{error}");
    }

    #[test]
    fn since_equal_to_until_is_allowed_and_matches_nothing_by_construction() {
        assert_eq!(
            resolve(Some(TimeBound::At(NOW)), Some(TimeBound::At(NOW)), NOW),
            Ok((Some(NOW), Some(NOW)))
        );
    }

    #[test]
    fn a_relative_since_after_an_absolute_until_is_compared_after_resolving() {
        let error = resolve(Some(TimeBound::Ago(0)), Some(TimeBound::At(NOW - 1)), NOW).expect_err("now > until");
        assert!(error.contains("--since"), "{error}");
    }

    #[test]
    fn days_from_civil_inverts_across_the_century_rules() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
    }
}
