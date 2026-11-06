//! `log`: print the records the log database holds, oldest first.
//!
//! The database is the one the compositor and the portal write (`gnoblin-log`'s `store`),
//! read back through the query the store defines. It is opened read-only, and this
//! command needs no compositor: it works while the session is down, which is when
//! the log is most wanted.

use std::io::Write;
use std::path::Path;

use gnoblin_log::store::{LogRecord, Query, default_database_path, read_entries};

/// Print the records matching `query` from `database`, or from the default database when none
/// is given. Text is one line per record; `--json` prints one JSON object per line.
pub(crate) fn run(database: Option<&Path>, query: &Query, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match print(database, query, json, out) {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "gnoblinctl: log: {message}");
            1
        }
    }
}

/// The records themselves, written to `out`.
fn print(database: Option<&Path>, query: &Query, json: bool, out: &mut dyn Write) -> Result<(), String> {
    let path = match database {
        Some(path) => path.to_path_buf(),
        None => default_database_path(|name| std::env::var(name).ok())
            .ok_or_else(|| "neither $XDG_STATE_HOME nor $HOME is set, so there is no default database".to_owned())?,
    };
    for entry in read_entries(&path, query)? {
        let line = if json { json_line(&entry) } else { text_line(&entry) };
        writeln!(out, "{line}").map_err(|error| format!("cannot write the records: {error}"))?;
    }
    Ok(())
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
fn text_line(entry: &LogRecord) -> String {
    let mut line = format!(
        "{} {:<5} {} {}: {}",
        iso8601_utc(entry.time_ms),
        entry.level,
        entry.component,
        entry.target,
        entry.message,
    );
    if !entry.fields.is_empty() {
        line.push(' ');
        line.push_str(&entry.fields);
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
            text_line(&entry),
            "1970-01-01T00:00:00.000Z WARN  wayland gnoblin::log: disk full path=/x"
        );
    }
}
