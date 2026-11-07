//! Tests of the keyring file against real files in a scratch directory. Each test gets its own
//! directory so the files of one test never leak into another.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{CollectionId, ItemId, Keyring, KeyringError, Passphrase};

/// A fresh directory under the system temp dir, unique to this process and call.
fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "gnoblin-keyring-test-{}-{label}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn pass(text: &str) -> Passphrase {
    Passphrase::new(text.to_owned()).expect("non-empty passphrase")
}

/// A path for a keyring file in a fresh directory, not yet created.
fn fresh_path(label: &str) -> PathBuf {
    scratch(label).join("keyring.db")
}

/// The error from an open that must fail. `Keyring` is not `Debug`, so `expect_err` is unavailable.
fn error_of(result: Result<Keyring, KeyringError>) -> KeyringError {
    match result {
        Ok(_) => panic!("the operation unexpectedly succeeded"),
        Err(error) => error,
    }
}

#[test]
fn an_empty_passphrase_is_refused() {
    assert!(matches!(
        Passphrase::new(String::new()),
        Err(KeyringError::EmptyPassphrase)
    ));
}

#[test]
fn an_item_round_trips_through_reopen_with_the_right_key() {
    let path = fresh_path("round-trip");
    let mut keyring = Keyring::create(&path, &pass("correct horse")).expect("creates");
    let login = keyring.create_collection("Login").expect("collection");
    let id = keyring
        .put_item(
            login,
            "wifi",
            "text/plain",
            b"hunter2",
            &[("service", "wifi"), ("user", "kieran")],
        )
        .expect("stores");
    drop(keyring);

    let reopened = Keyring::open(&path, &pass("correct horse")).expect("opens with the key");
    let item = reopened.item(id).expect("reads").expect("present");
    assert_eq!(item.label, "wifi");
    assert_eq!(item.collection, login);
    assert_eq!(item.content_type, "text/plain");
    assert_eq!(item.secret.as_slice(), b"hunter2");
    assert_eq!(
        item.attributes,
        vec![
            ("service".to_owned(), "wifi".to_owned()),
            ("user".to_owned(), "kieran".to_owned()),
        ]
    );
}

#[test]
fn a_wrong_key_is_reported_as_wrong_key() {
    let path = fresh_path("wrong-key");
    drop(Keyring::create(&path, &pass("right")).expect("creates"));
    assert!(matches!(
        error_of(Keyring::open(&path, &pass("wrong"))),
        KeyringError::WrongKey
    ));
}

#[test]
fn a_file_that_is_not_a_keyring_is_not_opened_with_any_key() {
    let path = fresh_path("not-a-keyring");
    std::fs::write(
        &path,
        b"plain text, not a database at all, padded out to some length........",
    )
    .expect("writes");
    assert!(matches!(
        error_of(Keyring::open(&path, &pass("anything"))),
        KeyringError::WrongKey
    ));
}

#[test]
fn create_refuses_to_replace_an_existing_file() {
    let path = fresh_path("exists");
    drop(Keyring::create(&path, &pass("one")).expect("creates"));
    assert!(matches!(
        error_of(Keyring::create(&path, &pass("two"))),
        KeyringError::Exists(_)
    ));
    // The first keyring is untouched and still opens with its own key.
    assert!(Keyring::open(&path, &pass("one")).is_ok());
}

#[test]
fn open_refuses_a_missing_file_without_creating_it() {
    let path = fresh_path("missing");
    assert!(matches!(
        error_of(Keyring::open(&path, &pass("any"))),
        KeyringError::Missing(_)
    ));
    assert!(!path.exists());
}

#[test]
fn rekey_makes_the_old_key_fail_and_the_new_key_open() {
    let path = fresh_path("rekey");
    let mut keyring = Keyring::create(&path, &pass("old")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let id = keyring
        .put_item(collection, "kept", "text/plain", b"secret", &[])
        .expect("stores");
    keyring.rekey(&pass("new")).expect("rekeys");
    drop(keyring);

    assert!(matches!(
        error_of(Keyring::open(&path, &pass("old"))),
        KeyringError::WrongKey
    ));
    let reopened = Keyring::open(&path, &pass("new")).expect("opens with the new key");
    assert_eq!(
        reopened.item(id).expect("reads").expect("present").secret.as_slice(),
        b"secret"
    );
}

#[test]
fn a_truncated_file_is_refused_rather_than_read_as_empty() {
    let path = fresh_path("truncated");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    for n in 0..64 {
        keyring
            .put_item(
                collection,
                &format!("item {n}"),
                "text/plain",
                &[7u8; 512],
                &[("n", n.to_string().as_str())],
            )
            .expect("stores");
    }
    drop(keyring);

    let len = std::fs::metadata(&path).expect("metadata").len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_len(len / 2))
        .expect("truncates");

    let error = error_of(Keyring::open(&path, &pass("key")));
    assert!(
        matches!(error, KeyringError::WrongKey | KeyringError::Corrupt(_)),
        "a truncated file must be refused, got {error}"
    );
}

#[test]
fn a_damaged_later_page_is_reported_as_corruption() {
    let path = fresh_path("damaged-page");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    for n in 0..64 {
        keyring
            .put_item(collection, &format!("item {n}"), "text/plain", &[9u8; 512], &[])
            .expect("stores");
    }
    drop(keyring);

    // Page 1 holds the key-check header; damage on page 2 must not look like a wrong key.
    overwrite(&path, 4096 + 100, &[0xA5; 64]);

    let error = error_of(Keyring::open(&path, &pass("key")));
    assert!(
        matches!(error, KeyringError::Corrupt(_)),
        "a damaged page must be corruption, got {error}"
    );
}

#[test]
fn two_handles_on_one_file_see_each_others_writes() {
    let path = fresh_path("two-handles");
    let mut writer = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = writer.create_collection("Login").expect("collection");
    let reader = Keyring::open(&path, &pass("key")).expect("second handle opens");
    let id = writer
        .put_item(collection, "seen", "text/plain", b"x", &[])
        .expect("stores");
    assert!(reader.item(id).expect("reads").is_some());
}

#[test]
fn search_matches_every_attribute_and_nothing_else() {
    let path = fresh_path("search");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let wifi = keyring
        .put_item(
            collection,
            "a",
            "text/plain",
            b"1",
            &[("service", "wifi"), ("user", "kieran")],
        )
        .expect("stores");
    let mail = keyring
        .put_item(
            collection,
            "b",
            "text/plain",
            b"2",
            &[("service", "mail"), ("user", "kieran")],
        )
        .expect("stores");
    assert_eq!(
        keyring.search_items(&[("user", "kieran")]).expect("searches"),
        vec![wifi, mail]
    );
    assert_eq!(
        keyring
            .search_items(&[("service", "mail"), ("user", "kieran")])
            .expect("searches"),
        vec![mail]
    );
    assert!(
        keyring
            .search_items(&[("service", "wifi"), ("user", "bob")])
            .expect("searches")
            .is_empty()
    );
    assert_eq!(keyring.search_items(&[]).expect("searches").len(), 2);
}

#[test]
fn a_deleted_item_is_gone_and_its_attributes_go_with_it() {
    let path = fresh_path("delete");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let id = keyring
        .put_item(collection, "gone", "text/plain", b"s", &[("service", "x")])
        .expect("stores");
    assert!(keyring.delete_item(id).expect("deletes"));
    assert!(keyring.item(id).expect("reads").is_none());
    assert!(keyring.search_items(&[("service", "x")]).expect("searches").is_empty());
    assert!(!keyring.delete_item(id).expect("deletes again"));
}

#[test]
fn an_alias_points_at_its_collection_and_can_be_moved() {
    let path = fresh_path("alias");
    let keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let first = keyring.create_collection("First").expect("collection");
    let second = keyring.create_collection("Second").expect("collection");
    assert_eq!(keyring.alias("default").expect("reads"), None);
    keyring.set_alias("default", first).expect("sets");
    assert_eq!(keyring.alias("default").expect("reads"), Some(first));
    keyring.set_alias("default", second).expect("moves");
    assert_eq!(keyring.alias("default").expect("reads"), Some(second));
}

/// Overwrites `bytes` at `offset` in the file at `path`, for simulating on-disk damage.
fn overwrite(path: &Path, offset: u64, bytes: &[u8]) {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("opens for damage");
    file.seek(SeekFrom::Start(offset)).expect("seeks");
    file.write_all(bytes).expect("damages");
}

#[test]
fn the_content_type_round_trips_through_reopen() {
    let path = fresh_path("content-type");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let id = keyring
        .put_item(collection, "cert", "application/x-pem-file", b"-----", &[])
        .expect("stores");
    drop(keyring);

    let reopened = Keyring::open(&path, &pass("key")).expect("opens");
    let item = reopened.item(id).expect("reads").expect("present");
    assert_eq!(item.content_type, "application/x-pem-file");
}

#[test]
fn a_version_one_file_opens_and_its_items_read_as_text_plain() {
    let path = fresh_path("upgrade");
    // Write the schema that shipped as version 1 by hand, so the upgrade is what runs on open.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("creates the file");
    let conn = crate::connect(&path, &pass("key"), rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).expect("connects");
    conn.execute_batch(crate::schema::V1).expect("writes version 1");
    conn.pragma_update(None, "user_version", 1).expect("stamps version 1");
    conn.execute("INSERT INTO collections (label) VALUES ('Login')", [])
        .expect("collection");
    conn.execute(
        "INSERT INTO items (collection_id, label, secret, created_ms, modified_ms)
         VALUES (1, 'old', x'0102', 0, 0)",
        [],
    )
    .expect("version 1 item");
    drop(conn);

    let keyring = Keyring::open(&path, &pass("key")).expect("a version 1 file opens and upgrades");
    let item = keyring.item(ItemId::from_raw(1)).expect("reads").expect("present");
    assert_eq!(item.content_type, "text/plain");
    assert_eq!(item.secret.as_slice(), &[1, 2]);
    drop(keyring);

    // The upgrade is stamped, so the next open takes the current-version path.
    let keyring = Keyring::open(&path, &pass("key")).expect("reopens after the upgrade");
    assert_eq!(
        keyring
            .item(ItemId::from_raw(1))
            .expect("reads")
            .expect("present")
            .content_type,
        "text/plain"
    );
}

#[test]
fn an_update_keeps_the_id_and_replaces_the_secret_label_and_attributes() {
    let path = fresh_path("update");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let id = keyring
        .put_item(collection, "old", "text/plain", b"old secret", &[("service", "old")])
        .expect("stores");
    let updated = keyring
        .update_item(
            id,
            "new",
            "application/octet-stream",
            b"new secret",
            &[("service", "new")],
        )
        .expect("updates");
    assert!(updated);
    let item = keyring.item(id).expect("reads").expect("present");
    assert_eq!(item.label, "new");
    assert_eq!(item.content_type, "application/octet-stream");
    assert_eq!(item.secret.as_slice(), b"new secret");
    assert_eq!(item.attributes, vec![("service".to_owned(), "new".to_owned())]);
    assert!(
        keyring
            .search_items(&[("service", "old")])
            .expect("searches")
            .is_empty()
    );
    assert_eq!(keyring.search_items(&[("service", "new")]).expect("searches"), vec![id]);
}

#[test]
fn an_update_of_a_missing_item_changes_nothing() {
    let path = fresh_path("update-missing");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let kept = keyring
        .put_item(collection, "kept", "text/plain", b"s", &[("service", "x")])
        .expect("stores");
    let missing = ItemId::from_raw(kept.raw() + 100);
    assert!(
        !keyring
            .update_item(missing, "ghost", "text/plain", b"ghost", &[("service", "x")])
            .expect("updates")
    );
    assert!(keyring.item(missing).expect("reads").is_none());
    assert_eq!(keyring.search_items(&[("service", "x")]).expect("searches"), vec![kept]);
}

#[test]
fn deleting_a_collection_removes_its_items_and_the_alias_that_named_it() {
    let path = fresh_path("delete-collection");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let doomed = keyring.create_collection("Doomed").expect("collection");
    let survivor = keyring.create_collection("Survivor").expect("collection");
    let gone = keyring
        .put_item(doomed, "gone", "text/plain", b"s", &[("service", "x")])
        .expect("stores");
    let kept = keyring
        .put_item(survivor, "kept", "text/plain", b"s", &[("service", "y")])
        .expect("stores");
    keyring.set_alias("default", doomed).expect("sets");
    assert!(keyring.delete_collection(doomed).expect("deletes"));
    assert!(keyring.item(gone).expect("reads").is_none());
    assert!(keyring.search_items(&[("service", "x")]).expect("searches").is_empty());
    assert_eq!(keyring.alias("default").expect("reads"), None);
    assert_eq!(keyring.collections().expect("lists").len(), 1);
    assert!(keyring.item(kept).expect("reads").is_some());
}

#[test]
fn deleting_a_missing_collection_removes_nothing() {
    let path = fresh_path("delete-collection-missing");
    let mut keyring = Keyring::create(&path, &pass("key")).expect("creates");
    let collection = keyring.create_collection("Login").expect("collection");
    let missing = CollectionId::from_raw(collection.raw() + 100);
    assert!(!keyring.delete_collection(missing).expect("deletes"));
    assert_eq!(keyring.collections().expect("lists").len(), 1);
}
