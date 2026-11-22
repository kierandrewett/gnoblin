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

/// What the shell logged of the value classes it built. Every class was built with its defaults and with values of its own, handed to the
/// library, and read back, and the three languages log the same words. F5: one name, field order and default in every language (a `Border`
/// is 1 wide and white, a `Shadow` blurs 12 and is translucent black, a blur is 12, a grade is 1); F6: a list takes the variants' classes;
/// F7: the library reads the records the classes encode (its own tests of the record fields are unchanged) and answers the same ones; F8:
/// each entry reads back as the variant its tag says; F9: a value that is given stays (a `-2` offset, a `0.5` grade, `inset`), and the
/// default of a field left out is applied once; F13: a shader's source and params cross as the part of the `shader` record they are, and a
/// shader with no params reads back with none; F15: eight filters of five variants in one list each come back in their place.
#[track_caller]
fn assert_the_value_classes_read_back(session: &mut Gnoblin) {
    let mut line_of = |what: &str| {
        let prefix = format!("style: value classes {what} ");
        let line = session.wait_log(what, |line| line.contains(&prefix));
        let at = line.text.find(&prefix).expect("the prefix is in the line");
        let rest = &line.text[at + prefix.len()..];
        let rest = rest.split("  generation=").next().unwrap_or(rest);
        (rest.trim_end_matches(" area=\"script\"").trim().to_owned(), line.text)
    };
    for (what, wanted) in [
        ("borders", "1 #ffffff 0 false | 3 #ff0000 -2 true"),
        ("shadows", "0 0 12 0 #00000080 false | 2 4 20 3 #112233 true"),
        (
            "backdrop filters",
            "blur 12 | blur 20 | saturate 1 | saturate 1.5 | brightness 1 | brightness 0.5 | contrast 1 | contrast 1.25",
        ),
        ("filters", "shader - | shader 0.5,2"),
    ] {
        let (got, text) = line_of(what);
        assert_eq!(got, wanted, "{text}");
    }
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_25_value_classes_with_defaults_read_back_from_a_std_shell() {
    let mut session = start_std(&std_shells().style);
    assert_the_value_classes_read_back(&mut session);
}

#[test]
fn example_25_value_classes_with_defaults_read_back_from_a_typescript_shell() {
    let (_dir, mut session) = script_session("25-native-style", "shell.ts");
    assert_the_value_classes_read_back(&mut session);
}

#[test]
fn example_25_value_classes_with_defaults_read_back_from_a_python_shell() {
    let (_dir, mut session) = script_session("25-native-style", "shell.py");
    assert_the_value_classes_read_back(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
