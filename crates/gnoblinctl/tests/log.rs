//! `gnoblinctl log` end to end: records written by the file sink are read back through the
//! command, and the time bounds and follow flags are checked as the user types them.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gnoblin_log::file::FileSink;
use gnoblin_log::record::{LogRecord, RecordSink};
use gnoblinctl::SocketConnection;

/// A fresh scratch directory, unique to this process and call.
fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "gnoblinctl-log-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

/// Writes `messages` at info into a fresh log directory, and returns its path. The records are
/// written through the file sink with the time now, so the time bounds see them as recent.
fn log_dir_with(messages: &[&str]) -> PathBuf {
    let dir = scratch("log");
    let sink = FileSink::open(&dir, None).expect("the log file opens");
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_millis() as i64;
    let records: Vec<LogRecord> = messages
        .iter()
        .map(|message| LogRecord {
            time_ms,
            level: "INFO",
            component: "compositor".to_owned(),
            role: "compositor",
            target: "gnoblin::test".to_owned(),
            message: (*message).to_owned(),
            fields: String::new(),
        })
        .collect();
    sink.write_batch(&records);
    dir
}

/// A connection nothing listens at. The log command never connects, so it is never used.
fn nowhere() -> SocketConnection {
    SocketConnection::at(scratch("absent.sock"))
}

/// Runs one command line, returning its exit code and streams.
fn run(args: &[&str]) -> (u8, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = gnoblinctl::run(
        args.iter().map(|arg| OsString::from(*arg)),
        nowhere(),
        &mut out,
        &mut err,
    );
    (
        code,
        String::from_utf8(out).expect("UTF-8 output"),
        String::from_utf8(err).expect("UTF-8 output"),
    )
}

#[test]
fn printing_once_shows_every_record_oldest_first() {
    let dir = log_dir_with(&["first", "second"]);
    let (code, out, err) = run(&["log", "--dir", &dir.display().to_string()]);
    assert_eq!(code, 0, "stderr: {err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].contains("first") && lines[1].contains("second"), "{out}");
}

#[test]
fn since_and_until_bound_the_records_by_their_age() {
    let dir = log_dir_with(&["recent"]);
    let db = dir.display().to_string();

    let (code, out, err) = run(&["log", "--dir", &db, "--since", "1d"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("recent"),
        "a record from moments ago is within a day: {out}"
    );

    let (code, out, err) = run(&["log", "--dir", &db, "--until", "1d"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.is_empty(), "nothing is older than a day ago: {out}");
}

#[test]
fn an_rfc3339_bound_is_accepted() {
    let dir = log_dir_with(&["stamped"]);
    let (code, out, err) = run(&[
        "log",
        "--dir",
        &dir.display().to_string(),
        "--since",
        "2000-01-01T00:00:00Z",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("stamped"), "{out}");
}

#[test]
fn the_removed_store_flag_is_a_usage_error() {
    let (code, out, err) = run(&["log", "--store", "/tmp"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("--store"), "the flag is named: {err}");
}

#[test]
fn a_malformed_bound_is_a_usage_error_that_names_the_value() {
    let (code, out, err) = run(&["log", "--since", "yesterday"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("yesterday"), "the value is named: {err}");
}

#[test]
fn interval_without_follow_is_a_usage_error() {
    let (code, out, err) = run(&["log", "--interval", "1s"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert!(!err.is_empty(), "the refusal is said");
}

#[test]
fn follow_and_until_conflict() {
    let (code, out, err) = run(&["log", "--follow", "--until", "1h"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(out.is_empty(), "{out}");
    assert!(!err.is_empty(), "the conflict is said");
}

#[test]
fn a_zero_interval_is_a_usage_error() {
    let (code, _, err) = run(&["log", "--follow", "--interval", "0ms"]);
    assert_eq!(code, 2, "stderr: {err}");
    assert!(err.contains("0ms"), "the value is named: {err}");
}
