//! The shipped binary: its flags, its exit codes and its refusals.
//!
//! These tests run `gnoblin` itself, so what they check is the process a supervisor
//! starts: the exit code it sees, the standard error it logs and the flags `--help`
//! promises. A role this build does not serve is the case that matters most — a supervisor
//! that mistook the refusal for a crash would restart a process that can never do the work.
//!
//! The roles that do run are `tests/guardian.rs`'s: a session is processes, so it is tested
//! as processes rather than as a function call.

use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Output};

use tempfile::TempDir;

/// The binary under test, built by cargo for this test.
const BIN: &str = env!("CARGO_BIN_EXE_gnoblin");

/// Runs `gnoblin` with `arguments`.
fn gnoblin(arguments: &[&str]) -> Output {
    Command::new(BIN).args(arguments).output().expect("the binary runs")
}

/// The exit code, which a supervisor reads before anything else.
fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A directory that is private, as `$XDG_RUNTIME_DIR` is.
fn private_directory() -> TempDir {
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the directory mode");
    directory
}

#[test]
fn an_unknown_role_is_a_command_line_error() {
    let output = gnoblin(&["--role", "daemon"]);
    assert_eq!(code(&output), 2, "a bad command line is exit 2");
    let message = stderr(&output);
    for role in ["guardian", "compositor", "shell", "services", "check"] {
        assert!(message.contains(role), "the refusal lists `{role}`: {message}");
    }
    assert!(
        !message.contains("runtime"),
        "the script runtime role is the shell role now: {message}"
    );
}

#[test]
fn the_help_documents_only_the_flags_this_build_has() {
    let output = gnoblin(&["--help"]);
    assert_eq!(code(&output), 0, "help is not an error");
    let help = stdout(&output);
    assert!(help.contains("--role <ROLE>"), "{help}");

    // Every long flag the help shows is one this build documents. A flag that appeared
    // here without a role reading it would be a promise the binary does not keep.
    let mut flags: Vec<&str> = help
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| token.starts_with("--"))
        .map(|token| token.trim_end_matches(|character: char| !character.is_ascii_alphanumeric()))
        .collect();
    flags.sort_unstable();
    flags.dedup();
    assert_eq!(
        flags,
        [
            "--help",
            "--isolate",
            "--language",
            "--list-shells",
            "--role",
            "--shell",
            "--version",
            "--watch"
        ],
        "{help}"
    );
    // The help says where the shell comes from: the tiers, in order, and what to do about an ambiguity.
    for text in [
        "PATH",
        "GNOBLIN_SHELL",
        "config.toml",
        "shell.{so,py,js,mjs,ts,mts}",
        "$XDG_DATA_HOME/gnoblin/shells/",
        "$XDG_DATA_DIRS",
        "default-shell",
        "equally preferred",
        "exits 1 before any compositor starts",
    ] {
        assert!(help.contains(text), "the help documents `{text}`:\n{help}");
    }
}

#[test]
fn a_path_and_a_name_together_are_a_usage_error() {
    let output = gnoblin(&["--shell", "tiler", "./x.so"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stderr(&output).contains("cannot be used with"), "{}", stderr(&output));
}

#[test]
fn listing_shells_takes_no_flag_that_runs_one() {
    for flag in ["--watch", "--isolate"] {
        let output = gnoblin(&["--list-shells", flag]);
        assert_eq!(code(&output), 2, "{flag}: {}", stderr(&output));
    }
}

#[test]
fn the_flags_of_the_guardian_are_refused_by_the_roles_it_starts() {
    for arguments in [
        &["--role", "compositor", "--isolate"][..],
        &["--role", "compositor", "--list-shells"][..],
        &["--role", "compositor", "--shell", "tiler"][..],
        &["--role", "services", "./x.so"][..],
        &["--role", "services", "--watch"][..],
        &["--role", "services", "--language", "python"][..],
        &["--role", "shell"][..],
    ] {
        let output = gnoblin(arguments);
        assert_eq!(code(&output), 2, "{arguments:?}: {}", stderr(&output));
        assert!(
            stderr(&output).starts_with("gnoblin: "),
            "{arguments:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn a_shell_path_that_is_not_a_file_is_refused_before_the_compositor_starts() {
    let output = gnoblin(&["--role", "compositor", "/nonexistent/libshell.so"]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("cannot find the shell module"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_version_is_the_full_gnoblin_version() {
    let output = gnoblin(&["--version"]);
    assert_eq!(code(&output), 0);
    let line = stdout(&output);
    let version = gnoblin_compositor::version().to_string();
    assert!(line.contains(&version), "the IPC version {version}: {line}");
    assert_eq!(version.split('.').count(), 3, "the version is a full triple");
    assert!(
        line.contains(env!("CARGO_PKG_VERSION")),
        "the build it came from: {line}"
    );
    assert!(line.starts_with("gnoblin "), "{line}");
}

#[test]
fn the_compositor_reports_a_refused_start() {
    // A socket in a directory another account can enter is refused before anything
    // binds, so the process exits instead of running a session nobody should reach.
    let directory = private_directory();
    let shared = directory.path().join("shared");
    std::fs::create_dir(&shared).expect("the directory");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).expect("the mode");
    let socket = shared.join("ipc.sock");

    let output = Command::new(BIN)
        .args(["--role", "compositor"])
        .env("GNOBLIN_IPC_SOCKET", &socket)
        .env("XDG_RUNTIME_DIR", directory.path())
        .env("XDG_STATE_HOME", directory.path())
        .output()
        .expect("the binary runs");
    assert_eq!(code(&output), 1, "a refused start is exit 1: {}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("compositor"), "{message}");
    assert!(message.contains("0700"), "the refusal says why: {message}");
    assert!(!socket.exists(), "nothing was bound");
}

#[test]
fn a_start_that_cannot_resolve_a_socket_names_both_variables() {
    let directory = private_directory();
    let output = Command::new(BIN)
        .args(["--role", "compositor"])
        .env("GNOBLIN_IPC_SOCKET", "")
        .env("XDG_RUNTIME_DIR", "")
        .env("HOME", directory.path())
        .output()
        .expect("the binary runs");
    // With neither variable naming a path there is nothing to serve, and the process
    // exits instead of choosing one.
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    let message = stderr(&output);
    assert!(message.contains("GNOBLIN_IPC_SOCKET"), "{message}");
    assert!(message.contains("XDG_RUNTIME_DIR"), "{message}");
}
