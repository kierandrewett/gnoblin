//! Example 25 run for real from the three languages: the shipped binary serving a shell that gives every window a style, stages a
//! transition that changes a style and a node of its own, and aborts it. What is asserted is what the shell logs it read back (the
//! radius and shadows it set, the rollback that ran once, the opacity the library put back) and what the tree holds: the shell's
//! `probe` node, where the aborted transition left it. One core and one std are mapped.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::support::snapshot;
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

#[track_caller]
fn assert_the_style_is_set_and_the_transition_rolled_back(session: &mut Gnoblin) {
    let line = session.wait_log("the shell styled the windows", |line| {
        line.contains("style: corner radius")
    });
    let text = line.text.to_lowercase();
    assert!(text.contains("corner radius 12"), "{}", line.text);
    assert!(text.contains("2 shadows"), "{}", line.text);
    assert!(text.contains("the first blur 8"), "{}", line.text);

    let aborted = session.wait_log("the shell aborted its transition", |line| {
        line.contains("style: transition aborted")
    });
    let text = aborted.text.to_lowercase();
    assert!(text.contains("rollback ran 1"), "{}", aborted.text);
    assert!(text.contains("opacity is set false"), "{}", aborted.text);
    assert!(text.contains("x is 0"), "{}", aborted.text);

    let ipc = session.ipc();
    let nodes = snapshot(&ipc);
    let probe = nodes
        .iter()
        .find(|node| {
            node.props
                .iter()
                .any(|prop| prop.key == "name" && prop.value_text == "probe")
        })
        .expect("the tree has the shell's probe node");
    let x = probe
        .props
        .iter()
        .find(|prop| prop.key == "x")
        .map_or(0.0, |prop| prop.value_double);
    assert_eq!(x, 0.0, "the aborted transition left the probe where it was");
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_25_a_std_shell_styles_the_windows_and_its_aborted_transition_is_rolled_back() {
    let mut session = start_std(&std_shells().style);
    assert_the_style_is_set_and_the_transition_rolled_back(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_25_a_typescript_shell_styles_the_windows_and_its_aborted_transition_is_rolled_back() {
    let (_dir, mut session) = script_session("25-native-style-ts", "shell.ts");
    assert_the_style_is_set_and_the_transition_rolled_back(&mut session);
}

#[test]
fn example_25_a_python_shell_styles_the_windows_and_its_aborted_transition_is_rolled_back() {
    let (_dir, mut session) = script_session("25-native-style-py", "shell.py");
    assert_the_style_is_set_and_the_transition_rolled_back(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
