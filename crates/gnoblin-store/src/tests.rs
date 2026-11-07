//! End-to-end tests of the consumer API against real files in a scratch directory. Each test gets
//! its own directory so the files of one test never leak into another.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gnoblin_model::Value;

use crate::{BatchOutcome, Change, DEFAULT_VALUE_LIMIT, Op, Options, Order, Query, State, Store};

/// A fresh directory under the system temp dir, unique to this process and call.
pub(crate) fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "gnoblin-store-test-{}-{label}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn open(label: &str) -> (Store, PathBuf) {
    let dir = scratch(label);
    (Store::open(&dir).expect("store opens"), dir)
}

#[test]
fn a_value_round_trips_with_its_revision_and_time() {
    let (store, _dir) = open("round-trip");
    let ns = store.namespace("prefs").unwrap();
    let rev = ns.put("ui/theme", Value::Text("dark".into())).unwrap();
    let entry = ns.get("ui/theme").unwrap().expect("present");
    assert_eq!(entry.key, "ui/theme");
    assert_eq!(entry.value, Value::Text("dark".into()));
    assert_eq!(entry.revision, rev);
    assert!(entry.updated_ms > 0);
    assert!(ns.get("ui/missing").unwrap().is_none());
}

#[test]
fn a_second_put_moves_the_revision_and_delete_reports_what_it_removed() {
    let (store, _dir) = open("revisions");
    let ns = store.namespace("prefs").unwrap();
    let first = ns.put("a", Value::Int(1)).unwrap();
    let second = ns.put("a", Value::Int(2)).unwrap();
    assert!(second > first);
    assert!(ns.delete("a").unwrap());
    assert!(!ns.delete("a").unwrap());
}

#[test]
fn a_namespace_name_cannot_leave_the_root_or_be_malformed() {
    let (store, _dir) = open("names");
    for bad in ["", "../escape", "a/b", "Upper", "with space", &"x".repeat(65)] {
        let error = store.namespace(bad).err().expect("refused");
        assert_eq!(error.code(), "invalid-name", "name {bad:?}");
    }
    assert!(store.namespace("ok-name_1").is_ok());
}

#[test]
fn a_bad_key_is_refused_before_any_write() {
    let (store, _dir) = open("keys");
    let ns = store.namespace("prefs").unwrap();
    assert_eq!(ns.put("", Value::Int(1)).err().unwrap().code(), "invalid-key");
    assert_eq!(ns.put("/leading", Value::Int(1)).err().unwrap().code(), "invalid-key");
    assert_eq!(ns.put("nul\0byte", Value::Int(1)).err().unwrap().code(), "invalid-key");
    assert!(ns.list("", None, 10).unwrap().is_empty());
}

#[test]
fn prefix_listing_matches_whole_segments_only() {
    let (store, _dir) = open("prefix");
    let ns = store.namespace("prefs").unwrap();
    for key in ["ui", "ui/theme", "ui/font/size", "ui0", "uiz/x"] {
        ns.put(key, Value::Bool(true)).unwrap();
    }
    assert_eq!(ns.list("ui", None, 10).unwrap(), vec!["ui", "ui/font/size", "ui/theme"]);
    assert_eq!(ns.list("ui/font", None, 10).unwrap(), vec!["ui/font/size"]);
    assert_eq!(ns.list("", None, 10).unwrap().len(), 5);
}

#[test]
fn listing_pages_with_an_after_cursor() {
    let (store, _dir) = open("paging");
    let ns = store.namespace("prefs").unwrap();
    for key in ["a", "b", "c", "d"] {
        ns.put(key, Value::Int(0)).unwrap();
    }
    let first = ns.list("", None, 2).unwrap();
    assert_eq!(first, vec!["a", "b"]);
    assert_eq!(ns.list("", Some("b"), 2).unwrap(), vec!["c", "d"]);
}

fn seeded(label: &str) -> (Store, crate::Namespace, PathBuf) {
    let (store, dir) = open(label);
    let ns = store.namespace("docs").unwrap();
    ns.put("notes/a", Value::Text("The quick brown fox".into())).unwrap();
    ns.put("notes/b", Value::Text("lazy dog".into())).unwrap();
    ns.put("n/1", Value::Double(1.5)).unwrap();
    ns.put("n/2", Value::Int(7)).unwrap();
    ns.put("n/3", Value::Double(9.0)).unwrap();
    ns.put("flag", Value::Bool(true)).unwrap();
    (store, ns, dir)
}

#[test]
fn equality_on_key_and_value_matches_only_the_same_type_and_value() {
    let (_store, ns, _dir) = seeded("eq");
    let by_key = ns.search(&Query::new().key_eq("flag")).unwrap();
    assert_eq!(by_key.len(), 1);
    let by_value = ns.search(&Query::new().value_eq(Value::Int(7))).unwrap();
    assert_eq!(by_value.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec!["n/2"]);
    // An Int 7 is not a Double 7.0: the encoding carries the type.
    assert!(
        ns.search(&Query::new().value_eq(Value::Double(7.0)))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_number_range_is_inclusive_and_skips_text() {
    let (_store, ns, _dir) = seeded("numbers");
    let hits = ns
        .search(&Query::new().number_between(1.5, 7.0).order(Order::KeyAsc))
        .unwrap();
    assert_eq!(
        hits.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["n/1", "n/2"]
    );
    assert_eq!(
        ns.search(&Query::new().number_between(f64::NAN, 1.0))
            .err()
            .unwrap()
            .code(),
        "invalid-query"
    );
}

#[test]
fn a_time_range_is_inclusive_on_both_ends() {
    let (_store, ns, _dir) = seeded("time");
    let all = ns.search(&Query::new()).unwrap();
    let lo = all.iter().map(|e| e.updated_ms).min().unwrap();
    let hi = all.iter().map(|e| e.updated_ms).max().unwrap();
    let hits = ns.search(&Query::new().updated_between(lo, hi)).unwrap();
    assert_eq!(hits.len(), all.len());
    assert!(
        ns.search(&Query::new().updated_between(hi + 1, hi + 2))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn contains_text_ignores_ascii_case_for_long_and_short_phrases() {
    let (_store, ns, _dir) = seeded("contains");
    let long = ns.search(&Query::new().contains("QUICK brown")).unwrap();
    assert_eq!(long.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec!["notes/a"]);
    // Two characters take the scan path; it must agree with the index.
    let short = ns.search(&Query::new().contains("DO")).unwrap();
    assert_eq!(
        short.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["notes/b"]
    );
    assert_eq!(
        ns.search(&Query::new().contains("")).err().unwrap().code(),
        "invalid-query"
    );
}

#[test]
fn an_embedded_quote_in_a_phrase_is_matched_not_parsed() {
    let (store, _dir) = open("quote");
    let ns = store.namespace("docs").unwrap();
    ns.put("q", Value::Text("say \"hi\" now".into())).unwrap();
    assert_eq!(ns.search(&Query::new().contains("\"hi\"")).unwrap().len(), 1);
    assert_eq!(ns.search(&Query::new().contains("\" OR ")).unwrap().len(), 0);
}

#[test]
fn order_limit_offset_and_count_compose() {
    let (_store, ns, _dir) = seeded("order");
    let desc = ns.search(&Query::new().prefix("n").order(Order::NumberDesc)).unwrap();
    assert_eq!(
        desc.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["n/3", "n/2", "n/1"]
    );
    let page = ns
        .search(&Query::new().prefix("n").order(Order::KeyDesc).limit(2).offset(1))
        .unwrap();
    assert_eq!(
        page.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["n/2", "n/1"]
    );
    assert_eq!(ns.count(&Query::new().prefix("n")).unwrap(), 3);
    assert_eq!(ns.count(&Query::new().number_between(0.0, 100.0)).unwrap(), 3);
}

#[test]
fn group_by_count_counts_by_leading_segments() {
    let (_store, ns, _dir) = seeded("group");
    let groups = ns.group_by_count(&Query::new(), 1).unwrap();
    assert!(groups.contains(&("n".to_owned(), 3)));
    assert!(groups.contains(&("notes".to_owned(), 2)));
}

#[test]
fn a_batch_applies_every_op_or_none() {
    let (store, _dir) = open("batch");
    let ns = store.namespace("prefs").unwrap();
    ns.put("keep", Value::Int(1)).unwrap();
    let outcome = ns.batch(&[
        Op::put("a", Value::Int(1)),
        Op::put("b", Value::Int(2)).if_revision(999),
    ]);
    assert_eq!(outcome.err().unwrap().code(), "revision-conflict");
    assert!(
        ns.get("a").unwrap().is_none(),
        "the first op must roll back with the second"
    );

    let ok = ns.batch(&[Op::put("a", Value::Int(1)), Op::delete("keep")]).unwrap();
    assert!(matches!(ok[0], BatchOutcome::Put { .. }));
    assert!(matches!(ok[1], BatchOutcome::Delete { removed: true }));
}

#[test]
fn a_transaction_that_fails_mid_way_leaves_no_partial_state() {
    let (store, _dir) = open("uncommitted");
    let ns = store.namespace("prefs").unwrap();
    // A value over the limit fails inside the transaction after an earlier op was staged.
    let big = Value::Bytes(vec![0; 70 * 1024]);
    assert_eq!(
        ns.batch(&[Op::put("first", Value::Int(1)), Op::put("big", big)])
            .err()
            .unwrap()
            .code(),
        "value-too-large"
    );
    assert!(ns.get("first").unwrap().is_none());
    assert_eq!(ns.count(&Query::new()).unwrap(), 0);
}

#[test]
fn an_oversize_value_is_refused_and_the_limit_is_configurable() {
    let dir = scratch("oversize");
    let store = Store::with_options(
        &dir,
        Options {
            value_limit: 16,
            ..Options::default()
        },
    )
    .unwrap();
    let ns = store.namespace("prefs").unwrap();
    let error = ns.put("k", Value::Bytes(vec![1; 64])).err().unwrap();
    assert_eq!(error.code(), "value-too-large");
    assert!(ns.put("k", Value::Int(1)).is_ok());
}

#[test]
fn the_quota_is_checked_inside_the_transaction_and_refuses_the_write() {
    let dir = scratch("quota");
    let store = Store::with_options(
        &dir,
        Options {
            quota: 2048,
            ..Options::default()
        },
    )
    .unwrap();
    let ns = store.namespace("prefs").unwrap();
    let mut refused = false;
    for i in 0..100 {
        match ns.put(&format!("k/{i}"), Value::Bytes(vec![7; 256])) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.code(), "quota-exceeded");
                refused = true;
                break;
            }
        }
    }
    assert!(refused, "the quota must stop the writes");
    // The refused write left the file usable: a small write still fits or fails cleanly.
    assert!(ns.count(&Query::new()).unwrap() > 0);
}

#[test]
fn a_corrupt_file_is_quarantined_and_the_namespace_starts_empty() {
    let dir = scratch("corrupt");
    std::fs::write(
        dir.join("prefs.sqlite3"),
        b"this is not a database at all, not even close",
    )
    .unwrap();
    let store = Store::open(&dir).unwrap();
    let ns = store.namespace("prefs").unwrap();
    assert_eq!(ns.state(), State::Quarantined);
    assert!(
        ns.get("anything").unwrap().is_none(),
        "reads are empty while quarantined"
    );
    assert_eq!(ns.put("k", Value::Int(1)).err().unwrap().code(), "store-unavailable");

    let moved: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("prefs.corrupt-"))
        .collect();
    assert_eq!(moved.len(), 1, "the damaged file is kept beside the store, not deleted");
}

#[test]
fn a_healthy_file_reopens_with_its_data() {
    let dir = scratch("reopen");
    {
        let store = Store::open(&dir).unwrap();
        store
            .namespace("prefs")
            .unwrap()
            .put("k", Value::Text("v".into()))
            .unwrap();
        store.close();
    }
    let store = Store::open(&dir).unwrap();
    let ns = store.namespace("prefs").unwrap();
    assert_eq!(ns.state(), State::Ready);
    assert_eq!(ns.get("k").unwrap().unwrap().value, Value::Text("v".into()));
}

#[test]
fn a_read_only_namespace_refuses_writes_and_keeps_reads() {
    let dir = scratch("readonly");
    {
        let store = Store::open(&dir).unwrap();
        store.namespace("prefs").unwrap().put("k", Value::Int(1)).unwrap();
        store.close();
    }
    let store = Store::with_options(
        &dir,
        Options {
            writable: false,
            ..Options::default()
        },
    )
    .unwrap();
    let ns = store.namespace("prefs").unwrap();
    assert!(!ns.writable());
    assert_eq!(ns.put("k", Value::Int(2)).err().unwrap().code(), "read-only");
    assert_eq!(ns.get("k").unwrap().unwrap().value, Value::Int(1));
}

#[test]
fn the_lifecycle_table_allows_only_the_listed_moves() {
    assert!(State::Opening.can_move_to(State::Ready));
    assert!(State::Opening.can_move_to(State::Quarantined));
    assert!(State::Ready.can_move_to(State::Closing));
    assert!(State::Quarantined.can_move_to(State::Closing));
    assert!(!State::Ready.can_move_to(State::Opening));
    assert!(!State::Closing.can_move_to(State::Ready));
    assert!(!State::Quarantined.can_move_to(State::Ready));
    assert_eq!(State::Quarantined.as_str(), "quarantined");
}

#[test]
fn watchers_receive_puts_and_deletes_under_their_prefix() {
    let (store, _dir) = open("watch");
    let ns = store.namespace("prefs").unwrap();
    let rx = ns.watch("ui").unwrap();
    ns.put("ui/theme", Value::Text("dark".into())).unwrap();
    ns.put("other", Value::Int(1)).unwrap();
    ns.delete("ui/theme").unwrap();

    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        Change::Put(entry) => assert_eq!(entry.key, "ui/theme"),
        other => panic!("expected a put, got {:?}", other.key()),
    }
    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        Change::Delete { key, .. } => assert_eq!(key, "ui/theme"),
        other => panic!("expected a delete, got {:?}", other.key()),
    }
    assert!(
        rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "the other key is not delivered"
    );
}

#[test]
fn concurrent_readers_see_committed_rows_while_a_writer_runs() {
    let (store, _dir) = open("concurrent");
    let ns = store.namespace("prefs").unwrap();
    for i in 0..20 {
        ns.put(&format!("k/{i:02}"), Value::Int(i)).unwrap();
    }
    std::thread::scope(|scope| {
        let writer = ns.clone();
        scope.spawn(move || {
            for i in 20..60 {
                writer.put(&format!("k/{i:02}"), Value::Int(i)).unwrap();
            }
        });
        for _ in 0..4 {
            let reader = ns.clone();
            scope.spawn(move || {
                for _ in 0..20 {
                    let n = reader.count(&Query::new()).unwrap();
                    assert!((20..=60).contains(&n));
                }
            });
        }
    });
    assert_eq!(ns.count(&Query::new()).unwrap(), 60);
}

/// A namespace file written by plain SQLite 3.50 with the version-1 schema opens unchanged under
/// the SQLCipher build with no key. The bundled-sqlcipher feature must not change the on-disk
/// format of an unkeyed file: a database without `PRAGMA key` is read and written as before.
#[test]
fn a_file_written_by_plain_sqlite_opens_unchanged_under_sqlcipher() {
    let dir = scratch("plain-fixture");
    let fixture = include_bytes!("fixtures/plain-sqlite-3.50.2.sqlite3");
    std::fs::write(dir.join("fixture.sqlite3"), fixture).expect("fixture copied");
    let store = Store::open(&dir).expect("store opens");
    let ns = store.namespace("fixture").expect("namespace opens");
    assert_eq!(ns.state(), State::Ready);
    let entry = ns.get("greeting").unwrap().expect("the fixture row is present");
    assert_eq!(entry.value, Value::Text("hello".into()));
    assert_eq!(entry.revision, 1);
    let rev = ns.put("greeting-again", Value::Text("hi".into())).unwrap();
    assert_eq!(rev, 2, "the revision continues from the fixture's meta row");
    assert_eq!(
        ns.list("", None, 10).unwrap(),
        vec!["greeting".to_owned(), "greeting-again".to_owned()]
    );
}

#[test]
fn a_log_store_returns_the_newest_record_written_under_a_key() {
    let (store, _dir) = open("log-newest");
    let logs = store.log_store("prompt").unwrap();
    logs.put("bingux-prompt", "first").unwrap();
    logs.put("other", "unrelated").unwrap();
    logs.put("bingux-prompt", "second").unwrap();
    let newest = logs.get("bingux-prompt").unwrap().expect("present");
    assert_eq!(newest.message, "second");
    assert_eq!(newest.level, "INFO");
    assert_eq!(newest.component, "bingux-prompt");
    assert!(logs.get("missing").unwrap().is_none());
}

#[test]
fn a_log_store_refuses_a_value_over_the_limit_and_a_bad_key() {
    let (store, _dir) = open("log-limits");
    let logs = store.log_store("prompt").unwrap();
    let big = "x".repeat(DEFAULT_VALUE_LIMIT + 1);
    assert_eq!(logs.put("area", &big).err().unwrap().code(), "value-too-large");
    assert_eq!(logs.put("", "value").err().unwrap().code(), "invalid-key");
    assert_eq!(logs.get("/rooted").err().unwrap().code(), "invalid-key");
}

#[test]
fn a_read_only_log_store_refuses_put_and_keeps_get() {
    let dir = scratch("log-readonly");
    {
        let store = Store::open(&dir).unwrap();
        store.log_store("prompt").unwrap().put("area", "kept").unwrap();
        store.close();
    }
    let store = Store::with_options(
        &dir,
        Options {
            writable: false,
            ..Options::default()
        },
    )
    .unwrap();
    let logs = store.log_store("prompt").unwrap();
    assert_eq!(logs.put("area", "refused").err().unwrap().code(), "read-only");
    assert_eq!(logs.get("area").unwrap().unwrap().message, "kept");
}

#[test]
fn a_log_store_and_a_namespace_of_one_name_keep_separate_files() {
    let (store, dir) = open("log-separate");
    store.namespace("prompt").unwrap().put("k", Value::Int(1)).unwrap();
    store.log_store("prompt").unwrap().put("area", "line").unwrap();
    assert!(dir.join("prompt.sqlite3").exists());
    assert!(dir.join("prompt.log.sqlite3").exists());
    assert_eq!(
        store.namespace("prompt").unwrap().get("k").unwrap().unwrap().value,
        Value::Int(1)
    );
}

#[test]
fn a_log_store_name_is_checked_like_a_namespace_name() {
    let (store, _dir) = open("log-names");
    for bad in ["", "../escape", "a/b", "Upper", "with space"] {
        assert_eq!(store.log_store(bad).err().unwrap().code(), "invalid-name", "{bad:?}");
    }
}
