//! Example 39 (`examples/39-native-errors`), run for real from the three languages: the shipped binary serving a shell that makes the
//! failures a call can have and logs what each one says.
//!
//! What it asserts is what the shell logs about the failure it caught. A call the library refuses gives a `GnoblinError` (a `Fail` in Rust)
//! with a `code` and a `target`, `Class#id.member`. The failure modes, each of which must be told apart from the others:
//!
//! - a refused write: a second `InputContext` whose name is taken is refused with `already_exists`, and the target names the manager's
//!   member that was called (`InputManager`, `add_context` or `addContext`, which has no id and so no `#`);
//! - a call on an object that is gone: a window that has closed is still a `Client`, and `close()` on it is refused with `not_found` and
//!   the target `Client#<id>.close`, the id being the window's own;
//! - a bad argument: a context with no name is `TypeError` in TypeScript and Python and has no code in Rust. It is not a `GnoblinError`, so a
//!   script's `catch` for a refused call does not take it;
//! - nothing is the wrong way round: the first context is accepted, a closed window is never answered, and the Python session has no
//!   traceback.

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Home, Launch, Start, shells, start_script_in, wait_for_window};
use crate::{copy_example, serving};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    TypeScript,
    Python,
}

fn session(language: Language) -> (Option<tempfile::TempDir>, Gnoblin) {
    match language {
        Language::Rust => {
            let launch = Launch::compositor(&[]).with_arg(&shells().errors);
            let mut session = Gnoblin::launch(Home::new(), launch);
            session.wait_serving();
            (None, session)
        }
        Language::TypeScript | Language::Python => {
            let dir = copy_example("39-native-errors");
            let file = if language == Language::TypeScript {
                "shell.ts"
            } else {
                "shell.py"
            };
            let mut session = start_script_in(Home::new(), None, Start::Compositor, &dir.path().join(file), &[]);
            serving(&mut session);
            (Some(dir), session)
        }
    }
}

/// The line the shell logged that holds `needle`, once it has.
#[track_caller]
fn logged(session: &mut Gnoblin, needle: &str) -> String {
    session.wait_log(needle, |line| line.contains(needle)).text
}

/// The target a line names, as `target=<text>` up to the next space (the log adds its own fields after the shell's text).
fn target_of(line: &str) -> &str {
    line.split_once("target=")
        .and_then(|(_, target)| target.split_whitespace().next())
        .unwrap_or("")
}

#[track_caller]
fn the_failures_say_why(language: Language, session: &mut Gnoblin) {
    session.wait_log("the shell is running", |line| line.contains("errors: running"));

    // A refused write: the name is taken.
    let taken = logged(session, "errors: a taken name is refused: ");
    assert!(
        taken.contains("code=already_exists"),
        "a taken name is already_exists: {taken}"
    );
    let target = target_of(&taken);
    let member = target.to_lowercase().replace('_', "");
    assert!(
        target.starts_with("InputManager") && member.ends_with("addcontext"),
        "the target is the manager's member: {taken}"
    );
    assert!(
        !session.log_text().contains("a taken name was accepted"),
        "{}",
        session.log_text()
    );

    // A bad argument has no code: it is a TypeError, and not a GnoblinError.
    let bad = logged(session, "errors: a bad argument is refused: ");
    assert!(
        bad.contains("TypeError=true"),
        "a name that is empty is a bad argument, which has no code ({language_name}): {bad}",
        language_name = match language {
            Language::Rust => "Rust",
            Language::TypeScript => "TypeScript",
            Language::Python => "Python",
        }
    );
    assert!(
        !session.log_text().contains("an empty name was accepted"),
        "{}",
        session.log_text()
    );

    // A call on an object that is gone: the window closed, and its `Client` says it is gone. The window is closed once
    // the compositor has published it: closed at once, the shell logged no closed window (Fedora 43 job 139077575345),
    // as a window can open and close before the shell's model hears of it.
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("foot", "Terminal");
    wait_for_window(session, "the compositor publishes the window", "foot", |_| true);
    mapper.close_window(0);
    let gone = logged(session, "errors: Terminal: a closed window is refused: ");
    assert!(
        gone.contains("code=not_found"),
        "a window that is gone is not_found: {gone}"
    );
    let target = target_of(&gone);
    assert!(
        target.starts_with("Client#")
            && target.ends_with(".close")
            && target["Client#".len()..target.len() - ".close".len()]
                .chars()
                .all(|c| c.is_ascii_digit())
            && target.len() > "Client#.close".len(),
        "the target is Client#<id>.close: {gone}"
    );
    assert!(
        !session.log_text().contains("a closed window was answered"),
        "a closed window is never answered: {}",
        session.log_text()
    );
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_39_a_core_shell_reads_the_code_and_target_of_a_failed_call() {
    let (_dir, mut session) = session(Language::Rust);
    the_failures_say_why(Language::Rust, &mut session);
}

#[test]
fn example_39_a_typescript_shell_catches_a_gnoblin_error_with_a_code_and_target() {
    let (_dir, mut session) = session(Language::TypeScript);
    the_failures_say_why(Language::TypeScript, &mut session);
}

#[test]
fn example_39_a_python_shell_catches_a_gnoblin_error_with_a_code_and_target() {
    let (_dir, mut session) = session(Language::Python);
    the_failures_say_why(Language::Python, &mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
