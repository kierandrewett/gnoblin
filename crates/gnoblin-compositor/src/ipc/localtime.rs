//! The time of day in the machine's time zone, for the lock card's clock.
//!
//! The compositor links no time library and forbids unsafe code, so it cannot ask libc for the local time. It reads the
//! zone itself, the way libc does: `$TZ` (a zoneinfo name or a POSIX rule string) or `/etc/localtime`, a TZif file. A
//! TZif file lists the zone's transitions up to some year and ends with a POSIX rule that says what happens after them;
//! recent files ("slim") hold few transitions and lean on the rule, so both are read. Only the offset from UTC matters
//! here, not the zone's name or its leap seconds.
//!
//! Anything it cannot read, or cannot parse, is UTC: a clock an hour off is better than none on a lock card, and the
//! card says nothing about the zone.

use std::fs;
use std::path::Path;

const DAY: i64 = 86_400;

/// A zone, as far as the time of day needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Zone {
    /// Transitions in UTC seconds, ascending, each with the offset that holds from it on.
    transitions: Vec<(i64, i32)>,
    /// The offset before the first transition.
    initial: i32,
    /// What holds after the last transition, when the file says.
    rule: Option<Rule>,
}

/// A POSIX rule: a standard offset and, when the zone has daylight saving, the dates it starts and ends.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    standard: i32,
    daylight: Option<Daylight>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Daylight {
    offset: i32,
    start: Date,
    end: Date,
}

/// `Mm.w.d/time`: the `d`th weekday (0 is Sunday) of week `w` (5 is the last) of month `m`, at `time` seconds into the
/// day, in the local time that holds just before the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Date {
    month: u32,
    week: u32,
    weekday: u32,
    time: i32,
}

impl Zone {
    /// UTC, for a machine whose zone cannot be read.
    pub(crate) const UTC: Zone = Zone {
        transitions: Vec::new(),
        initial: 0,
        rule: None,
    };

    /// The zone of this process: `$TZ`, else `/etc/localtime`, else UTC.
    pub(crate) fn system() -> Self {
        match std::env::var("TZ") {
            Ok(tz) if tz.is_empty() => Self::UTC,
            Ok(tz) => Self::named(tz.strip_prefix(':').unwrap_or(&tz)),
            Err(_) => Self::from_file(Path::new("/etc/localtime")).unwrap_or(Self::UTC),
        }
    }

    /// A zone by zoneinfo name or POSIX rule.
    fn named(name: &str) -> Self {
        if !name.contains("..") {
            let path = Path::new("/usr/share/zoneinfo").join(name.trim_start_matches('/'));
            if let Some(zone) = Self::from_file(&path) {
                return zone;
            }
        }
        parse_rule(name).map_or(Self::UTC, |rule| Zone {
            transitions: Vec::new(),
            initial: rule.standard,
            rule: Some(rule),
        })
    }

    fn from_file(path: &Path) -> Option<Self> {
        Self::parse(&fs::read(path).ok()?)
    }

    /// Reads a TZif file (version 1 data, or the 64-bit data of versions 2 and up with its footer).
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        let first = Header::read(bytes)?;
        if first.version == 0 {
            return first.zone(bytes.get(44..)?, 4, None);
        }
        // Version 2 and up: skip the 32-bit block, then the same layout again with 64-bit times and a footer.
        let skip = 44 + first.data_len(4);
        let second = Header::read(bytes.get(skip..)?)?;
        let data = bytes.get(skip + 44..)?;
        let body = second.data_len(8);
        let footer = data.get(body..).and_then(|rest| {
            let rest = rest.strip_prefix(b"\n")?;
            let end = rest.iter().position(|byte| *byte == b'\n')?;
            std::str::from_utf8(&rest[..end]).ok().map(str::to_owned)
        });
        second.zone(data, 8, footer.as_deref())
    }

    /// The offset from UTC, in seconds, that holds at `unix`.
    pub(crate) fn offset_at(&self, unix: i64) -> i32 {
        let after = self.transitions.partition_point(|(at, _)| *at <= unix);
        if after == self.transitions.len()
            && let Some(rule) = &self.rule
        {
            return rule.offset_at(unix);
        }
        match after.checked_sub(1) {
            Some(index) => self.transitions[index].1,
            None => self.initial,
        }
    }

    /// The hour (0 to 23) and minute (0 to 59) of day at `unix`.
    pub(crate) fn hour_and_minute(&self, unix: i64) -> (u32, u32) {
        let local = unix + i64::from(self.offset_at(unix));
        let seconds = local.rem_euclid(DAY);
        let (hour, minute) = (seconds / 3600, (seconds / 60) % 60);
        (u32::try_from(hour).unwrap_or(0), u32::try_from(minute).unwrap_or(0))
    }
}

/// The counts of a TZif header.
struct Header {
    version: u8,
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

impl Header {
    fn read(bytes: &[u8]) -> Option<Self> {
        let head = bytes.get(..44)?;
        if &head[..4] != b"TZif" {
            return None;
        }
        let count = |at: usize| -> Option<usize> {
            let raw: [u8; 4] = head[at..at + 4].try_into().ok()?;
            usize::try_from(u32::from_be_bytes(raw)).ok()
        };
        let version = match head[4] {
            0 => 0,
            digit @ b'2'..=b'9' => digit - b'0',
            _ => return None,
        };
        Some(Self {
            version,
            isutcnt: count(20)?,
            isstdcnt: count(24)?,
            leapcnt: count(28)?,
            timecnt: count(32)?,
            typecnt: count(36)?,
            charcnt: count(40)?,
        })
    }

    /// The size of the data block that follows the header, for times of `time_size` bytes.
    fn data_len(&self, time_size: usize) -> usize {
        self.timecnt * time_size
            + self.timecnt
            + self.typecnt * 6
            + self.charcnt
            + self.leapcnt * (time_size + 4)
            + self.isstdcnt
            + self.isutcnt
    }

    fn zone(&self, data: &[u8], time_size: usize, footer: Option<&str>) -> Option<Zone> {
        let times_end = self.timecnt * time_size;
        let indexes_end = times_end + self.timecnt;
        let types_end = indexes_end + self.typecnt * 6;
        let times = data.get(..times_end)?;
        let indexes = data.get(times_end..indexes_end)?;
        let types = data.get(indexes_end..types_end)?;
        let offsets: Vec<i32> = types
            .chunks_exact(6)
            .map(|chunk| i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        let mut transitions = Vec::with_capacity(self.timecnt);
        for (time, index) in times.chunks_exact(time_size).zip(indexes) {
            let at = if time_size == 8 {
                i64::from_be_bytes(time.try_into().ok()?)
            } else {
                i64::from(i32::from_be_bytes(time.try_into().ok()?))
            };
            transitions.push((at, *offsets.get(usize::from(*index))?));
        }
        // Before the first transition the zone keeps its first standard-time offset, as libc does.
        let initial = types.chunks_exact(6).find(|chunk| chunk[4] == 0).map_or_else(
            || offsets.first().copied(),
            |chunk| Some(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
        )?;
        Some(Zone {
            transitions,
            initial,
            rule: footer.and_then(parse_rule),
        })
    }
}

impl Rule {
    fn offset_at(&self, unix: i64) -> i32 {
        let Some(daylight) = &self.daylight else {
            return self.standard;
        };
        let year = year_of(unix + i64::from(self.standard));
        // The start is written in the standard offset's wall time, the end in daylight time's.
        let start = day_of(year, daylight.start) * DAY + i64::from(daylight.start.time) - i64::from(self.standard);
        let end = day_of(year, daylight.end) * DAY + i64::from(daylight.end.time) - i64::from(daylight.offset);
        let inside = if start < end {
            unix >= start && unix < end
        } else {
            unix >= start || unix < end
        };
        if inside { daylight.offset } else { self.standard }
    }
}

/// Parses a POSIX TZ string: `std offset [dst [offset] [,start[/time],end[/time]]]`. A rule form this does not read (the
/// Julian-day dates) is read as the standard offset alone.
fn parse_rule(text: &str) -> Option<Rule> {
    let mut rest = text;
    skip_name(&mut rest)?;
    let standard = -read_offset(&mut rest)?;
    if rest.is_empty() {
        return Some(Rule {
            standard,
            daylight: None,
        });
    }
    skip_name(&mut rest)?;
    let offset = if rest.is_empty() || rest.starts_with(',') {
        standard + 3600
    } else {
        -read_offset(&mut rest)?
    };
    let Some(dates) = rest.strip_prefix(',') else {
        return Some(Rule {
            standard,
            daylight: None,
        });
    };
    let (start, end) = dates.split_once(',')?;
    match (parse_date(start), parse_date(end)) {
        (Some(start), Some(end)) => Some(Rule {
            standard,
            daylight: Some(Daylight { offset, start, end }),
        }),
        _ => Some(Rule {
            standard,
            daylight: None,
        }),
    }
}

/// Drops a zone name: letters, or anything between `<` and `>`.
fn skip_name(rest: &mut &str) -> Option<()> {
    if let Some(quoted) = rest.strip_prefix('<') {
        let end = quoted.find('>')?;
        *rest = &quoted[end + 1..];
        return Some(());
    }
    let end = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
    if end < 3 {
        return None;
    }
    *rest = &rest[end..];
    Some(())
}

/// Reads `[+-]hh[:mm[:ss]]` as seconds.
fn read_offset(rest: &mut &str) -> Option<i32> {
    let (sign, body) = match rest.as_bytes().first()? {
        b'-' => (-1, &rest[1..]),
        b'+' => (1, &rest[1..]),
        _ => (1, *rest),
    };
    let end = body
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(body.len());
    let mut total = 0;
    for (index, part) in body[..end].split(':').enumerate() {
        let value: i32 = part.parse().ok()?;
        total += value * [3600, 60, 1].get(index).copied()?;
    }
    if end == 0 {
        return None;
    }
    *rest = &body[end..];
    Some(sign * total)
}

/// Reads `Mm.w.d[/time]`.
fn parse_date(text: &str) -> Option<Date> {
    let (date, time) = match text.split_once('/') {
        Some((date, time)) => (date, read_offset(&mut { time })?),
        None => (text, 7200),
    };
    let mut parts = date.strip_prefix('M')?.split('.');
    let month: u32 = parts.next()?.parse().ok()?;
    let week: u32 = parts.next()?.parse().ok()?;
    let weekday: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=5).contains(&week) || weekday > 6 || parts.next().is_some() {
        return None;
    }
    Some(Date {
        month,
        week,
        weekday,
        time,
    })
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year of the day that holds `local` seconds since the epoch.
fn year_of(local: i64) -> i64 {
    let days = local.div_euclid(DAY) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let year = year_of_era + era * 400;
    if shifted_month >= 10 { year + 1 } else { year }
}

/// The day, as days since the epoch, of a rule date in `year`.
fn day_of(year: i64, date: Date) -> i64 {
    let first = days_from_civil(year, date.month, 1);
    // 1970-01-01 was a Thursday (4), so the weekday of a day count is (4 + days) mod 7.
    let first_weekday = u32::try_from((4 + first).rem_euclid(7)).unwrap_or(0);
    let offset = (date.weekday + 7 - first_weekday) % 7;
    let mut day = first + i64::from(offset) + 7 * i64::from(date.week - 1);
    let next_month = if date.month == 12 {
        days_from_civil(year + 1, 1, 1)
    } else {
        days_from_civil(year, date.month + 1, 1)
    };
    // Week 5 means the last such weekday of the month.
    while day >= next_month {
        day -= 7;
    }
    day
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2024-07-01 12:00:00 UTC and 2024-01-15 12:00:00 UTC.
    const SUMMER: i64 = 1_719_835_200;
    const WINTER: i64 = 1_705_320_000;

    fn zone(rule: &str) -> Zone {
        Zone::named(rule)
    }

    #[test]
    fn a_fixed_offset_rule_is_that_offset() {
        let zone = zone("<+0530>-5:30");
        assert_eq!(zone.offset_at(SUMMER), 19_800);
        assert_eq!(zone.hour_and_minute(SUMMER), (17, 30));
    }

    #[test]
    fn northern_daylight_saving_follows_the_rule() {
        let london = zone("GMT0BST,M3.5.0/1,M10.5.0");
        assert_eq!(london.offset_at(WINTER), 0);
        assert_eq!(london.offset_at(SUMMER), 3600);
        assert_eq!(london.hour_and_minute(SUMMER), (13, 0));
        // 2024-03-31 00:59:59 UTC is still GMT; 01:00:00 UTC is BST (the last Sunday of March at 01:00 local).
        assert_eq!(london.offset_at(1_711_846_799), 0);
        assert_eq!(london.offset_at(1_711_846_800), 3600);
        // 2024-10-27 00:59:59 UTC is still BST; 01:00:00 UTC is GMT (02:00 BST on the last Sunday of October).
        assert_eq!(london.offset_at(1_729_990_799), 3600);
        assert_eq!(london.offset_at(1_729_990_800), 0);
    }

    #[test]
    fn southern_daylight_saving_wraps_the_new_year() {
        let sydney = zone("AEST-10AEDT,M10.1.0,M4.1.0/3");
        assert_eq!(sydney.offset_at(WINTER), 39_600, "January is summer there");
        assert_eq!(sydney.offset_at(SUMMER), 36_000, "July is winter there");
    }

    #[test]
    fn the_time_of_day_wraps_before_the_epoch_and_across_midnight() {
        let zone = zone("EST5");
        assert_eq!(zone.hour_and_minute(0), (19, 0), "1969-12-31 19:00 local");
        assert_eq!(zone.hour_and_minute(-1), (18, 59));
        assert_eq!(Zone::UTC.hour_and_minute(86_399), (23, 59));
        assert_eq!(Zone::UTC.hour_and_minute(86_400), (0, 0));
    }

    /// A version 2 file the way a slim zoneinfo build writes it: one transition in 1970 and a footer rule for the rest.
    fn slim_file() -> Vec<u8> {
        fn header() -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"TZif2");
            out.extend_from_slice(&[0; 15]);
            // isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt
            for count in [0_u32, 0, 0, 1, 2, 8] {
                out.extend_from_slice(&count.to_be_bytes());
            }
            out
        }
        let mut out = Vec::new();
        for time_size in [4, 8] {
            out.extend_from_slice(&header());
            if time_size == 4 {
                out.extend_from_slice(&0_i32.to_be_bytes());
            } else {
                out.extend_from_slice(&0_i64.to_be_bytes());
            }
            out.push(1); // the transition at 0 leads to type 1
            out.extend_from_slice(&0_i32.to_be_bytes());
            out.extend_from_slice(&[0, 0]); // type 0: UTC, standard
            out.extend_from_slice(&3600_i32.to_be_bytes());
            out.extend_from_slice(&[0, 4]); // type 1: UTC+1, standard
            out.extend_from_slice(b"UTC\0CET\0");
        }
        out.extend_from_slice(b"\nCET-1CEST,M3.5.0,M10.5.0/3\n");
        out
    }

    #[test]
    fn a_tzif_file_reads_its_transitions_and_its_footer_rule() {
        let zone = Zone::parse(&slim_file()).expect("a zone");
        assert_eq!(zone.offset_at(-1), 0, "before the first transition");
        assert_eq!(zone.offset_at(1), 3600, "at the transition");
        assert_eq!(zone.offset_at(WINTER), 3600, "after it the footer rule holds: CET");
        assert_eq!(zone.offset_at(SUMMER), 7200, "and CEST in July");
    }

    #[test]
    fn a_file_that_is_not_tzif_is_not_a_zone() {
        assert!(Zone::parse(b"not a zone").is_none());
        assert!(Zone::parse(&[]).is_none());
        assert!(Zone::parse(&slim_file()[..30]).is_none());
    }

    #[test]
    fn an_unreadable_rule_is_utc() {
        assert_eq!(zone("???").offset_at(SUMMER), 0);
    }
}
