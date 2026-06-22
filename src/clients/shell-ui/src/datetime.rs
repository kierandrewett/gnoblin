//! Local date/time for the datetime popout. Current local time/strings come
//! from libc's timezone + locale aware `localtime_r`/`strftime`; the calendar
//! grid for any month is pure arithmetic.

use std::ffi::{CStr, CString};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Now {
    pub year: i32,
    pub month: u32, // 1..=12
    pub day: u32,   // 1..=31
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    pub day_name: String,   // Locale day name.
    pub month_name: String, // Locale month name.
}

fn init_locale() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        libc::setlocale(libc::LC_ALL, c"".as_ptr());
    });
}

fn local_tm() -> Option<libc::tm> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    local_tm_from_secs(secs)
}

fn local_tm_from_secs(secs: u64) -> Option<libc::tm> {
    let ts = secs as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        if libc::localtime_r(&ts, tm.as_mut_ptr()).is_null() {
            return None;
        }
        Some(tm.assume_init())
    }
}

fn format_tm(tm: &libc::tm, fmt: &str) -> Option<String> {
    init_locale();
    if fmt.is_empty() {
        return Some(String::new());
    }
    let fmt = CString::new(fmt).ok()?;
    let mut bytes = 128usize;
    while bytes <= 4096 {
        let mut buf = vec![0u8; bytes];
        let written = unsafe {
            libc::strftime(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                fmt.as_ptr(),
                tm,
            )
        };
        if written > 0 {
            return unsafe {
                Some(
                    CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
        }
        bytes *= 2;
    }
    None
}

/// Format the current local time using a locale-aware `strftime` pattern.
pub fn format_local(fmt: &str) -> Option<String> {
    let tm = local_tm()?;
    format_tm(&tm, fmt)
}

/// Format a UNIX timestamp using a locale-aware `strftime` pattern.
pub fn format_unix(timestamp_secs: u64, fmt: &str) -> Option<String> {
    let tm = local_tm_from_secs(timestamp_secs)?;
    format_tm(&tm, fmt)
}

/// Format a year/month/day using a locale-aware `strftime` pattern.
pub fn format_date(year: i32, month: u32, day: u32, fmt: &str) -> Option<String> {
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month.saturating_sub(1).min(11) as i32;
    tm.tm_mday = day.clamp(1, 31) as i32;
    tm.tm_hour = 12;
    unsafe {
        libc::mktime(&mut tm);
    }
    format_tm(&tm, fmt)
}

/// Current local time (falls back to a fixed value if libc time fails).
pub fn now() -> Now {
    let parse = || -> Option<Now> {
        let tm = local_tm()?;
        let month = (tm.tm_mon + 1).clamp(1, 12) as u32;
        Some(Now {
            year: tm.tm_year + 1900,
            month,
            day: tm.tm_mday.max(1) as u32,
            hour: tm.tm_hour.clamp(0, 23) as u32,
            min: tm.tm_min.clamp(0, 59) as u32,
            sec: tm.tm_sec.clamp(0, 60) as u32,
            day_name: format_tm(&tm, "%A").unwrap_or_default(),
            month_name: format_tm(&tm, "%B").unwrap_or_default(),
        })
    };
    parse().unwrap_or(Now {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        min: 0,
        sec: 0,
        day_name: String::new(),
        month_name: String::new(),
    })
}

/// Localized abbreviated weekday labels, Monday first, for the calendar header.
pub fn weekday_labels_monday_first() -> Vec<String> {
    (1..=7)
        .map(|day| {
            format_date(2024, 1, day, "%a")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| day.to_string())
        })
        .collect()
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn ordinal_day(year: i32, month: u32, day: u32) -> u32 {
    let mut total = day;
    for m in 1..month {
        total += days_in_month(year, m);
    }
    total
}

/// Day of week (0 = Sunday) for a date, via Zeller's congruence.
pub fn weekday(year: i32, month: u32, day: u32) -> u32 {
    let (y, m) = if month < 3 {
        (year - 1, month + 12)
    } else {
        (year, month)
    };
    let k = y % 100;
    let j = y / 100;
    let h = (day as i32 + 13 * (m as i32 + 1) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    ((h + 6) % 7) as u32
}

fn iso_weekday(year: i32, month: u32, day: u32) -> u32 {
    ((weekday(year, month, day) + 6) % 7) + 1
}

fn iso_weeks_in_year(year: i32) -> u32 {
    let jan_1 = iso_weekday(year, 1, 1);
    if jan_1 == 4 || (jan_1 == 3 && is_leap(year)) {
        53
    } else {
        52
    }
}

pub fn iso_week_number(year: i32, month: u32, day: u32) -> u32 {
    let doy = ordinal_day(year, month, day) as i32;
    let dow = iso_weekday(year, month, day) as i32;
    let week = (doy - dow + 10) / 7;

    if week < 1 {
        iso_weeks_in_year(year - 1)
    } else if week as u32 > iso_weeks_in_year(year) {
        1
    } else {
        week as u32
    }
}

/// One calendar cell.
#[derive(Clone)]
pub struct Cell {
    pub day_num: i32,
    pub is_today: bool,
    pub is_other_month: bool,
    /// Saturday/Sunday in the Monday-first calendar grid.
    pub is_weekend: bool,
}

/// A 6×7 calendar grid for `(year, month)`, with leading/trailing days from the
/// adjacent months dimmed, and `today` (a day-of-month in this month) flagged.
pub fn calendar(year: i32, month: u32, today: Option<u32>) -> Vec<Cell> {
    // The popout's columns are Monday-first (M T W T F S S), so convert the
    // Sunday-first weekday of day 1 into a Monday-first lead.
    let lead = ((weekday(year, month, 1) + 6) % 7) as i32;
    let dim = days_in_month(year, month) as i32;
    let (py, pm) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let prev_days = days_in_month(py, pm) as i32;

    let mut cells = Vec::with_capacity(42);
    for i in 0..42 {
        let idx = i - lead; // 0-based day index within this month
        let is_weekend = i % 7 >= 5;
        if idx < 0 {
            cells.push(Cell {
                day_num: prev_days + idx + 1,
                is_today: false,
                is_other_month: true,
                is_weekend,
            });
        } else if idx < dim {
            let d = idx + 1;
            cells.push(Cell {
                day_num: d,
                is_today: today == Some(d as u32),
                is_other_month: false,
                is_weekend,
            });
        } else {
            cells.push(Cell {
                day_num: idx - dim + 1,
                is_today: false,
                is_other_month: true,
                is_weekend,
            });
        }
    }
    cells
}

pub fn week_numbers(year: i32, month: u32) -> Vec<i32> {
    let cells = calendar(year, month, None);
    let (py, pm) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };

    (0..6)
        .map(|row| {
            let cell = &cells[row * 7];
            let (cy, cm) = if cell.is_other_month && row == 0 {
                (py, pm)
            } else if cell.is_other_month {
                (ny, nm)
            } else {
                (year, month)
            };
            iso_week_number(cy, cm, cell.day_num as u32) as i32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_labels_are_available_for_calendar_header() {
        let labels = weekday_labels_monday_first();
        assert_eq!(labels.len(), 7);
        assert!(labels.iter().all(|label| !label.trim().is_empty()));
    }

    #[test]
    fn calendar_marks_saturday_and_sunday_as_weekends() {
        // July 2026 starts on a Wednesday in the Monday-first grid.
        let cells = calendar(2026, 7, None);
        let saturday = cells
            .iter()
            .find(|cell| !cell.is_other_month && cell.day_num == 4)
            .unwrap();
        let sunday = cells
            .iter()
            .find(|cell| !cell.is_other_month && cell.day_num == 5)
            .unwrap();
        let monday = cells
            .iter()
            .find(|cell| !cell.is_other_month && cell.day_num == 6)
            .unwrap();

        assert!(saturday.is_weekend);
        assert!(sunday.is_weekend);
        assert!(!monday.is_weekend);
    }

    #[test]
    fn formats_unix_timestamp_with_locale_pattern() {
        let formatted = format_unix(1_718_460_000, "%Y").unwrap();
        assert_eq!(formatted, "2024");
    }
}
