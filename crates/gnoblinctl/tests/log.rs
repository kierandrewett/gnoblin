//! `gnoblinctl log` end to end: records written by the logging facade are read back through
//! the command, and the time bounds and follow flags are checked as the user types them.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_log::__tracing::info;
use gnoblin_log::store::{self, Retention};
use gnoblinctl::SocketConnection;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "gnoblinctl-log-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory.join(name)
}

/// Writes `messages` at info through the facade into a fresh database, and returns its path.
fn database_with(messages: &[&str]) -> PathBuf {
    let path = scratch("log.sqlite");
    let sink = store::open(&path, "compositor", Retention::default()).expect("the database opens");
    store::with_database(&sink, "compositor", || {
        for message in messages {
            info!(target: "gnoblin::test", "{message}");
        }
    });
    sink.flush();
    path
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
    let path = database_with(&["first", "second"]);
    let (code, out, err) = run(&["log", "--database", &path.display().to_string()]);
    assert_eq!(code, 0, "stderr: {err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].contains("first") && lines[1].contains("second"), "{out}");
}

#[test]
fn since_and_until_bound_the_records_by_their_age() {
    let path = database_with(&["recent"]);
    let db = path.display().to_string();

    let (code, out, err) = run(&["log", "--database", &db, "--since", "1d"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("recent"),
        "a record from moments ago is within a day: {out}"
    );

    let (code, out, err) = run(&["log", "--database", &db, "--until", "1d"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.is_empty(), "nothing is older than a day ago: {out}");
}

#[test]
fn an_rfc3339_bound_is_accepted() {
    let path = database_with(&["stamped"]);
    let (code, out, err) = run(&[
        "log",
        "--database",
        &path.display().to_string(),
        "--since",
        "2000-01-01T00:00:00Z",
    ]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("stamped"), "{out}");
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
