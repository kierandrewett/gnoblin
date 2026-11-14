//! Config discovery: one order, one file name, and no dependence on who asked.
//!
//! The environment is supplied, not read, so a test proves each branch without
//! touching the test runner's environment. The temporary directory is the test
//! kit's, so a failed run leaves nothing behind.

use std::fs;
use std::path::Path;

use gnoblin_config::{Language, LocateError, Origin, SearchPaths, locate, locate_lua};
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
    assert_eq!(found.path(), root.join("config").join("gnoblin").join("init.ts"));
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
        root.join("home").join(".config").join("gnoblin").join("init.ts")
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
        .join("init.ts");
    fs::create_dir_all(expected.parent().expect("parent")).expect("create the config directory");
    fs::write(&expected, "export default {};\n").expect("write the config");
    let found = locate(&paths).expect("a path");
    assert_eq!(found.path(), expected.as_path());
    assert!(found.exists());
    assert_eq!(found.language(), Language::TypeScript);
    assert!(found.ignored().is_empty());
}

#[test]
fn a_missing_file_is_first_login() {
    let root = RuntimeDir::new("locate-missing").expect("temp dir");
    let found = locate(&home_under(&root)).expect("a path");
    assert!(!found.exists());
    assert_eq!(
        found.path(),
        root.join("home").join(".config").join("gnoblin").join("init.ts"),
        "a new user is told to create init.ts"
    );
    assert_eq!(found.language(), Language::TypeScript);
}

#[test]
fn a_directory_where_the_file_belongs_is_not_a_config_file() {
    let root = RuntimeDir::new("locate-directory").expect("temp dir");
    let paths = home_under(&root);
    let expected = root.join("home").join(".config").join("gnoblin").join("init.ts");
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

/// Writes each of `names` in the home fallback's config directory and returns the lookup for it.
fn home_with(root: &RuntimeDir, names: &[&str]) -> SearchPaths {
    let directory = root.join("home").join(".config").join("gnoblin");
    fs::create_dir_all(&directory).expect("create the config directory");
    for name in names {
        fs::write(directory.join(name), "// a config\n").expect("write the config");
    }
    home_under(root)
}

fn config_dir(root: &RuntimeDir) -> std::path::PathBuf {
    root.join("home").join(".config").join("gnoblin")
}

#[test]
fn init_js_is_found_when_there_is_no_init_ts() {
    let root = RuntimeDir::new("locate-js").expect("temp dir");
    let found = locate(&home_with(&root, &["init.js"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.js"));
    assert!(found.exists());
    assert_eq!(found.language(), Language::JavaScript);
}

#[test]
fn init_lua_is_found_when_it_is_the_only_one() {
    let root = RuntimeDir::new("locate-lua").expect("temp dir");
    let found = locate(&home_with(&root, &["init.lua"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.lua"));
    assert!(found.exists());
    assert_eq!(found.language(), Language::Lua);
    assert!(found.ignored().is_empty());
}

#[test]
fn init_ts_beats_init_js_and_init_lua_and_names_what_it_ignored() {
    let root = RuntimeDir::new("locate-all").expect("temp dir");
    let found = locate(&home_with(&root, &["init.lua", "init.js", "init.ts"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.ts"));
    assert_eq!(found.language(), Language::TypeScript);
    assert_eq!(
        found.ignored(),
        [config_dir(&root).join("init.js"), config_dir(&root).join("init.lua")]
    );
}

#[test]
fn init_js_beats_init_lua_and_names_it_as_ignored() {
    let root = RuntimeDir::new("locate-js-lua").expect("temp dir");
    let found = locate(&home_with(&root, &["init.lua", "init.js"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.js"));
    assert_eq!(found.ignored(), [config_dir(&root).join("init.lua")]);
}

#[test]
fn init_ts_beats_init_lua_and_names_it_as_ignored() {
    let root = RuntimeDir::new("locate-ts-lua").expect("temp dir");
    let found = locate(&home_with(&root, &["init.ts", "init.lua"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.ts"));
    assert_eq!(found.ignored(), [config_dir(&root).join("init.lua")]);
}

#[test]
fn the_xdg_config_home_directory_is_searched_for_every_name() {
    let root = RuntimeDir::new("locate-xdg-js").expect("temp dir");
    let directory = root.join("config").join("gnoblin");
    fs::create_dir_all(&directory).expect("create the config directory");
    fs::write(directory.join("init.js"), "// a config\n").expect("write the config");
    let home = home_with(&root, &["init.ts"]);
    let paths = SearchPaths {
        config_home: Some(root.join("config")),
        ..home
    };
    let found = locate(&paths).expect("a path");
    assert_eq!(
        found.path(),
        directory.join("init.js"),
        "the config home is the directory searched"
    );
    assert_eq!(found.origin(), Origin::ConfigHome);
}

#[test]
fn the_lua_lookup_reads_init_lua_even_when_init_ts_is_there() {
    let root = RuntimeDir::new("locate-lua-only").expect("temp dir");
    let paths = home_with(&root, &["init.ts", "init.lua"]);
    let found = locate_lua(&paths).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.lua"));
    assert!(found.exists());
    assert_eq!(found.language(), Language::Lua);
    assert!(
        found.ignored().is_empty(),
        "the Lua shell does not report files it cannot read"
    );
}

#[test]
fn the_lua_lookup_names_init_lua_when_it_is_missing() {
    let root = RuntimeDir::new("locate-lua-missing").expect("temp dir");
    let found = locate_lua(&home_under(&root)).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("init.lua"));
    assert!(!found.exists());
}

#[test]
fn the_lua_lookup_refuses_a_typescript_override() {
    let root = RuntimeDir::new("locate-lua-ts").expect("temp dir");
    let paths = SearchPaths {
        override_path: Some(root.join("mine.ts")),
        ..home_under(&root)
    };
    assert_eq!(
        locate_lua(&paths),
        Err(LocateError::NotLua {
            path: root.join("mine.ts")
        })
    );
}

#[test]
fn a_typescript_override_is_accepted_and_names_one_file() {
    let root = RuntimeDir::new("locate-ts-override").expect("temp dir");
    let chosen = root.join("mine.ts");
    fs::write(&chosen, "// a config\n").expect("write the config");
    let paths = SearchPaths {
        override_path: Some(chosen.clone()),
        ..home_with(&root, &["init.lua"])
    };
    let found = locate(&paths).expect("an override");
    assert_eq!(found.path(), chosen.as_path());
    assert!(found.exists());
    assert_eq!(found.language(), Language::TypeScript);
    assert!(
        found.ignored().is_empty(),
        "an override names one file and nothing is ignored"
    );
}
