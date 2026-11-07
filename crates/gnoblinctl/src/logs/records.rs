//! The records `log` reads: one JSON object per line in `gnoblin.jsonl` and its rotated copies.
//!
//! The file names and the rotation count are the sink's (`gnoblin_log::file`), so the reader
//! and the writer cannot disagree about where the records are. A line that is not a record is
//! skipped rather than reported: the file is shared by every gnoblin process, and one foreign
//! line must not hide the rest.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use gnoblin_log::file::{FILE_NAME, RETAINED};
use serde_json::Value;

/// One stored record, as `gnoblin_log::file` writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    /// Milliseconds since the epoch.
    pub(crate) time_ms: i64,
    pub(crate) level: String,
    pub(crate) role: String,
    pub(crate) component: String,
    pub(crate) target: String,
    pub(crate) message: String,
    pub(crate) fields: String,
}

impl Record {
    /// Parses one line, or `None` when it is not a complete record.
    pub(crate) fn parse(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line).ok()?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            time_ms: value.get("time_ms")?.as_i64()?,
            level: text("level")?,
            role: text("role")?,
            component: text("component")?,
            target: text("target")?,
            message: text("message")?,
            fields: text("fields")?,
        })
    }
}

/// The filters a read keeps a record for. Every field left `None` keeps everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    /// The least severe level shown, as the writer names it: `WARN` keeps `WARN` and `ERROR`.
    pub(crate) min_level: Option<String>,
    pub(crate) role: Option<String>,
    /// The event target, matched exactly.
    pub(crate) target: Option<String>,
    /// Keeps records at or after this time.
    pub(crate) since_ms: Option<i64>,
    /// Keeps records before this time.
    pub(crate) until_ms: Option<i64>,
}

impl Filter {
    /// Whether `record` passes every filter that is set.
    pub(crate) fn keeps(&self, record: &Record) -> bool {
        self.min_level
            .as_deref()
            .is_none_or(|min| severity(&record.level) <= severity(min))
            && self.role.as_deref().is_none_or(|role| record.role == role)
            && self.target.as_deref().is_none_or(|target| record.target == target)
            && self.since_ms.is_none_or(|since| record.time_ms >= since)
            && self.until_ms.is_none_or(|until| record.time_ms < until)
    }
}

/// The rank of a level name: lower is more severe. A name the writer does not produce ranks
/// last, so it passes no level filter.
fn severity(level: &str) -> u8 {
    match level {
        "ERROR" => 0,
        "WARN" => 1,
        "INFO" => 2,
        "DEBUG" => 3,
        "TRACE" => 4,
        _ => 5,
    }
}

/// The live file's path inside `dir`.
pub(crate) fn live_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// The rotated copies, oldest first: `gnoblin.jsonl.3` down to `gnoblin.jsonl.1`.
pub(crate) fn rotated_paths(dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    (1..=RETAINED)
        .rev()
        .map(|index| dir.join(format!("{FILE_NAME}.{index}")))
}

/// Reads every rotated copy under `dir` that `filter` keeps, oldest first. A missing copy is
/// skipped; it was never written, or it was rotated away again.
pub(crate) fn read_rotated(dir: &Path, filter: &Filter) -> io::Result<Vec<Record>> {
    let mut records = Vec::new();
    for path in rotated_paths(dir) {
        match fs::read(&path) {
            Ok(bytes) => {
                for line in String::from_utf8_lossy(&bytes).lines() {
                    records.extend(Record::parse(line).filter(|record| filter.keeps(record)));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(records)
}

/// The last `limit` records of `records`, which stay in order.
pub(crate) fn keep_newest(records: Vec<Record>, limit: usize) -> Vec<Record> {
    let mut newest: VecDeque<Record> = VecDeque::with_capacity(limit.min(records.len()));
    for record in records {
        if newest.len() == limit {
            newest.pop_front();
        }
        newest.push_back(record);
    }
    newest.into()
}

/// Reads the complete lines of `file` from `from`, appending the records `filter` keeps to
/// `out`, and returns the offset just past the last complete line. A trailing partial line is
/// left for the next read, because the writer has not finished it. A file shorter than `from`
/// was truncated in place, so it is read from its start.
pub(crate) fn read_lines(file: &mut File, from: u64, filter: &Filter, out: &mut Vec<Record>) -> io::Result<u64> {
    let length = file.metadata()?.len();
    let start = if length < from { 0 } else { from };
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let complete = bytes
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |index| index + 1);
    for line in String::from_utf8_lossy(&bytes[..complete]).lines() {
        out.extend(Record::parse(line).filter(|record| filter.keeps(record)));
    }
    Ok(start + complete as u64)
}

/// The device and inode of the file at `path`, or `None` when there is no file there.
pub(crate) fn identity(path: &Path) -> io::Result<Option<(u64, u64)>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.dev(), metadata.ino()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(time_ms: i64, level: &str, target: &str, message: &str) -> String {
        format!(
            "{{\"time_ms\":{time_ms},\"level\":\"{level}\",\"role\":\"compositor\",\
             \"component\":\"log\",\"target\":\"{target}\",\"message\":\"{message}\",\
             \"fields\":\"\"}}"
        )
    }

    #[test]
    fn a_line_that_is_not_a_record_parses_to_none() {
        assert_eq!(Record::parse("not json"), None);
        assert_eq!(Record::parse("{\"time_ms\":1}"), None);
        assert_eq!(
            Record::parse(&line(1, "INFO", "t", "m")).map(|record| record.message),
            Some("m".to_owned())
        );
    }

    #[test]
    fn the_level_filter_keeps_that_level_and_more_severe_ones() {
        let filter = Filter {
            min_level: Some("WARN".to_owned()),
            ..Filter::default()
        };
        let keeps = |level: &str| Record::parse(&line(1, level, "t", "m")).is_some_and(|record| filter.keeps(&record));
        assert!(keeps("ERROR"));
        assert!(keeps("WARN"));
        assert!(!keeps("INFO"));
        assert!(!keeps("TRACE"));
    }

    #[test]
    fn the_time_bounds_are_inclusive_below_and_exclusive_above() {
        let filter = Filter {
            since_ms: Some(10),
            until_ms: Some(20),
            ..Filter::default()
        };
        let keeps = |time| Record::parse(&line(time, "INFO", "t", "m")).is_some_and(|record| filter.keeps(&record));
        assert!(!keeps(9));
        assert!(keeps(10));
        assert!(keeps(19));
        assert!(!keeps(20));
    }

    #[test]
    fn keep_newest_keeps_the_last_records_in_order() {
        let records: Vec<Record> = (1..=5)
            .filter_map(|time| Record::parse(&line(time, "INFO", "t", "m")))
            .collect();
        let kept: Vec<i64> = keep_newest(records, 2).iter().map(|record| record.time_ms).collect();
        assert_eq!(kept, vec![4, 5]);
    }

    #[test]
    fn an_unfinished_last_line_is_left_for_the_next_read() {
        let directory = std::env::temp_dir().join(format!("gnoblinctl-records-{}", std::process::id()));
        fs::create_dir_all(&directory).expect("a scratch directory");
        let path = directory.join("partial.jsonl");
        let whole = line(1, "INFO", "t", "whole");
        fs::write(&path, format!("{whole}\n{{\"time_ms\":2")).expect("written");
        let mut file = File::open(&path).expect("opened");
        let mut out = Vec::new();
        let offset = read_lines(&mut file, 0, &Filter::default(), &mut out).expect("read");
        assert_eq!(offset as usize, whole.len() + 1);
        assert_eq!(out.len(), 1);
    }
}
