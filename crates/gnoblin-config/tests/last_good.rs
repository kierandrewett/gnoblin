//! The last-good store: the document that was accepted last, and the reasons a
//! stored document is refused.
//!
//! Every test here writes to a private temporary directory, so the state of the
//! developer's own session is never read or changed.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use gnoblin_config::{Config, Fault, Resolution, Status, Store, StoreError};
use gnoblin_testkit::RuntimeDir;
use serde_json::json;

use support::{flag, int, number, table, text};

fn config(document: gnoblin_config::Value) -> Config {
    Config::from_value(document).expect("the test document is valid")
}

fn source(root: &RuntimeDir) -> PathBuf {
    root.join("config").join("gnoblin").join("init.lua")
}

/// Write a raw envelope, with the modes the store expects.
fn write_raw(store: &Store, contents: &[u8]) {
    fs::create_dir_all(store.directory()).expect("state directory");
    fs::set_permissions(store.directory(), fs::Permissions::from_mode(0o700)).expect("private directory");
    fs::write(store.path(), contents).expect("raw document");
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o600)).expect("private file");
}

fn refusal(key: &str) -> gnoblin_config::Report {
    // A report comes from a real validation failure, so the store tests never
    // depend on a constructor the public API does not offer.
    Config::from_value(table([(key, gnoblin_config::Value::Bool(true))]))
        .expect_err("an unknown top-level key is refused")
}

#[test]
fn a_saved_document_loads_back_with_its_source() {
    let root = RuntimeDir::new("last-good-round-trip").expect("temp dir");
    let store = Store::new(root.path());
    let path = source(&root);
    let saved = config(table([("cursor", table([("size", int(32))]))]));
    store.save(&path, &saved).expect("save");

    let loaded = store.load(&path).expect("load");
    assert_eq!(loaded.format, gnoblin_config::FORMAT);
    assert_eq!(&loaded.source, &path);
    assert_eq!(loaded.config, saved);
}

#[test]
fn the_stored_document_is_private_and_leaves_no_temporary_file() {
    let root = RuntimeDir::new("last-good-private").expect("temp dir");
    let store = Store::new(root.path());
    store
        .save(&source(&root), &config(table([("cursor", table([("size", int(24))]))])))
        .expect("save");

    let directory = fs::metadata(store.directory()).expect("directory metadata");
    assert_eq!(directory.permissions().mode() & 0o777, 0o700);
    let file = fs::metadata(store.path()).expect("file metadata");
    assert_eq!(file.permissions().mode() & 0o777, 0o600);

    let names: Vec<String> = fs::read_dir(store.directory())
        .expect("read the state directory")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["last-good-config.json".to_owned()]);
}

#[test]
fn a_stored_document_holds_no_deprecated_spelling() {
    let root = RuntimeDir::new("last-good-alias").expect("temp dir");
    let store = Store::new(root.path());
    let with_alias = config(table([("window-management", table([("focus-mode", text("sloppy"))]))]));
    assert_eq!(
        with_alias.deprecations().len(),
        1,
        "the document used a deprecated spelling"
    );
    store.save(&source(&root), &with_alias).expect("save");

    let stored = fs::read_to_string(store.path()).expect("read the stored document");
    assert!(stored.contains("hover"), "the current spelling is stored");
    assert!(!stored.contains("sloppy"), "the deprecated spelling is not stored");
    let loaded = store.load(&source(&root)).expect("load");
    assert!(
        loaded.config.deprecations().is_empty(),
        "a stored document needs no rewrite"
    );
}

#[test]
fn a_truncated_document_is_refused() {
    let root = RuntimeDir::new("last-good-torn").expect("temp dir");
    let store = Store::new(root.path());
    write_raw(&store, b"{\"format\":1,\"source\":\"/tmp/init.lua\",\"docu");
    match store.load(&source(&root)) {
        Err(StoreError::Malformed { .. }) => {}
        other => panic!("a torn write must be refused, got {other:?}"),
    }
}

#[test]
fn a_document_from_an_older_schema_is_refused_with_its_problem() {
    let root = RuntimeDir::new("last-good-old-schema").expect("temp dir");
    let store = Store::new(root.path());
    let path = source(&root);
    let envelope = json!({
        "format": 1,
        "source": path.to_string_lossy(),
        "document": {"window-management": {"center-new-windows": true}},
    });
    write_raw(&store, serde_json::to_string(&envelope).expect("encode").as_bytes());
    match store.load(&path) {
        Err(StoreError::Rejected(report)) => {
            assert_eq!(report.problems().len(), 1);
            assert_eq!(
                report.problems()[0].path.to_string(),
                "window-management.center-new-windows"
            );
            assert!(matches!(report.problems()[0].fault, Fault::Removed { .. }));
        }
        other => panic!("an older schema must be refused, got {other:?}"),
    }
}

#[test]
fn an_unknown_envelope_format_is_refused() {
    let root = RuntimeDir::new("last-good-format").expect("temp dir");
    let store = Store::new(root.path());
    let path = source(&root);
    let envelope = json!({"format": 2, "source": path.to_string_lossy(), "document": {}});
    write_raw(&store, serde_json::to_string(&envelope).expect("encode").as_bytes());
    match store.load(&path) {
        Err(StoreError::Format { found }) => assert_eq!(found, 2),
        other => panic!("a newer format must be refused, got {other:?}"),
    }
}

#[test]
fn a_document_stored_for_another_path_is_refused() {
    let root = RuntimeDir::new("last-good-other-source").expect("temp dir");
    let store = Store::new(root.path());
    store
        .save(
            &root.join("one").join("init.lua"),
            &config(table([("prompts", table([("enabled", flag(true))]))])),
        )
        .expect("save");
    match store.load(&root.join("two").join("init.lua")) {
        Err(StoreError::OtherSource { stored }) => assert_eq!(stored, root.join("one").join("init.lua")),
        other => panic!("another path's document must be refused, got {other:?}"),
    }
}

#[test]
fn a_stored_document_readable_by_other_users_is_refused() {
    let root = RuntimeDir::new("last-good-insecure").expect("temp dir");
    let store = Store::new(root.path());
    store
        .save(&source(&root), &config(table([("cursor", table([("size", int(24))]))])))
        .expect("save");
    fs::set_permissions(store.path(), fs::Permissions::from_mode(0o644)).expect("loosen the file");
    match store.load(&source(&root)) {
        Err(StoreError::Insecure { .. }) => {}
        other => panic!("a world-readable document must be refused, got {other:?}"),
    }
}

#[test]
fn accept_stores_the_document_it_accepted() {
    let root = RuntimeDir::new("last-good-accept").expect("temp dir");
    let store = Store::new(root.path());
    let path = source(&root);
    let resolution = store.accept(&path, Ok(config(table([("cursor", table([("size", int(48))]))]))));
    assert_eq!(resolution.status_name(), "current");
    assert!(resolution.save_error.is_none(), "the save succeeded");
    assert_eq!(
        resolution
            .config()
            .expect("a document")
            .cursor
            .as_ref()
            .expect("cursor")
            .size,
        Some(48)
    );
    assert!(store.path().is_file(), "the accepted document is stored");
}

#[test]
fn accept_falls_back_to_the_last_accepted_document() {
    let root = RuntimeDir::new("last-good-fallback").expect("temp dir");
    let store = Store::new(root.path());
    let path = source(&root);
    let good = config(table([
        ("cursor", table([("size", int(24))])),
        ("input", table([("mouse", table([("speed", number(0.25))]))])),
    ]));
    store.save(&path, &good).expect("save");

    let resolution: Resolution = store.accept(&path, Err(refusal("broken")));
    assert_eq!(resolution.status_name(), "using-last-good");
    assert_eq!(resolution.config(), Some(&good), "the stored document is applied");
    let reason = resolution.status.reason().expect("the refusal is the reason");
    assert_eq!(reason.problems()[0].path.to_string(), "broken");
}

#[test]
fn accept_without_a_stored_document_is_unusable() {
    let root = RuntimeDir::new("last-good-unusable").expect("temp dir");
    let store = Store::new(root.path());
    let resolution = store.accept(&source(&root), Err(refusal("broken")));
    assert_eq!(resolution.status_name(), "unusable");
    assert!(
        resolution.config().is_none(),
        "the caller falls back to its own defaults"
    );
    assert!(
        resolution.status.last_good_error().is_some(),
        "the shell can say why no stored document was used"
    );
    assert!(matches!(resolution.status, Status::Unusable { .. }));
}

#[test]
fn a_save_that_cannot_write_keeps_the_document_and_reports_the_failure() {
    let root = RuntimeDir::new("last-good-save-failure").expect("temp dir");
    let store = Store::new(root.path());
    // A regular file where the state directory belongs makes every write fail
    // without making the test run as a user who cannot write to the temporary
    // directory.
    fs::write(store.directory(), b"not a directory").expect("block the state directory");
    let resolution = store.accept(
        &source(&root),
        Ok(config(table([("cursor", table([("size", int(24))]))]))),
    );
    assert_eq!(
        resolution.status_name(),
        "current",
        "the document is still the accepted one"
    );
    assert!(resolution.config().is_some());
    assert!(
        resolution.save_error.is_some(),
        "the failure is reported so it can be logged"
    );
}
