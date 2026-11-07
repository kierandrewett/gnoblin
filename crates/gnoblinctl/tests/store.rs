//! `gnoblinctl store` end to end: namespaces written through the store are read back through the
//! command, and the read-only verbs refuse a missing store rather than creating one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_model::Value;
use gnoblin_store::{Retention, Store};
use gnoblinctl::SocketConnection;

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "gnoblinctl-store-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory.join(name)
}

/// A store directory holding one namespace `theme` with `alpha` set to a text value, and the log
/// kind, which the listing must not show as a namespace.
fn store_with_namespace() -> PathBuf {
    let root = scratch("store");
    let store = Store::open(&root).expect("the store opens");
    let theme = store.namespace("theme").expect("the namespace opens");
    theme
        .put("alpha", Value::Text("dark".to_owned()))
        .expect("the write succeeds");
    store.logs(Retention::default()).expect("the log kind opens");
    root
}

/// A connection nothing listens at. The store command never connects, so it is never used.
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

fn dir(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn list_shows_each_namespace_and_not_the_log_file() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["store", "list", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("NAME"), "{out}");
    assert!(out.contains("theme"), "{out}");
    assert!(out.contains("ready"), "{out}");
    assert!(!out.contains("log"), "the log file is not a namespace: {out}");
}

#[test]
fn list_on_a_missing_directory_is_empty_and_creates_nothing() {
    let root = scratch("never-made");
    let (code, out, err) = run(&["store", "list", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("no stores at"), "{out}");
    assert!(!root.exists(), "a read must not create the directory");
}

#[test]
fn get_prints_the_stored_value() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["store", "get", "theme", "alpha", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), "\"dark\"", "{out}");
}

#[test]
fn get_json_carries_the_key_value_revision_and_write_time() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["--json", "store", "get", "theme", "alpha", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    let entry: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON object");
    assert_eq!(entry["key"], "alpha");
    assert_eq!(entry["value"], "dark");
    assert_eq!(entry["revision"], 1);
    assert!(entry["updated_ms"].as_i64().is_some(), "{out}");
}

#[test]
fn get_of_an_unset_key_is_an_error_not_an_empty_value() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["store", "get", "theme", "missing", "--store", &dir(&root)]);
    assert_eq!(code, 1, "stdout: {out}");
    assert!(err.contains("missing is not set in theme"), "{err}");
}

#[test]
fn a_mistyped_store_is_reported_and_not_created() {
    let root = store_with_namespace();
    let (code, _, err) = run(&["store", "get", "themes", "alpha", "--store", &dir(&root)]);
    assert_eq!(code, 1);
    assert!(err.contains("no store named themes"), "{err}");
    assert!(
        !root.join("themes.sqlite3").exists(),
        "a read must not create a store file"
    );
}

#[test]
fn usage_reports_bytes_against_the_quota_for_every_namespace() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["--json", "store", "usage", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    let rows: serde_json::Value = serde_json::from_str(out.trim()).expect("a JSON array");
    let row = &rows[0];
    assert_eq!(row["name"], "theme");
    assert!(row["bytes"].as_u64().is_some_and(|bytes| bytes > 0), "{out}");
    assert_eq!(row["quota"], gnoblin_store::DEFAULT_QUOTA);
}

#[test]
fn usage_names_one_store_when_asked() {
    let root = store_with_namespace();
    let (code, out, err) = run(&["store", "usage", "theme", "--store", &dir(&root)]);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.contains("theme"), "{out}");
    assert!(out.contains("QUOTA"), "{out}");
}

#[test]
fn a_store_verb_is_required() {
    let (code, _, err) = run(&["store"]);
    assert_ne!(code, 0);
    assert!(err.contains("store"), "{err}");
}

#[test]
fn the_verbs_that_would_write_are_not_offered() {
    let (code, _, err) = run(&["store", "put", "theme", "alpha", "1"]);
    assert_ne!(code, 0, "writes are blocked on the grant design: {err}");
}
