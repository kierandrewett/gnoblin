//! Example 25 run for real from the three languages: the shipped binary serving a shell that gives every window a style, stages a
//! transition that changes a style and a node of its own, and aborts it. What is asserted is what the shell logs it read back (the
//! radius and shadows it set, the rollback that ran once, the opacity the library put back) and what the tree holds: the shell's
//! `probe` node, where the aborted transition left it. One core and one std are mapped.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells, wait_for_nodes};
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
    assert!(
        text.contains("rollback ran 1"),
        "{}\n{}",
        aborted.text,
        session.log_text()
    );
    assert!(text.contains("opacity is set false"), "{}", aborted.text);
    assert!(text.contains("x is 0"), "{}", aborted.text);

    // The shell logged the line before the tree held its node: it sends what it wrote from its first turn after it started, and the
    // compositor applies that on its own thread. The tree is read when it holds the node.
    let is_probe = |node: &gnoblin_proto::tree::snapshot_node::Properties| {
        node.props
            .iter()
            .any(|prop| prop.key == "name" && prop.value_text == "probe")
    };
    let nodes = wait_for_nodes(session, "the tree holds the shell's probe node", |nodes| {
        nodes.iter().any(is_probe)
    });
    let probe = nodes
        .iter()
        .find(|node| is_probe(node))
        .expect("the probe was in the tree that was read");
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
    let (_dir, mut session) = script_session("25-native-style", "shell.ts");
    assert_the_style_is_set_and_the_transition_rolled_back(&mut session);
}

#[test]
fn example_25_a_python_shell_styles_the_windows_and_its_aborted_transition_is_rolled_back() {
    let (_dir, mut session) = script_session("25-native-style", "shell.py");
    assert_the_style_is_set_and_the_transition_rolled_back(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
