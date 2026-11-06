//! `log`: print the records the log database holds, oldest first.
//!
//! The database is the one the compositor and the portal write (`gnoblin-log`'s `store`),
//! read back through the query the store defines. It is opened read-only, and this
//! command needs no compositor: it works while the session is down, which is when
//! the log is most wanted.
//!
//! `--follow` keeps printing new records as they are written. It polls the store every
//! `--interval` and stops when the process is interrupted, or when the output pipe closes.
//! Interruption is the default signal action, so no record is half-written: each record
//! is one line, flushed with its batch.

mod bounds;
mod follow;

use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_cli_style::{ColorChoice, ColourToken, OutputEnvironment, OutputTheme};
use gnoblin_log::store::{LogRecord, Query, default_database_path, newest_id, read_entries};

pub(crate) use bounds::{TimeBound, parse_bound};

/// The default poll interval for `--follow`.
pub(crate) const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);

/// One `log` invocation, as parsed from the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LogRequest {
    /// The database to read; `None` for the default one.
    pub(crate) database: Option<PathBuf>,
    /// The filters. The time bounds are not in it: they are resolved against the clock when
    /// the command runs, from `since` and `until`.
    pub(crate) query: Query,
    pub(crate) since: Option<TimeBound>,
    pub(crate) until: Option<TimeBound>,
    /// `Some(interval)` to keep following; `None` to print once and exit.
    pub(crate) follow: Option<Duration>,
    pub(crate) json: bool,
}

/// Print the records matching the request, or follow them. Text is one line per record;
/// `--json` prints one JSON object per line.
pub(crate) fn run(request: &LogRequest, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match execute(request, out, err) {
        Ok(()) => 0,
        Err(Failure::Silent) => 0,
        Err(Failure::Message(message)) => {
            let _ = writeln!(err, "gnoblinctl: log: {message}");
            1
        }
    }
}

/// Why `log` stopped early. A closed pipe is not a failure: the reader has what it wanted.
enum Failure {
    Message(String),
    Silent,
}

fn execute(request: &LogRequest, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), Failure> {
    let path = match &request.database {
        Some(path) => path.clone(),
        None => default_database_path(|name| std::env::var(name).ok()).ok_or_else(|| {
            Failure::Message("neither $XDG_STATE_HOME nor $HOME is set, so there is no default database".to_owned())
        })?,
    };
    let (since_ms, until_ms) = bounds::resolve(request.since, request.until, now_ms()).map_err(Failure::Message)?;
    let query = Query {
        since_ms,
        until_ms,
        ..request.query.clone()
    };
    let theme = OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect_stdout());
    let json = request.json;
    let mut emit = |records: &[LogRecord]| -> io::Result<()> {
        for entry in records {
            let line = if json {
                json_line(entry)
            } else {
                text_line(entry, &theme)
            };
            writeln!(out, "{line}")?;
        }
        out.flush()
    };

    let outcome = match request.follow {
        None => read_entries(&path, &query)
            .map_err(FollowOutcome::Read)
            .and_then(|records| emit(&records).map_err(FollowOutcome::Write)),
        Some(interval) => {
            let mut follower = follow::Follower::new(FileSource { path: &path }, query);
            let mut report = |message: &str| {
                let _ = writeln!(err, "gnoblinctl: log: {message}");
            };
            follow::run(&mut follower, interval, &mut emit, &mut report, |wait| {
                std::thread::sleep(wait);
                true
            })
            .map_err(FollowOutcome::Write)
        }
    };
    match outcome {
        Ok(()) => Ok(()),
        Err(FollowOutcome::Write(error)) if error.kind() == io::ErrorKind::BrokenPipe => Err(Failure::Silent),
        Err(FollowOutcome::Write(error)) => Err(Failure::Message(format!("cannot write the records: {error}"))),
        Err(FollowOutcome::Read(message)) => Err(Failure::Message(message)),
    }
}

/// Either kind of failure from reading or writing, before it is turned into a [`Failure`].
enum FollowOutcome {
    Read(String),
    Write(io::Error),
}

/// The store at a path, read by the follower. The identity is the file's device and inode, so a
/// replaced database is noticed; a missing file has no identity and is simply waited for.
struct FileSource<'a> {
    path: &'a Path,
}

impl follow::Source for FileSource<'_> {
    fn identity(&self) -> Option<(u64, u64)> {
        std::fs::metadata(self.path)
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    }

    fn newest_id(&self) -> Result<i64, String> {
        newest_id(self.path)
    }

    fn read(&self, query: &Query) -> Result<Vec<LogRecord>, String> {
        read_entries(self.path, query)
    }
}

/// The current time in milliseconds since the epoch.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Parses `--interval`: a whole number of milliseconds (`500ms`) or seconds (`2s`), above zero.
/// The error names the flag and the value.
pub(crate) fn parse_interval(text: &str) -> Result<Duration, String> {
    let refuse = || format!("--interval: \"{text}\" is not an interval such as 500ms or 2s");
    let (digits, unit_ms) = if let Some(digits) = text.strip_suffix("ms") {
        (digits, 1_u64)
    } else if let Some(digits) = text.strip_suffix('s') {
        (digits, 1_000)
    } else {
        return Err(refuse());
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(refuse());
    }
    let count: u64 = digits.parse().map_err(|_| refuse())?;
    let millis = count.checked_mul(unit_ms).ok_or_else(refuse)?;
    if millis == 0 {
        return Err(format!("--interval: \"{text}\" must be longer than zero"));
    }
    Ok(Duration::from_millis(millis))
}

/// One record as a JSON object. `time_ms` is milliseconds since the epoch, for scripts.
fn json_line(entry: &LogRecord) -> String {
    serde_json::json!({
        "id": entry.id,
        "time_ms": entry.time_ms,
        "level": entry.level,
        "component": entry.component,
        "role": entry.role,
        "target": entry.target,
        "message": entry.message,
        "fields": entry.fields,
    })
    .to_string()
}

/// One record as text: UTC time, level, component, target, message, then the fields if any.
/// The level is coloured by severity, and the colour is dropped when the theme says so.
fn text_line(entry: &LogRecord, theme: &OutputTheme) -> String {
    let level = match entry.level.as_str() {
        "ERROR" => ColourToken::Danger,
        "WARN" => ColourToken::Warning,
        "INFO" => ColourToken::Info,
        _ => ColourToken::Muted,
    };
    let mut line = format!(
        "{} {} {} {}: {}",
        theme.paint(ColourToken::Muted, &iso8601_utc(entry.time_ms)),
        theme.paint(level, &format!("{:<5}", entry.level)),
        theme.paint(ColourToken::Accent, &entry.component),
        entry.target,
        entry.message,
    );
    if !entry.fields.is_empty() {
        line.push(' ');
        line.push_str(&theme.paint(ColourToken::Muted, &entry.fields));
    }
    line
}

/// Formats milliseconds since the epoch as an ISO 8601 UTC timestamp with milliseconds.
fn iso8601_utc(time_ms: i64) -> String {
    let seconds = time_ms.div_euclid(1_000);
    let millis = time_ms.rem_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let second_of_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60,
    )
}

/// The proleptic Gregorian date of a day count from 1970-01-01, as `(year, month, day)`.
/// Howard Hinnant's `civil_from_days`, which needs no calendar crate.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> OutputTheme {
        OutputTheme::new(false, false, 80)
    }

    #[test]
    fn the_epoch_and_a_known_instant_format_as_utc() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn a_text_line_names_the_time_level_component_target_and_message_then_the_fields() {
        let entry = LogRecord {
            id: 7,
            time_ms: 0,
            level: "WARN".to_owned(),
            component: "wayland".to_owned(),
            role: "compositor".to_owned(),
            target: "gnoblin::log".to_owned(),
            message: "disk full".to_owned(),
            fields: "path=/x".to_owned(),
        };
        assert_eq!(
            text_line(&entry, &plain()),
            "1970-01-01T00:00:00.000Z WARN  wayland gnoblin::log: disk full path=/x"
        );
    }

    #[test]
    fn a_coloured_line_keeps_the_same_words_and_adds_escapes() {
        let entry = LogRecord {
            id: 1,
            time_ms: 0,
            level: "ERROR".to_owned(),
            component: "wayland".to_owned(),
            role: "compositor".to_owned(),
            target: "t".to_owned(),
            message: "m".to_owned(),
            fields: String::new(),
        };
        let coloured = text_line(&entry, &OutputTheme::new(true, false, 80));
        assert!(coloured.contains("\x1b[31mERROR\x1b[0m"));
        assert_eq!(gnoblin_cli_style::strip_ansi(&coloured), text_line(&entry, &plain()));
    }

    #[test]
    fn intervals_are_milliseconds_or_seconds_above_zero() {
        assert_eq!(parse_interval("500ms"), Ok(Duration::from_millis(500)));
        assert_eq!(parse_interval("2s"), Ok(Duration::from_secs(2)));
        assert_eq!(parse_interval("1ms"), Ok(Duration::from_millis(1)));
        assert_eq!(DEFAULT_INTERVAL, Duration::from_millis(500));
    }

    #[test]
    fn a_bad_interval_names_the_flag_and_the_value() {
        for text in [
            "",
            "ms",
            "0ms",
            "0s",
            "1.5s",
            "10",
            "-1s",
            "2m",
            "5 s",
            "99999999999999999999s",
        ] {
            let message = parse_interval(text).expect_err(text);
            assert!(message.starts_with("--interval: "), "{message}");
            assert!(message.contains(&format!("\"{text}\"")), "{message}");
        }
    }
}
