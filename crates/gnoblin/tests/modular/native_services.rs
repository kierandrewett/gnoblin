//! Example 23 (`examples/23-native-services`), run for real from three languages: the shipped binary on a headless backend serving a shell that
//! uses the session services of the core: it starts a command line, watches the session for idleness, begins a launch no window answers, and
//! asks for the session lock with `<Super>e`.
//!
//! What is asserted is what the shell logs about what the library told it: the program's exit code and what it printed, the launch's time-out,
//! the idle watch firing with no input, and the lock request settling (accepted or refused: whether the shell's connection
//! may take the lock is the compositor's decision, and the operation settles either way). The dialog policy, the reload gap and the rest of the
//! services are tested against fake connections in `crates/gnoblin-core-impl/tests/services.rs`.

use std::fs;

use crate::support::native_shell::{Gnoblin, KEY_E, Start, Typist, shells, start_script};
use crate::{copy_example, serving};

/// What every shell of the example logs, in the order the session brings it.
#[track_caller]
fn assert_the_services_are_told(session: &mut Gnoblin) {
    session.wait_log("the shell is running", |line| line.contains("services: running"));
    let ended = session.wait_log("the process ended", |line| line.contains("services: the process ended"));
    assert!(
        ended.text.contains(r#"with code 3 and printed "hello""#),
        "{}",
        ended.text
    );
    let late = session.wait_log("the launch timed out", |line| line.contains("services: the launch"));
    assert!(late.text.contains("never timed out"), "{}", late.text);
    session.wait_log("the session was idle", |line| {
        line.contains("services: the session is idle")
    });

    // The chord the shell claimed asks for the lock. That the watch resumes at input is tested against the connection's readings in
    // `crates/gnoblin-core-impl/tests/services.rs`: whether the headless compositor's virtual keyboard resets its idle reading is its own
    // matter, and this session does not decide it.
    let mut typist = Typist::connect(&session.wayland());
    typist.super_plus(KEY_E);
    session.wait_log("the lock request settled", |line| {
        line.contains("services: the lock request settled")
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_23_a_core_shell_is_told_of_a_program_a_launch_idleness_and_the_lock() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().services, &[], &[]);
    session.wait_serving();
    assert_the_services_are_told(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_23_a_typescript_shell_is_told_of_a_program_a_launch_idleness_and_the_lock() {
    let (_dir, mut session) = script_session("23-native-services", "shell.ts");
    assert_the_services_are_told(&mut session);
}

#[test]
fn example_23_a_python_shell_is_told_of_a_program_a_launch_idleness_and_the_lock() {
    let (_dir, mut session) = script_session("23-native-services", "shell.py");
    assert_the_services_are_told(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}

/// The shell of `file` in the example, started late: `delay` runs before its first line and holds the script for longer than the idle
/// time its watch asks for (300 ms) and than the compositor needs to publish a reading with that much idle time in it. The script
/// makes its watch against a reading that already has the session idle for longer than the watch asks, which is what a shell does that
/// starts slowly, as the Python shell does on any machine and the others do on a busy one.
fn late_script_session(name: &str, file: &str, delay: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let path = dir.path().join(file);
    let text = fs::read_to_string(&path).expect("the example's shell");
    fs::write(&path, format!("{delay}\n{text}")).expect("the delayed shell");
    let mut session = start_script(Start::Compositor, &path, &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_23_a_typescript_shell_that_starts_late_is_still_told_the_session_is_idle() {
    let (_dir, mut session) = late_script_session(
        "23-native-services",
        "shell.ts",
        "const resume = Date.now() + 1500; while (Date.now() < resume) {}",
    );
    assert_the_services_are_told(&mut session);
}

#[test]
fn example_23_a_python_shell_that_starts_late_is_still_told_the_session_is_idle() {
    let (_dir, mut session) = late_script_session("23-native-services", "shell.py", "import time; time.sleep(1.5)");
    assert_the_services_are_told(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
