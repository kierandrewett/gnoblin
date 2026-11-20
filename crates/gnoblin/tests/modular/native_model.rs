//! Example 21 of `design/native-std-port.md`, run for real: the shipped binary on a headless backend serving a native shell that uses the
//! typed object model of the core (`gnoblin_core::objects`), with a real Wayland client that maps and closes windows.
//!
//! What it asserts is what the shell logs about what the library told it: the windows there at the start, a selection (`where`) that
//! a window joins when its application id matches and not otherwise, the lookups of a window that is absent (none in each language, not
//! an error) and of one that is there (found by its id), and the close
//! of a window heard through the `gnoblin` object with its last title still readable.

use crate::support::Mapper;
use crate::support::native_shell::{Gnoblin, Start, shells, start_script};
use crate::{copy_example, serving};

#[test]
fn example_21_a_core_shell_follows_the_windows_through_the_object_model() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().model, &[], &[]);
    session.wait_serving();
    follows_the_windows(&mut session, "None");
}

#[test]
fn example_21_a_python_shell_follows_the_windows_through_the_generated_classes() {
    let dir = copy_example("21-native-model-py");
    let mut session = start_script(Start::Compositor, &dir.path().join("shell.py"), &[]);
    serving(&mut session);
    follows_the_windows(&mut session, "None");
}

#[test]
fn example_21_a_typescript_shell_follows_the_windows_through_the_generated_classes() {
    let dir = copy_example("21-native-model-ts");
    let mut session = start_script(Start::Compositor, &dir.path().join("shell.ts"), &[]);
    serving(&mut session);
    follows_the_windows(&mut session, "undefined");
}

/// What either shell of the example logs as windows come and go.
///
/// A lookup of a window that is not there answers none, in the language's own word (`none`), and one of a window that is there answers
/// it: the terminal is found by its id.
fn follows_the_windows(session: &mut Gnoblin, none: &str) {
    session.wait_log("the shell is running", |line| line.contains("model: running"));
    let absent = format!("model: absent lookups: at {none}, get {none}");
    assert!(
        session.log_text().contains(&absent),
        "an absent `at` and `get` answer none, not an error: {}",
        session.log_text()
    );

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("other", "Elsewhere");
    mapper.map_window("foot", "Terminal");
    session.wait_log("the terminal joined the selection", |line| {
        line.contains("model: a terminal joined")
    });
    session.wait_log("the present window was found by its id", |line| {
        line.contains("model: the terminal is found by its id: true")
    });
    assert!(
        !session.log_text().contains("a terminal joined: Elsewhere"),
        "a window of another application is not in the selection: {}",
        session.log_text()
    );

    mapper.close_window(1);
    session.wait_log("the close was heard through the gnoblin object", |line| {
        line.contains("closed, last titled \"Terminal\"")
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}
