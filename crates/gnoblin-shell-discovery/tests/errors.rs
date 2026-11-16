//! The error text, word for word. The launcher prints it after `gnoblin: ` and exits 1 without starting a
//! compositor, so these are the sentences a person reads when a login has no shell.

mod support;

use std::ffi::OsString;
use std::path::PathBuf;

use gnoblin_shell_discovery::{Request, discover};
use support::Sandbox;

/// The error text of a request that must fail, with the sandbox root replaced.
fn failure(sandbox: &Sandbox, request: &Request) -> String {
    let error = discover(&sandbox.environment, request).expect_err("the lookup must fail");
    sandbox.normalized(&error.to_string())
}

const REMEDIES: &str = "Run a shell directly:     gnoblin ./myshell.so
Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)
Set a default:            [shell] default = \"NAME\"  in <root>/home/.config/gnoblin/config.toml";

#[test]
fn no_shell_found_lists_every_place_and_the_three_remedies() {
    let sandbox = Sandbox::new();
    let expected = format!(
        "no shell found.

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. <root>/home/.config/gnoblin/config.toml   missing
  4. <root>/home/.config/gnoblin/shell.{{so,py,js,mjs,ts,mts}}   none present
  5. <root>/home/.local/share/gnoblin/shells/  missing
  6. <root>/usr-local/share/gnoblin/shells/  missing;  <root>/usr/share/gnoblin/shells/  missing
  7. <root>/prefix/share/gnoblin/default-shell/shell.*   not installed

{REMEDIES}"
    );
    assert_eq!(failure(&sandbox, &Request::Discover), expected);
}

#[test]
fn the_launcher_prints_the_error_after_its_own_name() {
    let sandbox = Sandbox::new();
    let error = discover(&sandbox.environment, &Request::Discover).expect_err("nothing installed");
    assert!(format!("gnoblin: {error}").starts_with("gnoblin: no shell found.\n\nLooked in this order:\n  1. "));
}

#[test]
fn empty_places_say_empty_and_a_config_without_a_section_says_so() {
    let sandbox = Sandbox::new();
    sandbox.write("home/.config/gnoblin/config.toml", "# nothing\n");
    sandbox.dir("home/.local/share/gnoblin/shells");
    sandbox.dir("usr/share/gnoblin/shells");
    sandbox.dir("prefix/share/gnoblin/default-shell");
    let expected = format!(
        "no shell found.

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. <root>/home/.config/gnoblin/config.toml   no [shell] section
  4. <root>/home/.config/gnoblin/shell.{{so,py,js,mjs,ts,mts}}   none present
  5. <root>/home/.local/share/gnoblin/shells/  empty
  6. <root>/usr-local/share/gnoblin/shells/  missing;  <root>/usr/share/gnoblin/shells/  empty
  7. <root>/prefix/share/gnoblin/default-shell/shell.*   not installed

{REMEDIES}"
    );
    assert_eq!(failure(&sandbox, &Request::Discover), expected);
}

#[test]
fn without_a_home_the_error_names_the_xdg_expressions_instead() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.home = None;
    sandbox.environment.packaged_default_dir = None;
    let expected = "no shell found.

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. $XDG_CONFIG_HOME/gnoblin/config.toml   no home directory
  4. $XDG_CONFIG_HOME/gnoblin/shell.{so,py,js,mjs,ts,mts}   no home directory
  5. $XDG_DATA_HOME/gnoblin/shells/  no home directory
  6. <root>/usr-local/share/gnoblin/shells/  missing;  <root>/usr/share/gnoblin/shells/  missing
  7. <prefix>/share/gnoblin/default-shell/shell.*   location unknown

Run a shell directly:     gnoblin ./myshell.so
Pick an installed shell:  gnoblin --shell NAME    (gnoblin --list-shells)
Set a default:            [shell] default = \"NAME\"  in $XDG_CONFIG_HOME/gnoblin/config.toml";
    assert_eq!(failure(&sandbox, &Request::Discover), expected);
}

#[test]
fn an_ambiguity_lists_the_candidates_then_where_it_looked_then_the_remedies() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/lines.py");
    let module = sandbox.touch("opt/tiler/libtiler.so");
    sandbox.write(
        "home/.local/share/gnoblin/shells/tiler.toml",
        &format!("path = \"{}\"\n", module.display()),
    );
    let expected = format!(
        "several shells are equally preferred:
  lines  <root>/home/.local/share/gnoblin/shells/lines.py  (python)
  tiler  <root>/home/.local/share/gnoblin/shells/tiler.toml -> <root>/opt/tiler/libtiler.so  (native)

Looked in this order:
  1. command line            not given
  2. GNOBLIN_SHELL           not set
  3. <root>/home/.config/gnoblin/config.toml   missing
  4. <root>/home/.config/gnoblin/shell.{{so,py,js,mjs,ts,mts}}   none present
  5. <root>/home/.local/share/gnoblin/shells/  2 shells
  6. <root>/usr-local/share/gnoblin/shells/  not reached;  <root>/usr/share/gnoblin/shells/  not reached
  7. <root>/prefix/share/gnoblin/default-shell/shell.*   not reached

{REMEDIES}"
    );
    assert_eq!(failure(&sandbox, &Request::Discover), expected);
}

#[test]
fn an_ambiguity_among_the_conventional_files_names_both() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.config/gnoblin/shell.so");
    sandbox.touch("home/.config/gnoblin/shell.ts");
    let text = failure(&sandbox, &Request::Discover);
    assert!(
        text.starts_with(
            "several shells are equally preferred:
  shell  <root>/home/.config/gnoblin/shell.so  (native)
  shell  <root>/home/.config/gnoblin/shell.ts  (ts)

Looked in this order:
"
        ),
        "{text}"
    );
    assert!(
        text.contains("  4. <root>/home/.config/gnoblin/shell.{so,py,js,mjs,ts,mts}   2 shells\n"),
        "{text}"
    );
}

#[test]
fn an_ambiguous_name_has_no_trace_because_it_looked_nowhere_else() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/foo.so");
    sandbox.touch("home/.local/share/gnoblin/shells/foo.py");
    let expected = format!(
        "several shells are equally preferred:
  foo  <root>/home/.local/share/gnoblin/shells/foo.py  (python)
  foo  <root>/home/.local/share/gnoblin/shells/foo.so  (native)

{REMEDIES}"
    );
    assert_eq!(failure(&sandbox, &Request::Name("foo".to_owned())), expected);
}

#[test]
fn a_missing_name_lists_what_is_installed_and_the_nearest_match() {
    let sandbox = Sandbox::new();
    sandbox.touch("home/.local/share/gnoblin/shells/tiler.so");
    sandbox.touch("home/.local/share/gnoblin/shells/lines.py");
    let expected = "no shell named \"tilr\" (from the command line).
Did you mean \"tiler\"?

Installed shells:
  lines  <root>/home/.local/share/gnoblin/shells/lines.py
  tiler  <root>/home/.local/share/gnoblin/shells/tiler.so

Looked in:
  <root>/home/.local/share/gnoblin/shells/
  <root>/usr-local/share/gnoblin/shells/  (missing)
  <root>/usr/share/gnoblin/shells/  (missing)
  <root>/prefix/share/gnoblin/default-shell/  (missing)";
    assert_eq!(failure(&sandbox, &Request::Name("tilr".to_owned())), expected);
}

#[test]
fn a_missing_name_with_nothing_installed_says_so() {
    let sandbox = Sandbox::new();
    let expected = "no shell named \"tiler\" (from the command line).

No shells are installed.

Looked in:
  <root>/home/.local/share/gnoblin/shells/  (missing)
  <root>/usr-local/share/gnoblin/shells/  (missing)
  <root>/usr/share/gnoblin/shells/  (missing)
  <root>/prefix/share/gnoblin/default-shell/  (missing)";
    assert_eq!(failure(&sandbox, &Request::Name("tiler".to_owned())), expected);
}

#[test]
fn a_missing_name_from_the_environment_or_the_config_says_who_asked() {
    let mut sandbox = Sandbox::new();
    sandbox.environment.gnoblin_shell = Some(OsString::from("ghost"));
    assert!(failure(&sandbox, &Request::Discover).starts_with("no shell named \"ghost\" (from GNOBLIN_SHELL)."));
    sandbox.environment.gnoblin_shell = None;
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\ndefault = \"ghost\"\n");
    assert!(
        failure(&sandbox, &Request::Discover)
            .starts_with("no shell named \"ghost\" (from [shell] in <root>/home/.config/gnoblin/config.toml).")
    );
}

#[test]
fn a_missing_path_names_who_asked() {
    let mut sandbox = Sandbox::new();
    assert_eq!(
        failure(&sandbox, &Request::Path(PathBuf::from("./nope.so"))),
        "./nope.so: no such file (from the command line)."
    );
    sandbox.environment.gnoblin_shell = Some(OsString::from("./x.so"));
    assert_eq!(
        failure(&sandbox, &Request::Discover),
        "./x.so: no such file (from GNOBLIN_SHELL)."
    );
    sandbox.environment.gnoblin_shell = None;
    sandbox.write("home/.config/gnoblin/config.toml", "[shell]\npath = \"ghost.so\"\n");
    assert_eq!(
        failure(&sandbox, &Request::Discover),
        "<root>/home/.config/gnoblin/ghost.so: no such file (from [shell] in <root>/home/.config/gnoblin/config.toml)."
    );
}

#[test]
fn a_bare_word_that_is_not_a_file_says_did_you_mean_shell_name() {
    let sandbox = Sandbox::new();
    assert_eq!(
        failure(&sandbox, &Request::Path(PathBuf::from("tiler"))),
        "tiler: no such file (from the command line).
Did you mean --shell tiler?    (a path needs a `/`, as in ./tiler)"
    );
}

#[test]
fn an_unusable_shell_says_what_is_wrong_with_it() {
    let sandbox = Sandbox::new();
    sandbox.write("usr/share/gnoblin/shells/tiler.toml", "path = \"/opt/gone/shell.py\"\n");
    assert_eq!(
        failure(&sandbox, &Request::Name("tiler".to_owned())),
        "the shell \"tiler\" cannot be used (from the command line):
  <root>/usr/share/gnoblin/shells/tiler.toml: its target /opt/gone/shell.py does not exist"
    );
    assert_eq!(
        failure(&sandbox, &Request::Discover),
        "the shell \"tiler\" cannot be used:
  <root>/usr/share/gnoblin/shells/tiler.toml: its target /opt/gone/shell.py does not exist"
    );
}

#[test]
fn a_directory_without_a_shell_file_says_what_to_put_in_it() {
    let sandbox = Sandbox::new();
    sandbox.dir("work/mine");
    assert_eq!(
        failure(&sandbox, &Request::Path(PathBuf::from("mine"))),
        "mine: is a directory with no shell.{so,py,js,mjs,ts,mts} in it (from the command line)."
    );
}

#[test]
fn a_name_that_cannot_be_a_name_says_to_give_a_path() {
    let sandbox = Sandbox::new();
    assert_eq!(
        failure(&sandbox, &Request::Name("a/b".to_owned())),
        "\"a/b\" is not a shell name (from the command line): a name is one word with no `/`; to run a file, give its path."
    );
}

#[test]
fn a_bad_config_names_the_file_and_the_problem() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "home/.config/gnoblin/config.toml",
        "[shell]\ndefault = \"a\"\npath = \"/b\"\n",
    );
    assert_eq!(
        failure(&sandbox, &Request::Discover),
        "<root>/home/.config/gnoblin/config.toml: `[shell]` sets both `default` and `path`; keep one"
    );
}

#[test]
fn an_ambiguity_whose_candidate_is_broken_says_so_in_the_list() {
    let sandbox = Sandbox::new();
    sandbox.write("usr/share/gnoblin/shells/broken.toml", "path = \"/opt/gone.py\"\n");
    sandbox.touch("usr/share/gnoblin/shells/fine.so");
    let text = failure(&sandbox, &Request::Discover);
    assert!(
        text.starts_with(
            "several shells are equally preferred:
  broken  <root>/usr/share/gnoblin/shells/broken.toml  (unusable: its target /opt/gone.py does not exist)
  fine    <root>/usr/share/gnoblin/shells/fine.so  (native)
"
        ),
        "{text}"
    );
}
