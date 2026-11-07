//! `log`: print the records the compositor, the portal and the other gnoblin processes wrote to
//! `gnoblin.jsonl`, oldest first.
//!
//! The file and its rotated copies are read directly, so this command needs no compositor and
//! works while the session is down, which is when the log is most wanted. The file layout is the
//! sink's own (`gnoblin_log::file`), so the reader and the writer agree on the names.
//!
//! `--follow` keeps printing new records as they are written, polling every `--interval`. It
//! stops when the process is interrupted, or when the output pipe closes. Interruption is the
//! default signal action, so no record is half-written: each record is one line.

mod bounds;
mod follow;
mod records;

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_cli_style::{ColorChoice, ColourToken, OutputEnvironment, OutputTheme};
use gnoblin_log::file::default_dir;

pub(crate) use bounds::{TimeBound, parse_bound};
pub(crate) use records::Filter;
use records::Record;

/// The default poll interval for `--follow`.
pub(crate) const DEFAULT_INTERVAL: Duration = Duration::from_millis(500);

/// One `log` invocation, as parsed from the command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LogRequest {
    /// The log directory to read; `None` for the default one.
    pub(crate) dir: Option<PathBuf>,
    /// The level, role and target filters. The time bounds are not in it: they are resolved
    /// against the clock when the command runs, from `since` and `until`.
    pub(crate) filter: Filter,
    /// How many of the newest matching records to print.
    pub(crate) limit: usize,
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
    let dir = match &request.dir {
        Some(path) => path.clone(),
        None => default_dir(|name| std::env::var(name).ok()).ok_or_else(|| {
            Failure::Message(
                "neither $XDG_STATE_HOME nor $HOME is set, so there is no default log directory".to_owned(),
            )
        })?,
    };
    let (since_ms, until_ms) = bounds::resolve(request.since, request.until, now_ms()).map_err(Failure::Message)?;
    let filter = Filter {
        since_ms,
        until_ms,
        ..request.filter.clone()
    };
    let live = records::live_path(&dir);
    if request.follow.is_none() && !live.exists() {
        // A mistyped `--dir` is reported rather than shown as an empty log. Following waits
        // for the file instead, because the session may be about to start writing it.
        return Err(Failure::Message(format!("no log file at {}", live.display())));
    }

    let theme = OutputTheme::resolve(ColorChoice::Auto, &OutputEnvironment::detect_stdout());
    let json = request.json;
    let mut emit = |records: &[Record]| -> io::Result<()> {
        for record in records {
            let line = if json {
                json_line(record)
            } else {
                text_line(record, &theme)
            };
            writeln!(out, "{line}")?;
        }
        out.flush()
    };
    let mut follower = follow::Follower::new(dir.clone(), filter, request.limit);
    match request.follow {
        None => {
            let records = follower.start().map_err(|error| read_failure(&dir, &error))?;
            emit(&records).map_err(write_failure)
        }
        Some(interval) => {
            let mut report = |message: &str| {
                let _ = writeln!(err, "gnoblinctl: log: {message}");
            };
            follow::follow(&mut follower, interval, &mut emit, &mut report).map_err(write_failure)
        }
    }
}

fn read_failure(dir: &std::path::Path, error: &io::Error) -> Failure {
    Failure::Message(format!("cannot read the log in {}: {error}", dir.display()))
}

fn write_failure(error: io::Error) -> Failure {
    if error.kind() == io::ErrorKind::BrokenPipe {
        Failure::Silent
    } else {
        Failure::Message(format!("cannot write the records: {error}"))
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
fn json_line(record: &Record) -> String {
    serde_json::json!({
        "time_ms": record.time_ms,
        "level": record.level,
        "component": record.component,
        "role": record.role,
        "target": record.target,
        "message": record.message,
        "fields": record.fields,
    })
    .to_string()
}

/// One record as text: UTC time, level, component, target, message, then the fields if any.
/// The level is coloured by severity, and the colour is dropped when the theme says so.
fn text_line(record: &Record, theme: &OutputTheme) -> String {
    let level = match record.level.as_str() {
        "ERROR" => ColourToken::Danger,
        "WARN" => ColourToken::Warning,
        "INFO" => ColourToken::Info,
        _ => ColourToken::Muted,
    };
    let mut line = format!(
        "{} {} {} {}: {}",
        theme.paint(ColourToken::Muted, &iso8601_utc(record.time_ms)),
        theme.paint(level, &format!("{:<5}", record.level)),
        theme.paint(ColourToken::Accent, &record.component),
        record.target,
        record.message,
    );
    if !record.fields.is_empty() {
        line.push(' ');
        line.push_str(&theme.paint(ColourToken::Muted, &record.fields));
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

    fn record(level: &str, target: &str, message: &str, fields: &str) -> Record {
        Record {
            time_ms: 0,
            level: level.to_owned(),
            role: "compositor".to_owned(),
            component: "wayland".to_owned(),
            target: target.to_owned(),
            message: message.to_owned(),
            fields: fields.to_owned(),
        }
    }

    #[test]
    fn the_epoch_and_a_known_instant_format_as_utc() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn a_text_line_names_the_time_level_component_target_and_message_then_the_fields() {
        let entry = record("WARN", "gnoblin::log", "disk full", "path=/x");
        assert_eq!(
            text_line(&entry, &plain()),
            "1970-01-01T00:00:00.000Z WARN  wayland gnoblin::log: disk full path=/x"
        );
    }

    #[test]
    fn a_coloured_line_keeps_the_same_words_and_adds_escapes() {
        let entry = record("ERROR", "t", "m", "");
        let coloured = text_line(&entry, &OutputTheme::new(true, false, 80));
        assert!(coloured.contains("\x1b[31mERROR\x1b[0m"));
        assert_eq!(gnoblin_cli_style::strip_ansi(&coloured), text_line(&entry, &plain()));
    }

    #[test]
    fn a_json_line_carries_the_record_fields_without_an_id() {
        let entry = record("INFO", "t", "m", "k=v");
        let value: serde_json::Value = serde_json::from_str(&json_line(&entry)).expect("valid JSON");
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["fields"], "k=v");
        assert!(value.get("id").is_none());
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
