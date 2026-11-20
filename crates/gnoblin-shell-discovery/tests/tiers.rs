//! Each tier of the lookup, the precedence between tiers, and the XDG rules that decide where tiers 3 to 6 look.

mod support;

use std::ffi::OsString;
use std::path::PathBuf;

use gnoblin_shell_discovery::{DiscoveryError, Language, Request, Selection, Source, Tier, discover};
use support::Sandbox;

fn run(sandbox: &Sandbox) -> Result<Selection, DiscoveryError> {
    discover(&sandbox.environment, &Request::Discover)
}

fn chosen(sandbox: &Sandbox) -> Selection {
    run(sandbox).unwrap_or_else(|error| panic!("a shell was expected: {error}"))
}

// Tier 1: the command line.

#[test]
fn a_path_on_the_command_line_is_relative_to_the_working_directory() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("work/mine.py");
    let selection = discover(&sandbox.environment, &Request::Path(PathBuf::from("./mine.py"))).expect("found");
    assert_eq!(selection.path, file);
    assert_eq!(selection.name, "mine");
    assert_eq!(selection.language, Language::Python);
    assert_eq!(selection.chosen_by, Tier::CommandLine);
    assert_eq!(selection.found_in, Tier::CommandLine);
}

#[test]
fn an_absolute_path_on_the_command_line_is_used_as_given() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("elsewhere/shell.so");
    let selection = discover(&sandbox.environment, &Request::Path(file.clone())).expect("found");
    assert_eq!(selection.path, file);
    assert_eq!(selection.language, Language::Native);
}

#[test]
fn the_command_line_beats_every_other_tier() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("work/mine.js");
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.touch("elsewhere/env.py");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"a\"\n");
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.environment.gnoblin_shell = Some(sandbox.at("elsewhere/env.py").into_os_string());
    let selection = discover(&sandbox.environment, &Request::Path(PathBuf::from("mine.js"))).expect("found");
    assert_eq!(selection.name, "mine");
    assert_eq!(selection.chosen_by, Tier::CommandLine);
}

#[test]
fn a_name_on_the_command_line_is_found_in_the_installed_shells() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.touch("home/.local/share/gnoblin/shells/tiler.so");
    sandbox.touch("home/.local/share/gnoblin/shells/lines.py");
    let selection = discover(&sandbox.environment, &Request::Name("lines".to_owned())).expect("found");
    assert_eq!(selection.name, "lines");
    assert_eq!(selection.chosen_by, Tier::CommandLine);
    assert_eq!(selection.found_in, Tier::UserShells);
}

#[test]
fn a_name_is_not_found_among_the_conventional_user_files() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    let error = discover(&sandbox.environment, &Request::Name("shell".to_owned())).expect_err("not installed");
    assert!(matches!(error, DiscoveryError::NameNotFound { .. }), "{error}");
}

#[test]
fn a_name_found_in_an_earlier_data_tier_shadows_a_later_one() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/tiler.py");
    sandbox.touch("usr/share/gnoblin/shells/tiler.so");
    let selection = discover(&sandbox.environment, &Request::Name("tiler".to_owned())).expect("found");
    assert_eq!(selection.found_in, Tier::UserShells);
    assert_eq!(selection.language, Language::Python);
}

#[test]
fn the_packaged_default_is_the_shell_named_default() {
    let sandbox = Sandbox::new();
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    let selection = discover(&sandbox.environment, &Request::Name("default".to_owned())).expect("found");
    assert_eq!(selection.found_in, Tier::Packaged);
    assert_eq!(selection.language, Language::TypeScript);
}

#[test]
fn a_missing_path_on_the_command_line_does_not_fall_through() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    let error = discover(&sandbox.environment, &Request::Path(PathBuf::from("./nope.so"))).expect_err("missing");
    assert!(
        matches!(
            error,
            DiscoveryError::PathMissing {
                source: Source::CommandLine,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_missing_name_on_the_command_line_does_not_fall_through() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.environment.gnoblin_shell = Some(sandbox.touch("elsewhere/env.py").into_os_string());
    let error = discover(&sandbox.environment, &Request::Name("zzz".to_owned())).expect_err("missing");
    assert!(
        matches!(
            error,
            DiscoveryError::NameNotFound {
                source: Source::CommandLine,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn a_name_with_a_slash_is_not_a_name() {
    let sandbox = Sandbox::new();
    for name in ["a/b", "", ".", ".."] {
        let error = discover(&sandbox.environment, &Request::Name(name.to_owned())).expect_err("invalid");
        assert!(matches!(error, DiscoveryError::InvalidName { .. }), "{name}: {error}");
    }
}

// Tier 2: GNOBLIN_SHELL.

#[test]
fn gnoblin_shell_with_a_slash_is_a_path_relative_to_the_working_directory() {
    let mut sandbox = Sandbox::new();
    let file = sandbox.touch("work/sub/mine.ts");
    sandbox.environment.gnoblin_shell = Some(OsString::from("sub/mine.ts"));
    let selection = chosen(&sandbox);
    assert_eq!(selection.path, file);
    assert_eq!(selection.chosen_by, Tier::Environment);
    assert_eq!(selection.language, Language::TypeScript);
}

#[test]
fn gnoblin_shell_without_a_slash_is_a_name() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("usr/share/gnoblin/shells/tiler.so");
    sandbox.environment.gnoblin_shell = Some(OsString::from("tiler"));
    let selection = chosen(&sandbox);
    assert_eq!(selection.name, "tiler");
    assert_eq!(selection.chosen_by, Tier::Environment);
    assert_eq!(selection.found_in, Tier::SystemShells);
}

#[test]
fn an_empty_gnoblin_shell_is_ignored() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    sandbox.environment.gnoblin_shell = Some(OsString::new());
    assert_eq!(chosen(&sandbox).chosen_by, Tier::UserConfig);
}

#[test]
fn an_unresolvable_gnoblin_shell_is_an_error_and_does_not_fall_through() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    sandbox.environment.gnoblin_shell = Some(OsString::from("./missing.so"));
    let error = run(&sandbox).expect_err("missing");
    assert!(
        matches!(
            error,
            DiscoveryError::PathMissing {
                source: Source::Environment,
                ..
            }
        ),
        "{error}"
    );
    sandbox.environment.gnoblin_shell = Some(OsString::from("nobody"));
    let error = run(&sandbox).expect_err("missing");
    assert!(
        matches!(
            error,
            DiscoveryError::NameNotFound {
                source: Source::Environment,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn gnoblin_shell_beats_the_config_file_and_the_directories() {
    let mut sandbox = Sandbox::new();
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"a\"\n");
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.touch("elsewhere/env.py");
    sandbox.environment.gnoblin_shell = Some(sandbox.at("elsewhere/env.py").into_os_string());
    assert_eq!(chosen(&sandbox).chosen_by, Tier::Environment);
}

// Tier 3: config.toml.

#[test]
fn config_default_names_an_installed_shell() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"tiler\"\n");
    sandbox.touch("usr/share/gnoblin/shells/tiler.so");
    sandbox.touch("home/.config/gnoblin/shell.ts");
    let selection = chosen(&sandbox);
    assert_eq!(selection.name, "tiler");
    assert_eq!(selection.chosen_by, Tier::Config);
    assert_eq!(selection.found_in, Tier::SystemShells);
}

#[test]
fn config_path_may_be_absolute() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("opt/tiler/shell.py");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        &format!("[shell]\npath = \"{}\"\n", file.display()),
    );
    let selection = chosen(&sandbox);
    assert_eq!(selection.path, file);
    assert_eq!(selection.chosen_by, Tier::Config);
    assert_eq!(selection.found_in, Tier::Config);
}

#[test]
fn config_path_is_relative_to_the_directory_holding_config_toml() {
    let mut sandbox = Sandbox::new();
    let file = sandbox.touch("home/.config/gnoblin/shells/mine.js");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        "[shell]\npath = \"shells/mine.js\"\n",
    );
    // The working directory is somewhere else entirely.
    sandbox.environment.working_dir = Some(sandbox.at("work"));
    assert_eq!(chosen(&sandbox).path, file);
}

#[test]
fn config_path_expands_a_leading_tilde() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("home/dev/mine.py");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        "[shell]\npath = \"~/dev/mine.py\"\n",
    );
    assert_eq!(chosen(&sandbox).path, file);
}

#[test]
fn config_path_with_a_tilde_and_no_home_is_an_error() {
    let mut sandbox = Sandbox::new();
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        "[shell]\npath = \"~/dev/mine.py\"\n",
    );
    sandbox.environment.xdg_config_home = Some(sandbox.at("home/.config"));
    sandbox.environment.home = None;
    let error = run(&sandbox).expect_err("no home");
    assert!(
        matches!(
            error,
            DiscoveryError::PathUnusable {
                source: Source::Config(_),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn config_default_and_path_together_is_an_error() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/a.so");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        "[shell]\ndefault = \"a\"\npath = \"/x\"\n",
    );
    let error = run(&sandbox).expect_err("both keys");
    assert!(matches!(error, DiscoveryError::Config(_)), "{error}");
    assert!(
        error
            .to_string()
            .ends_with("`[shell]` sets both `default` and `path`; keep one"),
        "{error}"
    );
}

#[test]
fn an_unresolvable_config_default_or_path_is_an_error_and_does_not_fall_through() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"ghost\"\n");
    let error = run(&sandbox).expect_err("no such shell");
    assert!(
        matches!(
            error,
            DiscoveryError::NameNotFound {
                source: Source::Config(_),
                ..
            }
        ),
        "{error}"
    );
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\npath = \"ghost.so\"\n");
    let error = run(&sandbox).expect_err("no such file");
    assert!(
        matches!(
            error,
            DiscoveryError::PathMissing {
                source: Source::Config(_),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn config_language_overrides_detection_for_the_shell_it_selects() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("opt/tiler/main.script");
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        &format!("[shell]\npath = \"{}\"\nlanguage = \"python\"\n", file.display()),
    );
    assert_eq!(chosen(&sandbox).language, Language::Python);
}

#[test]
fn a_config_without_default_or_path_is_skipped_and_still_supplies_isolate() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\nisolate = true\n");
    let selection = chosen(&sandbox);
    assert_eq!(selection.chosen_by, Tier::UserConfig);
    assert_eq!(selection.isolate, Some(true));
}

#[test]
fn isolate_is_read_whichever_tier_chose_the_shell() {
    let sandbox = Sandbox::new();
    sandbox.touch("work/mine.js");
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\nisolate = false\n");
    let selection = discover(&sandbox.environment, &Request::Path(PathBuf::from("mine.js"))).expect("found");
    assert_eq!(selection.isolate, Some(false));
}

#[test]
fn isolate_is_absent_without_a_config() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    assert_eq!(chosen(&sandbox).isolate, None);
}

#[test]
fn unknown_config_keys_are_warnings_not_errors() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    sandbox.write("home/.config/gnoblin/config.toml", "theme = 1\n[shell]\ncolour = 2\n");
    let selection = chosen(&sandbox);
    let mut messages: Vec<&str> = selection
        .warnings
        .iter()
        .map(|warning| warning.message.as_str())
        .collect();
    messages.sort_unstable();
    assert_eq!(messages, vec!["unknown key `shell.colour`", "unknown key `theme`"]);
    assert_eq!(
        selection.warnings[0].source,
        sandbox.at("home/.config/gnoblin/config.toml")
    );
}

#[test]
fn a_broken_config_is_an_error_when_the_lookup_reaches_it() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    let config = sandbox.write("home/.config/gnoblin/config.toml", "[shell\ndefault = 1\n");
    match run(&sandbox).expect_err("not TOML") {
        DiscoveryError::Config(error) => {
            assert_eq!(error.path, config);
            assert!(error.message.starts_with("line "), "{}", error.message);
        }
        other => panic!("expected a config error, got {other}"),
    }
}

#[test]
fn a_broken_config_only_warns_when_a_higher_tier_chose_the_shell() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.js");
    let config = sandbox.write("home/.config/gnoblin/config.toml", "[shell\n");
    sandbox.environment.gnoblin_shell = Some(sandbox.touch("elsewhere/env.py").into_os_string());
    let selection = chosen(&sandbox);
    assert_eq!(selection.chosen_by, Tier::Environment);
    assert_eq!(selection.isolate, None);
    assert_eq!(selection.warnings.len(), 1);
    assert_eq!(selection.warnings[0].source, config);
    assert!(
        selection.warnings[0].message.starts_with("ignored: line "),
        "{}",
        selection.warnings[0]
    );
}

#[test]
fn a_config_of_the_wrong_types_is_an_error() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\nisolate = \"yes\"\n");
    let error = run(&sandbox).expect_err("a string is not a boolean");
    assert!(
        error.to_string().ends_with("`[shell] isolate` must be true or false"),
        "{error}"
    );
}

// Tier 4: the conventional user shells.

#[test]
fn each_conventional_extension_is_found_with_its_language() {
    for (extension, language) in [
        ("so", Language::Native),
        ("py", Language::Python),
        ("js", Language::JavaScript),
        ("mjs", Language::JavaScript),
        ("ts", Language::TypeScript),
        ("mts", Language::TypeScript),
    ] {
        let sandbox = Sandbox::new();
        let file = sandbox.touch(&format!("home/.config/gnoblin/shell.{extension}"));
        let selection = chosen(&sandbox);
        assert_eq!(selection.path, file, "{extension}");
        assert_eq!(selection.language, language, "{extension}");
        assert_eq!(selection.chosen_by, Tier::UserConfig, "{extension}");
        assert_eq!(selection.name, "shell", "{extension}");
    }
}

#[test]
fn two_conventional_files_are_an_ambiguity() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.so");
    sandbox.touch("home/.config/gnoblin/shell.ts");
    match run(&sandbox).expect_err("ambiguous") {
        DiscoveryError::Ambiguous { candidates, trace, .. } => {
            let names: Vec<_> = candidates.iter().filter_map(|candidate| candidate.path()).collect();
            assert_eq!(
                names,
                vec![
                    sandbox.at("home/.config/gnoblin/shell.so"),
                    sandbox.at("home/.config/gnoblin/shell.ts")
                ]
            );
            assert!(trace.is_some());
        }
        other => panic!("expected an ambiguity, got {other}"),
    }
}

#[test]
fn a_conventional_file_beats_the_installed_shells() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.py");
    sandbox.touch("home/.local/share/gnoblin/shells/default.so");
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    assert_eq!(chosen(&sandbox).chosen_by, Tier::UserConfig);
}

#[test]
fn an_old_init_file_is_not_a_conventional_shell() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/init.ts");
    sandbox.touch("home/.config/gnoblin/init.js");
    assert!(matches!(run(&sandbox), Err(DiscoveryError::NoShell { .. })));
}

#[test]
fn a_directory_named_shell_is_not_a_conventional_shell() {
    let sandbox = Sandbox::new();
    sandbox.dir("home/.config/gnoblin/shell.ts");
    assert!(matches!(run(&sandbox), Err(DiscoveryError::NoShell { .. })));
}

// Tier 5: the user's installed shells.

#[test]
fn the_only_user_shell_is_chosen() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("home/.local/share/gnoblin/shells/tiler.so");
    let selection = chosen(&sandbox);
    assert_eq!(selection.path, file);
    assert_eq!(selection.chosen_by, Tier::UserShells);
    assert_eq!(selection.name, "tiler");
}

#[test]
fn a_shell_named_default_wins_inside_one_directory() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/aaa.so");
    sandbox.touch("home/.local/share/gnoblin/shells/default.py");
    sandbox.touch("home/.local/share/gnoblin/shells/zzz.ts");
    assert_eq!(chosen(&sandbox).name, "default");
}

#[test]
fn several_shells_without_a_default_are_an_ambiguity() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/lines.py");
    sandbox.touch("home/.local/share/gnoblin/shells/tiler.so");
    match run(&sandbox).expect_err("ambiguous") {
        DiscoveryError::Ambiguous { candidates, .. } => {
            let names: Vec<&str> = candidates.iter().map(|candidate| candidate.name.as_str()).collect();
            assert_eq!(names, vec!["lines", "tiler"], "candidates are in name order");
        }
        other => panic!("expected an ambiguity, got {other}"),
    }
}

#[test]
fn two_forms_of_one_name_are_an_ambiguity_even_for_default() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/default.so");
    sandbox.touch("home/.local/share/gnoblin/shells/default.py");
    sandbox.touch("home/.local/share/gnoblin/shells/other.js");
    match run(&sandbox).expect_err("ambiguous") {
        DiscoveryError::Ambiguous { candidates, .. } => {
            let names: Vec<&str> = candidates.iter().map(|candidate| candidate.name.as_str()).collect();
            assert_eq!(names, vec!["default", "default"]);
        }
        other => panic!("expected an ambiguity, got {other}"),
    }
}

#[test]
fn a_launcher_and_a_module_of_one_name_are_an_ambiguity() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/foo.ts");
    sandbox.write("home/.local/share/gnoblin/shells/foo.toml", "path = \"/dev/null\"\n");
    assert!(matches!(run(&sandbox), Err(DiscoveryError::Ambiguous { .. })));
}

#[test]
fn a_named_shell_with_two_forms_is_an_ambiguity() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/foo.so");
    sandbox.touch("home/.local/share/gnoblin/shells/foo.py");
    let error = discover(&sandbox.environment, &Request::Name("foo".to_owned())).expect_err("ambiguous");
    assert!(
        matches!(error, DiscoveryError::Ambiguous { trace: None, .. }),
        "{error}"
    );
}

#[test]
fn files_that_are_not_shells_are_not_candidates() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/README.md");
    sandbox.touch("home/.local/share/gnoblin/shells/.hidden.so");
    sandbox.touch("home/.local/share/gnoblin/shells/libtiler.so.1");
    sandbox.touch("home/.local/share/gnoblin/shells/Tiler.PY");
    sandbox.dir("home/.local/share/gnoblin/shells/empty-dir");
    sandbox.touch("home/.local/share/gnoblin/shells/only.so");
    assert_eq!(chosen(&sandbox).name, "only");
}

#[test]
fn user_shells_beat_system_shells() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/mine.so");
    sandbox.touch("usr/share/gnoblin/shells/theirs.so");
    assert_eq!(chosen(&sandbox).name, "mine");
}

// Tier 6: the system directories.

#[test]
fn the_first_data_directory_with_any_shell_decides() {
    let sandbox = Sandbox::new();
    sandbox.touch("usr-local/share/gnoblin/shells/local.so");
    sandbox.touch("usr/share/gnoblin/shells/system.so");
    let selection = chosen(&sandbox);
    assert_eq!(selection.name, "local");
    assert_eq!(selection.chosen_by, Tier::SystemShells);
}

#[test]
fn an_empty_earlier_data_directory_is_skipped() {
    let sandbox = Sandbox::new();
    sandbox.dir("usr-local/share/gnoblin/shells");
    sandbox.touch("usr/share/gnoblin/shells/system.so");
    assert_eq!(chosen(&sandbox).name, "system");
}

#[test]
fn each_data_directory_is_decided_on_its_own() {
    let sandbox = Sandbox::new();
    // `default` is in the later directory only: the earlier directory has several shells and no default, so it is
    // an ambiguity; the later default does not rescue it.
    sandbox.touch("usr-local/share/gnoblin/shells/a.so");
    sandbox.touch("usr-local/share/gnoblin/shells/b.so");
    sandbox.touch("usr/share/gnoblin/shells/default.so");
    assert!(matches!(run(&sandbox), Err(DiscoveryError::Ambiguous { .. })));
}

#[test]
fn xdg_data_dirs_order_is_precedence() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("a/gnoblin/shells/from-a.so");
    sandbox.touch("b/gnoblin/shells/from-b.so");
    sandbox.environment.xdg_data_dirs = Some(OsString::from(format!(
        "{}:{}",
        sandbox.at("b").display(),
        sandbox.at("a").display()
    )));
    assert_eq!(chosen(&sandbox).name, "from-b");
}

#[test]
fn a_relative_xdg_data_dirs_entry_is_ignored() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("work/rel/gnoblin/shells/relative.so");
    sandbox.touch("abs/gnoblin/shells/absolute.so");
    sandbox.environment.xdg_data_dirs = Some(OsString::from(format!("rel:{}", sandbox.at("abs").display())));
    assert_eq!(chosen(&sandbox).name, "absolute");
}

// Tier 7: the packaged default.

#[test]
fn the_packaged_default_runs_on_a_fresh_install() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    let selection = chosen(&sandbox);
    assert_eq!(selection.path, file);
    assert_eq!(selection.name, "default");
    assert_eq!(selection.chosen_by, Tier::Packaged);
    assert_eq!(selection.language, Language::TypeScript);
}

#[test]
fn the_packaged_default_is_the_last_resort() {
    let sandbox = Sandbox::new();
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    sandbox.touch("usr/share/gnoblin/shells/system.py");
    assert_eq!(chosen(&sandbox).chosen_by, Tier::SystemShells);
}

#[test]
fn the_packaged_default_is_not_found_through_xdg_data_dirs() {
    let mut sandbox = Sandbox::new();
    sandbox.touch("usr/share/gnoblin/default-shell/shell.ts");
    sandbox.environment.packaged_default_dir = None;
    assert!(matches!(run(&sandbox), Err(DiscoveryError::NoShell { .. })));
}

#[test]
fn a_user_shell_named_default_shadows_the_packaged_default() {
    let sandbox = Sandbox::new();
    sandbox.touch("prefix/share/gnoblin/default-shell/shell.ts");
    sandbox.touch("home/.local/share/gnoblin/shells/default.py");
    let selection = discover(&sandbox.environment, &Request::Name("default".to_owned())).expect("found");
    assert_eq!(selection.found_in, Tier::UserShells);
}

#[test]
fn nothing_anywhere_is_an_error_and_not_a_guess() {
    let sandbox = Sandbox::new();
    assert!(matches!(run(&sandbox), Err(DiscoveryError::NoShell { .. })));
}

// XDG home handling.

#[test]
fn an_absolute_xdg_config_home_replaces_dot_config() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.xdg_config_home = Some(sandbox.at("cfg"));
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.touch("cfg/gnoblin/shell.py");
    let selection = chosen(&sandbox);
    assert_eq!(selection.path, sandbox.at("cfg/gnoblin/shell.py"));
}

#[test]
fn a_relative_xdg_config_home_is_ignored() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.xdg_config_home = Some(PathBuf::from("relative"));
    sandbox.touch("home/.config/gnoblin/shell.ts");
    sandbox.touch("work/relative/gnoblin/shell.py");
    assert_eq!(chosen(&sandbox).path, sandbox.at("home/.config/gnoblin/shell.ts"));
}

#[test]
fn an_absolute_xdg_data_home_replaces_dot_local_share() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.xdg_data_home = Some(sandbox.at("data"));
    sandbox.touch("home/.local/share/gnoblin/shells/home.so");
    sandbox.touch("data/gnoblin/shells/data.so");
    assert_eq!(chosen(&sandbox).name, "data");
}

#[test]
fn a_relative_xdg_data_home_is_ignored() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.xdg_data_home = Some(PathBuf::from("relative"));
    sandbox.touch("home/.local/share/gnoblin/shells/home.so");
    sandbox.touch("work/relative/gnoblin/shells/rel.so");
    assert_eq!(chosen(&sandbox).name, "home");
}

#[test]
fn without_a_home_the_user_tiers_are_skipped_and_the_system_tiers_still_work() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.home = None;
    sandbox.touch("usr/share/gnoblin/shells/system.so");
    assert_eq!(chosen(&sandbox).name, "system");
}

#[test]
fn a_relative_home_is_not_a_home() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.home = Some(PathBuf::from("home"));
    sandbox.touch("work/home/.config/gnoblin/shell.ts");
    assert!(matches!(run(&sandbox), Err(DiscoveryError::NoShell { .. })));
}
