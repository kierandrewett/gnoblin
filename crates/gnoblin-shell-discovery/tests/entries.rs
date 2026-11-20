//! The forms an installed shell takes: module files, directories, symlinks and launcher files; and the paths a person
//! can give directly.

mod support;

use std::path::PathBuf;

use gnoblin_shell_discovery::{DiscoveryError, Form, Language, Request, Selection, Source, Tier, discover};
use support::Sandbox;

fn by_name(sandbox: &Sandbox, name: &str) -> Result<Selection, DiscoveryError> {
    discover(&sandbox.environment, &Request::Name(name.to_owned()))
}

fn by_path(sandbox: &Sandbox, path: impl Into<PathBuf>) -> Result<Selection, DiscoveryError> {
    discover(&sandbox.environment, &Request::Path(path.into()))
}

// Directories.

#[test]
fn a_directory_with_a_shell_file_is_a_shell_named_for_the_directory() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("home/.local/share/gnoblin/shells/tiler/shell.py");
    sandbox.touch("home/.local/share/gnoblin/shells/tiler/helper.py");
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.path, file);
    assert_eq!(selection.location, sandbox.at("home/.local/share/gnoblin/shells/tiler"));
    assert_eq!(selection.language, Language::Python);
}

#[test]
fn a_directory_with_two_shell_files_is_an_ambiguity() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/tiler/shell.py");
    sandbox.touch("home/.local/share/gnoblin/shells/tiler/shell.ts");
    assert!(matches!(
        by_name(&sandbox, "tiler"),
        Err(DiscoveryError::Ambiguous { .. })
    ));
}

#[test]
fn a_directory_path_on_the_command_line_runs_its_shell_file() {
    let sandbox = Sandbox::new();
    let file = sandbox.touch("work/mine/shell.ts");
    let selection = by_path(&sandbox, "mine").expect("found");
    assert_eq!(selection.path, file);
    assert_eq!(selection.name, "mine");
}

#[test]
fn a_directory_path_with_no_shell_file_is_unusable() {
    let sandbox = Sandbox::new();
    sandbox.touch("work/mine/other.ts");
    match by_path(&sandbox, "mine").expect_err("no shell file") {
        DiscoveryError::PathUnusable { reason, .. } => {
            assert_eq!(reason, "is a directory with no shell.{so,py,js,mjs,ts,mts} in it");
        }
        other => panic!("expected an unusable path, got {other}"),
    }
}

// Symlinks.

#[test]
fn a_symlink_is_named_for_the_link_not_the_target() {
    let sandbox = Sandbox::new();
    let target = sandbox.touch("dev/tiler/libtiler_impl.so");
    sandbox.link("home/.local/share/gnoblin/shells/tiler.so", &target);
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.name, "tiler");
    assert_eq!(selection.path, sandbox.at("home/.local/share/gnoblin/shells/tiler.so"));
    assert_eq!(selection.language, Language::Native);
}

#[test]
fn a_symlink_to_a_directory_is_a_directory_shell() {
    let sandbox = Sandbox::new();
    let directory = sandbox.dir("dev/mine");
    sandbox.touch("dev/mine/shell.js");
    sandbox.link("home/.local/share/gnoblin/shells/renamed", &directory);
    let selection = by_name(&sandbox, "renamed").expect("found");
    assert_eq!(
        selection.path,
        sandbox.at("home/.local/share/gnoblin/shells/renamed/shell.js")
    );
    assert_eq!(selection.language, Language::JavaScript);
}

#[test]
fn a_dangling_symlink_is_a_candidate_that_cannot_be_used() {
    let sandbox = Sandbox::new();
    sandbox.link("home/.local/share/gnoblin/shells/gone.so", &sandbox.at("not/there.so"));
    match by_name(&sandbox, "gone").expect_err("dangling") {
        DiscoveryError::Unusable { name, problem, .. } => {
            assert_eq!(name, "gone");
            assert_eq!(
                problem,
                format!(
                    "the symlink points to {}, which does not exist",
                    sandbox.at("not/there.so").display()
                )
            );
        }
        other => panic!("expected an unusable shell, got {other}"),
    }
}

#[test]
fn a_dangling_symlink_with_another_name_is_not_a_candidate() {
    let sandbox = Sandbox::new();
    sandbox.link("home/.local/share/gnoblin/shells/notes", &sandbox.at("not/there"));
    sandbox.touch("home/.local/share/gnoblin/shells/real.so");
    assert_eq!(
        discover(&sandbox.environment, &Request::Discover).expect("found").name,
        "real"
    );
}

#[test]
fn a_symlink_on_the_command_line_is_followed() {
    let sandbox = Sandbox::new();
    let target = sandbox.touch("dev/real.py");
    let link = sandbox.link("work/alias.py", &target);
    let selection = by_path(&sandbox, link.clone()).expect("found");
    assert_eq!(selection.path, link);
    assert_eq!(selection.name, "alias");
}

// Launcher files.

fn install_launcher(sandbox: &Sandbox, name: &str, text: &str) -> PathBuf {
    sandbox.write(&format!("usr/share/gnoblin/shells/{name}.toml"), text)
}

#[test]
fn a_launcher_module_is_relative_to_the_library_directory() {
    let sandbox = Sandbox::new();
    let module = sandbox.touch("lib/gnoblin/shells/tiler/libtiler.so");
    let launcher = install_launcher(
        &sandbox,
        "tiler",
        "module = \"tiler/libtiler.so\"\ndescription = \"A tiling shell\"\n",
    );
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.name, "tiler");
    assert_eq!(selection.path, module);
    assert_eq!(selection.launcher, Some(launcher.clone()));
    assert_eq!(selection.location, launcher);
    assert_eq!(selection.language, Language::Native);
    assert_eq!(selection.description.as_deref(), Some("A tiling shell"));
    assert_eq!(selection.found_in, Tier::SystemShells);
}

#[test]
fn a_launcher_path_may_be_absolute() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/shell.py");
    install_launcher(&sandbox, "tiler", &format!("path = \"{}\"\n", script.display()));
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.path, script);
    assert_eq!(selection.language, Language::Python);
}

#[test]
fn a_launcher_path_is_relative_to_the_launcher_file() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("usr/share/gnoblin/shells/tiler-files/main.ts");
    install_launcher(&sandbox, "tiler", "path = \"tiler-files/main.ts\"\n");
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.path, script);
    assert_eq!(selection.language, Language::TypeScript);
}

#[test]
fn a_launcher_path_expands_a_leading_tilde() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("home/dev/shell.js");
    install_launcher(&sandbox, "mine", "path = \"~/dev/shell.js\"\n");
    assert_eq!(by_name(&sandbox, "mine").expect("found").path, script);
}

#[test]
fn a_launcher_reached_through_a_symlink_is_relative_to_the_real_file() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("dev/tiler/shell.py");
    let launcher = sandbox.write("dev/tiler/tiler.toml", "path = \"shell.py\"\n");
    sandbox.link("home/.local/share/gnoblin/shells/tiler.toml", &launcher);
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.path, script);
    assert_eq!(
        selection.launcher,
        Some(sandbox.at("home/.local/share/gnoblin/shells/tiler.toml"))
    );
}

#[test]
fn a_launcher_language_overrides_detection() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/entry");
    install_launcher(
        &sandbox,
        "tiler",
        &format!("path = \"{}\"\nlanguage = \"python\"\n", script.display()),
    );
    assert_eq!(by_name(&sandbox, "tiler").expect("found").language, Language::Python);
}

#[test]
fn a_launcher_for_another_language_is_passed_through_for_the_loader() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/shell.rb");
    install_launcher(&sandbox, "tiler", &format!("path = \"{}\"\n", script.display()));
    assert_eq!(
        by_name(&sandbox, "tiler").expect("found").language,
        Language::Other("rb".to_owned())
    );
}

#[test]
fn a_launcher_unknown_key_is_a_warning_naming_the_launcher() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/shell.py");
    let launcher = install_launcher(
        &sandbox,
        "tiler",
        &format!("path = \"{}\"\nauthor = \"me\"\n", script.display()),
    );
    let selection = by_name(&sandbox, "tiler").expect("found");
    assert_eq!(selection.warnings.len(), 1);
    assert_eq!(selection.warnings[0].source, launcher);
    assert_eq!(selection.warnings[0].message, "unknown key `author`");
}

fn unusable(sandbox: &Sandbox, name: &str) -> String {
    match by_name(sandbox, name).expect_err("the launcher cannot be used") {
        DiscoveryError::Unusable { problem, .. } => problem,
        other => panic!("expected an unusable shell, got {other}"),
    }
}

#[test]
fn a_launcher_with_a_missing_target_is_an_error_that_names_it() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "tiler", "path = \"/opt/gone/shell.py\"\n");
    assert_eq!(
        unusable(&sandbox, "tiler"),
        "its target /opt/gone/shell.py does not exist"
    );
}

#[test]
fn a_launcher_with_a_missing_module_names_the_resolved_path() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "tiler", "module = \"tiler/libtiler.so\"\n");
    assert_eq!(
        unusable(&sandbox, "tiler"),
        format!(
            "its target {} does not exist",
            sandbox.at("lib/gnoblin/shells/tiler/libtiler.so").display()
        )
    );
}

#[test]
fn a_launcher_with_a_directory_target_is_unusable() {
    let sandbox = Sandbox::new();
    let directory = sandbox.dir("opt/tiler");
    install_launcher(&sandbox, "tiler", &format!("path = \"{}\"\n", directory.display()));
    assert_eq!(
        unusable(&sandbox, "tiler"),
        format!("its target {} is a directory; name the file", directory.display())
    );
}

#[test]
fn a_launcher_pointing_at_a_launcher_is_unusable() {
    let sandbox = Sandbox::new();
    let other = sandbox.write("opt/other.toml", "path = \"/dev/null\"\n");
    install_launcher(&sandbox, "tiler", &format!("path = \"{}\"\n", other.display()));
    assert_eq!(
        unusable(&sandbox, "tiler"),
        format!("its target {} is another launcher file", other.display())
    );
}

#[test]
fn a_launcher_needs_exactly_one_of_module_and_path() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "none", "description = \"nothing to run\"\n");
    install_launcher(&sandbox, "both", "module = \"a.so\"\npath = \"/b.py\"\n");
    assert_eq!(unusable(&sandbox, "none"), "it has neither `module` nor `path`");
    assert_eq!(unusable(&sandbox, "both"), "it has both `module` and `path`; keep one");
}

#[test]
fn a_launcher_that_is_not_toml_is_unusable() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "bad", "path = \n");
    assert!(unusable(&sandbox, "bad").starts_with("line "));
}

#[test]
fn a_launcher_key_of_the_wrong_type_is_unusable() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "bad", "path = 3\n");
    assert_eq!(unusable(&sandbox, "bad"), "`path` must be a string");
}

#[test]
fn a_launcher_module_without_a_library_directory_says_so() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.module_dir = None;
    install_launcher(&sandbox, "tiler", "module = \"tiler/libtiler.so\"\n");
    assert_eq!(
        unusable(&sandbox, "tiler"),
        "`module` is relative to the library directory, which is not known here; use `path`"
    );
}

#[test]
fn a_broken_launcher_takes_part_in_the_ambiguity_rule() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "broken", "path = \"/opt/gone.py\"\n");
    sandbox.touch("usr/share/gnoblin/shells/fine.so");
    match discover(&sandbox.environment, &Request::Discover).expect_err("two candidates") {
        DiscoveryError::Ambiguous { candidates, .. } => {
            let names: Vec<&str> = candidates.iter().map(|candidate| candidate.name.as_str()).collect();
            assert_eq!(names, vec!["broken", "fine"]);
            assert_eq!(candidates[0].form, Form::Launcher);
            assert!(candidates[0].target.is_err());
        }
        other => panic!("expected an ambiguity, got {other}"),
    }
}

#[test]
fn a_launcher_named_default_wins_over_a_broken_launcher() {
    let sandbox = Sandbox::new();
    install_launcher(&sandbox, "broken", "path = \"/opt/gone.py\"\n");
    let script = sandbox.touch("opt/shell.py");
    install_launcher(&sandbox, "default", &format!("path = \"{}\"\n", script.display()));
    assert_eq!(
        discover(&sandbox.environment, &Request::Discover).expect("found").path,
        script
    );
}

#[test]
fn a_launcher_file_can_be_given_as_a_path() {
    let sandbox = Sandbox::new();
    let script = sandbox.touch("opt/tiler/shell.py");
    let launcher = sandbox.write("work/tiler.toml", &format!("path = \"{}\"\n", script.display()));
    let selection = by_path(&sandbox, "tiler.toml").expect("found");
    assert_eq!(selection.path, script);
    assert_eq!(selection.launcher, Some(launcher));
    assert_eq!(selection.chosen_by, Tier::CommandLine);
}

// Direct paths and language detection.

#[test]
fn a_file_that_starts_with_the_elf_magic_is_native_whatever_it_is_called() {
    let sandbox = Sandbox::new();
    sandbox.write_elf("work/myshell");
    sandbox.write_elf("work/odd.py");
    assert_eq!(by_path(&sandbox, "myshell").expect("found").language, Language::Native);
    assert_eq!(by_path(&sandbox, "./odd.py").expect("found").language, Language::Native);
}

#[test]
fn an_unknown_extension_is_named_for_the_loader_to_decide() {
    let sandbox = Sandbox::new();
    sandbox.touch("work/shell.rb");
    sandbox.touch("work/plain");
    assert_eq!(
        by_path(&sandbox, "./shell.rb").expect("found").language,
        Language::Other("rb".to_owned())
    );
    assert_eq!(
        by_path(&sandbox, "./plain").expect("found").language,
        Language::Other(String::new())
    );
}

#[test]
fn a_bare_word_that_is_not_a_file_suggests_the_name_option() {
    let sandbox = Sandbox::new();
    match by_path(&sandbox, "tiler").expect_err("not a file") {
        DiscoveryError::PathMissing { bare_word, source, .. } => {
            assert!(bare_word);
            assert_eq!(source, Source::CommandLine);
        }
        other => panic!("expected a missing path, got {other}"),
    }
    match by_path(&sandbox, "./tiler").expect_err("not a file") {
        DiscoveryError::PathMissing { bare_word, .. } => assert!(!bare_word),
        other => panic!("expected a missing path, got {other}"),
    }
}

#[test]
fn a_bare_word_that_is_a_file_is_a_path() {
    let sandbox = Sandbox::new();
    sandbox.touch("work/tiler.so");
    assert_eq!(by_path(&sandbox, "tiler.so").expect("found").name, "tiler");
}

#[test]
fn a_path_that_is_not_a_regular_file_is_unusable() {
    let sandbox = Sandbox::new();
    let error = by_path(&sandbox, "/dev/null").expect_err("a device");
    match error {
        DiscoveryError::PathUnusable { reason, .. } => assert_eq!(reason, "is not a regular file"),
        other => panic!("expected an unusable path, got {other}"),
    }
}
