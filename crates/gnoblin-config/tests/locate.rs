//! Config discovery: one order, one file name, and no dependence on who asked.
//!
//! The configuration file is the shell that discovery selects, so the tiers themselves are tested in
//! `gnoblin-shell-discovery`; what these tests pin is what this crate adds on top: the first-login path
//! and which selected shells are configuration sources at all. The environment is supplied, not read, so a
//! test proves each branch without touching the test runner's environment. The temporary directory is the test
//! kit's, so a failed run leaves nothing behind.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use gnoblin_config::{Environment, Language, LocateError, Tier, locate};
use gnoblin_testkit::RuntimeDir;

/// `<root>/home` is the home; the data directories are inside the root too, so the host's `/usr/share` is not read.
fn home_under(root: &RuntimeDir) -> Environment {
    Environment {
        home: Some(root.join("home")),
        xdg_data_dirs: Some(OsString::from(root.join("share"))),
        ..Environment::default()
    }
}

/// Writes each of `names` in the home fallback's config directory and returns the environment for it.
fn home_with(root: &RuntimeDir, names: &[&str]) -> Environment {
    let directory = config_dir(root);
    fs::create_dir_all(&directory).expect("create the config directory");
    for name in names {
        fs::write(directory.join(name), "// a config\n").expect("write the config");
    }
    home_under(root)
}

fn config_dir(root: &RuntimeDir) -> PathBuf {
    root.join("home").join(".config").join("gnoblin")
}

#[test]
fn the_shell_variable_wins_over_everything_else() {
    let root = RuntimeDir::new("locate-override").expect("temp dir");
    let chosen = root.join("mine.ts");
    fs::write(&chosen, "// a config\n").expect("write the config");
    let environment = Environment {
        home: Some(root.join("home")),
        xdg_config_home: Some(root.join("config")),
        gnoblin_shell: Some(chosen.clone().into_os_string()),
        xdg_data_dirs: Some(OsString::from(root.join("share"))),
        ..Environment::default()
    };
    let found = locate(&environment).expect("the variable is a path");
    assert_eq!(found.path(), chosen.as_path());
    assert_eq!(found.tier(), Tier::Environment);
    assert!(found.exists());
}

#[test]
fn a_shell_variable_naming_a_missing_file_is_an_error_not_a_first_login() {
    let root = RuntimeDir::new("locate-override-missing").expect("temp dir");
    let environment = Environment {
        gnoblin_shell: Some(root.join("mine.ts").into_os_string()),
        ..home_under(&root)
    };
    let error = locate(&environment).expect_err("an explicit request is never guessed");
    assert!(matches!(error, LocateError::Discovery(_)), "{error}");
    assert!(
        error.to_string().contains("no such file (from GNOBLIN_SHELL)"),
        "{error}"
    );
}

#[test]
fn the_xdg_config_home_is_preferred_over_the_home_directory() {
    let root = RuntimeDir::new("locate-xdg").expect("temp dir");
    let environment = Environment {
        xdg_config_home: Some(root.join("config")),
        ..home_under(&root)
    };
    let found = locate(&environment).expect("a path");
    assert_eq!(found.path(), root.join("config").join("gnoblin").join("shell.ts"));
    assert_eq!(found.tier(), Tier::UserConfig);
}

#[test]
fn a_relative_config_home_is_ignored() {
    let root = RuntimeDir::new("locate-relative").expect("temp dir");
    let environment = Environment {
        xdg_config_home: Some(Path::new("relative/config").to_path_buf()),
        ..home_under(&root)
    };
    let found = locate(&environment).expect("the home fallback");
    assert_eq!(found.path(), config_dir(&root).join("shell.ts"));
}

#[test]
fn an_existing_file_is_reported_as_present() {
    let root = RuntimeDir::new("locate-present").expect("temp dir");
    let found = locate(&home_with(&root, &["shell.ts"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("shell.ts"));
    assert!(found.exists());
    assert_eq!(found.language(), Language::TypeScript);
    assert_eq!(found.tier(), Tier::UserConfig);
}

#[test]
fn a_missing_file_is_first_login() {
    let root = RuntimeDir::new("locate-first-login").expect("temp dir");
    let found = locate(&home_under(&root)).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("shell.ts"));
    assert!(!found.exists(), "nothing is installed yet, and that is not an error");
    assert_eq!(found.language(), Language::TypeScript);
}

#[test]
fn a_directory_where_the_file_belongs_is_not_a_config_file() {
    let root = RuntimeDir::new("locate-directory").expect("temp dir");
    fs::create_dir_all(config_dir(&root).join("shell.ts")).expect("a directory named like the file");
    let found = locate(&home_under(&root)).expect("a path");
    assert!(!found.exists());
}

#[test]
fn without_a_home_or_a_config_home_there_is_no_path() {
    let environment = Environment {
        xdg_data_dirs: Some(OsString::from("/nonexistent/share")),
        ..Environment::default()
    };
    assert_eq!(locate(&environment), Err(LocateError::NoHome));
}

#[test]
fn shell_js_is_found_when_there_is_no_shell_ts() {
    let root = RuntimeDir::new("locate-js").expect("temp dir");
    let found = locate(&home_with(&root, &["shell.js"])).expect("a path");
    assert_eq!(found.path(), config_dir(&root).join("shell.js"));
    assert!(found.exists());
    assert_eq!(found.language(), Language::JavaScript);
}

#[test]
fn shell_ts_beside_shell_js_is_an_ambiguity_and_not_a_silent_choice() {
    let root = RuntimeDir::new("locate-ambiguous").expect("temp dir");
    let error = locate(&home_with(&root, &["shell.ts", "shell.js"])).expect_err("two shells are equally preferred");
    assert!(matches!(error, LocateError::Discovery(_)), "{error}");
    assert!(
        error.to_string().starts_with("several shells are equally preferred:"),
        "{error}"
    );
}

#[test]
fn an_old_init_file_is_not_read() {
    let root = RuntimeDir::new("locate-old-name").expect("temp dir");
    let found = locate(&home_with(&root, &["init.ts", "init.js"])).expect("a path");
    assert!(
        !found.exists(),
        "init.ts was renamed shell.ts and nothing reads the old name"
    );
    assert_eq!(found.path(), config_dir(&root).join("shell.ts"));
}

#[test]
fn a_shell_in_another_language_is_not_a_configuration_source() {
    let root = RuntimeDir::new("locate-python").expect("temp dir");
    let python = root.join("mine.py");
    fs::write(&python, "# a shell\n").expect("write the shell");
    let environment = Environment {
        gnoblin_shell: Some(python.clone().into_os_string()),
        ..home_under(&root)
    };
    let error = locate(&environment).expect_err("this runtime reads TypeScript and JavaScript");
    assert_eq!(
        error,
        LocateError::UnsupportedLanguage {
            path: python,
            language: "python".to_owned()
        }
    );
}

#[test]
fn a_typescript_shell_named_by_the_variable_is_accepted_and_names_one_file() {
    let root = RuntimeDir::new("locate-ts-override").expect("temp dir");
    let chosen = root.join("mine.ts");
    fs::write(&chosen, "// a config\n").expect("write the config");
    let environment = Environment {
        gnoblin_shell: Some(chosen.clone().into_os_string()),
        ..home_with(&root, &[])
    };
    let found = locate(&environment).expect("a path");
    assert_eq!(found.path(), chosen.as_path());
    assert!(found.exists());
    assert_eq!(found.language(), Language::TypeScript);
}

#[test]
fn the_config_toml_default_is_how_a_user_picks_an_installed_shell() {
    let root = RuntimeDir::new("locate-config-toml").expect("temp dir");
    let environment = home_with(&root, &["config.toml", "shell.ts"]);
    let shells = root.join("share").join("gnoblin").join("shells");
    fs::create_dir_all(&shells).expect("the shells directory");
    fs::write(shells.join("tiler.js"), "// a shell\n").expect("an installed shell");
    fs::write(config_dir(&root).join("config.toml"), "[shell]\ndefault = \"tiler\"\n").expect("config.toml");
    let found = locate(&environment).expect("a path");
    assert_eq!(found.path(), shells.join("tiler.js"));
    assert_eq!(found.tier(), Tier::Config);
    assert_eq!(found.language(), Language::JavaScript);
}

#[test]
fn a_bad_config_toml_is_an_error_naming_the_file() {
    let root = RuntimeDir::new("locate-bad-config-toml").expect("temp dir");
    let environment = home_with(&root, &["config.toml", "shell.ts"]);
    fs::write(config_dir(&root).join("config.toml"), "[shell]\nisolate = 3\n").expect("config.toml");
    let error = locate(&environment).expect_err("a number is not a boolean");
    assert!(error.to_string().contains("config.toml"), "{error}");
    assert!(
        error.to_string().contains("`[shell] isolate` must be true or false"),
        "{error}"
    );
}

#[test]
fn unknown_config_toml_keys_travel_as_warnings() {
    let root = RuntimeDir::new("locate-warnings").expect("temp dir");
    let environment = home_with(&root, &["config.toml", "shell.ts"]);
    fs::write(config_dir(&root).join("config.toml"), "[shell]\ncolour = 1\n").expect("config.toml");
    let found = locate(&environment).expect("a path");
    assert_eq!(found.warnings().len(), 1);
    assert!(
        found.warnings()[0].ends_with("config.toml: unknown key `shell.colour`"),
        "{:?}",
        found.warnings()
    );
}
