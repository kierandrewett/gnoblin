//! `clock`: a text that shows the time, and `format_time`, which writes one.
//!
//! The format is strftime's, as far as the toolkit ever took it: `%H` `%M` `%S` (24-hour time), `%I` and `%p` (12-hour), `%d` `%e` `%m`
//! `%y` `%Y` `%j` (date), `%a` `%A` `%b` `%B` (names), `%T` (`%H:%M:%S`), `%R` (`%H:%M`), `%F` (`%Y-%m-%d`) and `%%`. Any other `%`
//! sequence stays as written. The time is local time: the zone is libc's (`$TZ`, else `/etc/localtime`).
#![allow(unsafe_code)]

use std::rc::Rc;

use gnoblin_object::Val;

use super::update::epoch_ms;
use super::widget::{Cid, Special, Ui};

unsafe extern "C" {
    /// Reads `$TZ` again.
    fn tzset();
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A time broken into local fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    pub(crate) year: i64,
    /// 0 to 11.
    pub(crate) month: usize,
    pub(crate) day: i64,
    pub(crate) hour: i64,
    pub(crate) minute: i64,
    pub(crate) second: i64,
    /// 0 is Sunday.
    pub(crate) weekday: usize,
    /// 1 is the first of January.
    pub(crate) day_of_year: i64,
}

/// The local time of `milliseconds` since the Unix epoch.
pub(crate) fn local_fields(milliseconds: f64) -> Fields {
    let seconds = (milliseconds / 1000.0).floor() as libc::time_t;
    // SAFETY: `tm` is a live, zeroed local that `localtime_r` fills; `seconds` is a plain value. `tzset` takes nothing.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tzset();
        if libc::localtime_r(&raw const seconds, &raw mut tm).is_null() {
            return Fields {
                year: 1970,
                month: 0,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                weekday: 4,
                day_of_year: 1,
            };
        }
        tm
    };
    Fields {
        year: i64::from(tm.tm_year) + 1900,
        month: usize::try_from(tm.tm_mon).unwrap_or(0).min(11),
        day: i64::from(tm.tm_mday),
        hour: i64::from(tm.tm_hour),
        minute: i64::from(tm.tm_min),
        second: i64::from(tm.tm_sec),
        weekday: usize::try_from(tm.tm_wday).unwrap_or(0).min(6),
        day_of_year: i64::from(tm.tm_yday) + 1,
    }
}

/// Formats a time as `format` says, in local time.
pub(crate) fn format_time(format: &str, milliseconds: f64) -> String {
    let t = local_fields(milliseconds);
    let two = |value: i64| format!("{value:02}");
    let hours12 = if t.hour % 12 == 0 { 12 } else { t.hour % 12 };
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(code) = chars.next() else {
            out.push('%');
            break;
        };
        match code {
            'H' => out.push_str(&two(t.hour)),
            'I' => out.push_str(&two(hours12)),
            'M' => out.push_str(&two(t.minute)),
            'S' => out.push_str(&two(t.second)),
            'p' => out.push_str(if t.hour < 12 { "AM" } else { "PM" }),
            'd' => out.push_str(&two(t.day)),
            'e' => out.push_str(&format!("{:>2}", t.day)),
            'm' => out.push_str(&two(i64::try_from(t.month).unwrap_or(0) + 1)),
            'y' => out.push_str(&two(t.year.rem_euclid(100))),
            'Y' => out.push_str(&t.year.to_string()),
            'j' => out.push_str(&format!("{:03}", t.day_of_year)),
            'a' => out.push_str(&DAYS[t.weekday][..3]),
            'A' => out.push_str(DAYS[t.weekday]),
            'b' => out.push_str(&MONTHS[t.month][..3]),
            'B' => out.push_str(MONTHS[t.month]),
            'T' => out.push_str(&format!("{}:{}:{}", two(t.hour), two(t.minute), two(t.second))),
            'R' => out.push_str(&format!("{}:{}", two(t.hour), two(t.minute))),
            'F' => out.push_str(&format!(
                "{}-{}-{}",
                t.year,
                two(i64::try_from(t.month).unwrap_or(0) + 1),
                two(t.day)
            )),
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

/// Milliseconds until a clock with this format shows another text: the next second for a format with the `%S` or `%T` field, the next
/// minute otherwise, on the system clock's boundaries.
pub(crate) fn changes_in(format: &str) -> u64 {
    let period: i64 = if format.contains("%S") || format.contains("%T") {
        1000
    } else {
        60_000
    };
    let now = epoch_ms().floor() as i64;
    u64::try_from(period - now.rem_euclid(period)).unwrap_or(1)
}

/// The time a clock shows: its `now`, or the system clock.
fn reading(clock: Option<&gnoblin_object::Closure>) -> Result<f64, String> {
    match clock {
        Some(source) => source
            .call(0, &[])
            .and_then(|value| value.double())
            .map_err(|fail| fail.message),
        None => Ok(epoch_ms()),
    }
}

/// Sets the text from the format and the time. A clock that cannot be read keeps its text.
pub(crate) fn set_text(ui: &Ui, cid: Cid) -> Result<(), String> {
    let Some((format, source)) = ui
        .read(cid, |widget| match &widget.sp {
            Special::Clock(part) => Some((part.format.clone(), part.now.clone())),
            _ => None,
        })
        .flatten()
    else {
        return Ok(());
    };
    let text = format_time(&format, reading(source.as_ref())?);
    ui.tweak(cid, |widget| widget.t.text = text);
    Ok(())
}

/// Refreshes the displayed time using the format and the clock.
pub(crate) fn refresh(ui: &Rc<Ui>, cid: Cid) -> Result<(), String> {
    set_text(ui, cid)
}

/// `format_time` as the object protocol's function: the arguments are a format and milliseconds.
pub(crate) fn call(args: &[Val]) -> gnoblin_object::Res<Val> {
    Ok(Val::Text(format_time(args[0].text()?, args[1].double()?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2024-01-05 14:07:09 UTC, a Friday.
    const FRIDAY: f64 = 1_704_463_629_000.0;

    fn utc() {
        // SAFETY: the tests of this module run in one process and none reads another variable of the environment.
        unsafe { std::env::set_var("TZ", "UTC") };
    }

    #[test]
    fn the_fields_a_format_names_are_written() {
        utc();
        for (format, want) in [
            ("%H:%M", "14:07"),
            ("%I:%M %p", "02:07 PM"),
            ("%a %d %b %Y", "Fri 05 Jan 2024"),
            ("%T %%", "14:07:09 %"),
            ("%Q", "%Q"),
            ("%F %j", "2024-01-05 005"),
            ("%A %B %e %y %R", "Friday January  5 24 14:07"),
            ("%S %m", "09 01"),
            ("100%", "100%"),
        ] {
            assert_eq!(format_time(format, FRIDAY), want, "{format}");
        }
    }

    #[test]
    fn noon_and_midnight_are_twelve_on_the_twelve_hour_clock() {
        utc();
        assert_eq!(format_time("%I %p", 1_704_412_800_000.0), "12 AM");
        assert_eq!(format_time("%I %p", 1_704_456_000_000.0), "12 PM");
    }

    #[test]
    fn a_clock_changes_on_the_second_or_the_minute() {
        assert!(changes_in("%H:%M:%S") <= 1000);
        assert!(changes_in("%T") <= 1000);
        assert!(changes_in("%H:%M") <= 60_000);
        assert!(changes_in("%H:%M") >= 1);
    }
}
