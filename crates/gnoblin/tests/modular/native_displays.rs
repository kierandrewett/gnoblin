//! Example 33 run for real from the three languages: the shipped binary serving a shell that works with the output profiles, displays and
//! viewports of `libgnoblin-std` through its generated classes, on the headless backend's one output.
//!
//! The shell lists the displays (the library keeps one `single` display for the output), shows its second workspace on the display
//! through the display's viewport, adds an output profile for the output and hears the compositor take its request, adds a display of
//! its own over the profile (which takes the output from the display of the library) and removes it again (which gives the output a
//! display of the library's), and tries what the checks refuse. What is asserted is what the shell logs and what the compositor's tree
//! holds: the output's scale after the profile, and the workspace shown. One core and one std are mapped.
//!
//! A second output, so a `mirror` and a `span` display, is the replay's (`tests/displays.rs` of the std crate), over the fake compositor:
//! the shipped binary's headless backend has one output.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::support::snapshot;
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// The scale the compositor holds for its first screen.
fn scale(session: &Gnoblin) -> Option<f64> {
    snapshot(&session.ipc())
        .iter()
        .filter(|node| node.class == "screen")
        .flat_map(|node| node.props.iter())
        .find(|prop| prop.key == "output_scale")
        .map(|prop| prop.value_double)
}

#[track_caller]
fn the_displays_are_listed_a_profile_is_applied_and_a_display_comes_and_goes(session: &mut Gnoblin) {
    session.wait_log("the shell started", |line| line.contains("displays: running"));
    let listed = session.wait_log("the library keeps a display for the output", |line| {
        line.contains("displays: listed display-")
    });
    assert!(
        listed.text.contains(":single:1:one"),
        "a single display that presents one output and shows the first workspace: {}",
        listed.text
    );
    session.wait_log("the viewport shows the second workspace", |line| {
        line.contains("displays: the display display-") && line.contains(" shows two")
    });
    session.wait_log("the profile is in the manager", |line| {
        line.contains("is in the manager: true")
    });
    let answered = session.wait_log("the compositor took the profile's request", |line| {
        line.contains("displays: the compositor answered the request")
    });
    assert!(answered.text.contains("answered the request: ok"), "{}", answered.text);
    session.wait_log("the profile reads back", |line| {
        line.contains("displays: the profile reads back scale 1.25 mode preferred")
    });
    session.wait_for("the compositor holds the profile's scale", |session| {
        scale(session).is_some_and(|scale| (scale - 1.25).abs() < 1e-9)
    });

    // The shell's display takes the output from the library's, and the second display over the same profile is refused.
    session.wait_log("the display of the shell's own is listed alone", |line| {
        line.contains("displays: listed main:single:1:one")
    });
    let overlap = session.wait_log("a second display over the output is refused", |line| {
        line.contains("displays: refused overlap:")
    });
    assert!(
        overlap.text.contains("\"rival\"") && overlap.text.contains("\"main\""),
        "the refusal names both displays: {}",
        overlap.text
    );
    session.wait_log("the display is removed", |line| {
        line.contains("displays: removed main: true")
    });
    session.wait_log("the output has a display of the library's again", |line| {
        line.contains("displays: listed display-") && line.contains(":single:1:one")
    });

    let scale_refused = session.wait_log("a scale out of range is refused", |line| {
        line.contains("displays: refused scale:")
    });
    assert!(
        scale_refused.text.contains("must be a number from 0.5 to 4"),
        "{}",
        scale_refused.text
    );
    let selectors = session.wait_log("a display with nothing to present is refused", |line| {
        line.contains("displays: refused selectors:")
    });
    assert!(selectors.text.contains("must list at least one"), "{}", selectors.text);
    session.wait_log("the shell is done", |line| line.contains("displays: done"));
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_33_a_std_shell_lists_the_displays_applies_a_profile_and_adds_and_removes_a_display() {
    let mut session = start_std(&std_shells().displays);
    the_displays_are_listed_a_profile_is_applied_and_a_display_comes_and_goes(&mut session);
}

#[test]
fn example_33_a_typescript_shell_lists_the_displays_applies_a_profile_and_adds_and_removes_a_display() {
    let (_dir, mut session) = script_session("33-native-displays", "shell.ts");
    the_displays_are_listed_a_profile_is_applied_and_a_display_comes_and_goes(&mut session);
}

#[test]
fn example_33_a_python_shell_lists_the_displays_applies_a_profile_and_adds_and_removes_a_display() {
    let (_dir, mut session) = script_session("33-native-displays", "shell.py");
    the_displays_are_listed_a_profile_is_applied_and_a_display_comes_and_goes(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
