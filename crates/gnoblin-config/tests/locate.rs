//! Config discovery: one order, one file name, and no dependence on who asked.
//!
//! The environment is supplied, not read, so a test proves each branch without
//! touching the test runner's environment. The temporary directory is the test
//! kit's, so a failed run leaves nothing behind.

use std::fs;
use std::path::Path;

use gnoblin_config::{LocateError, Origin, SearchPaths, locate};
use gnoblin_testkit::RuntimeDir;

fn home_under(root: &RuntimeDir) -> SearchPaths {
    SearchPaths {
        home: Some(root.join("home")),
        ..SearchPaths::default()
    }
}

#[test]
fn the_override_wins_over_everything_else() {
    let root = RuntimeDir::new("locate-override").expect("temp dir");
    let chosen = root.join("mine.lua");
    let paths = SearchPaths {
        home: Some(root.join("home")),
        config_home: Some(root.join("config")),
        override_path: Some(chosen.clone()),
    };
    let found = locate(&paths).expect("the override is a path");
    assert_eq!(found.path(), chosen.as_path());
    assert_eq!(found.origin(), Origin::Override);
    assert!(!found.exists(), "the file is not there yet, and that is not an error");
}

#[test]
fn the_xdg_config_home_is_preferred_over_the_home_directory() {
    let root = RuntimeDir::new("locate-xdg").expect("temp dir");
    let paths = SearchPaths {
        home: Some(root.join("home")),
        config_home: Some(root.join("config")),
        override_path: None,
    };
    let found = locate(&paths).expect("a path");
    assert_eq!(found.path(), root.join("config").join("gnoblin").join("init.lua"));
    assert_eq!(found.origin(), Origin::ConfigHome);
}

#[test]
fn a_relative_config_home_is_ignored() {
    let root = RuntimeDir::new("locate-relative").expect("temp dir");
    let paths = SearchPaths {
        config_home: Some(Path::new("relative/config").to_path_buf()),
        ..home_under(&root)
    };
    let found = locate(&paths).expect("the home fallback");
    assert_eq!(
        found.path(),
        root.join("home").join(".config").join("gnoblin").join("init.lua")
    );
    assert_eq!(found.origin(), Origin::Home);
}

#[test]
fn an_existing_file_is_reported_as_present() {
    let root = RuntimeDir::new("locate-present").expect("temp dir");
    let paths = home_under(&root);
    let expected = paths
        .home
        .as_ref()
        .expect("home")
        .join(".config")
        .join("gnoblin")
        .join("init.lua");
    fs::create_dir_all(expected.parent().expect("parent")).expect("create the config directory");
    fs::write(&expected, "gnoblin.configure {}\n").expect("write the config");
    let found = locate(&paths).expect("a path");
    assert_eq!(found.path(), expected.as_path());
    assert!(found.exists());
}

#[test]
fn a_missing_file_is_first_login() {
    let root = RuntimeDir::new("locate-missing").expect("temp dir");
    let found = locate(&home_under(&root)).expect("a path");
    assert!(!found.exists());
}

#[test]
fn a_directory_where_the_file_belongs_is_not_a_config_file() {
    let root = RuntimeDir::new("locate-directory").expect("temp dir");
    let paths = home_under(&root);
    let expected = root.join("home").join(".config").join("gnoblin").join("init.lua");
    fs::create_dir_all(&expected).expect("create a directory with the config's name");
    let found = locate(&paths).expect("a path");
    assert!(!found.exists(), "a directory is not a configuration file");
}

#[test]
fn a_toml_override_is_refused_with_a_sentence() {
    let root = RuntimeDir::new("locate-toml").expect("temp dir");
    let paths = SearchPaths {
        override_path: Some(root.join("gnoblin.toml")),
        ..home_under(&root)
    };
    match locate(&paths) {
        Err(LocateError::UnsupportedExtension { path }) => {
            assert_eq!(path, root.join("gnoblin.toml"));
        }
        other => panic!("expected an unsupported extension, got {other:?}"),
    }
}

#[test]
fn without_a_home_or_a_config_home_there_is_no_path() {
    assert_eq!(locate(&SearchPaths::default()), Err(LocateError::NoHome));
}
