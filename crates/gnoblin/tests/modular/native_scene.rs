//! Example 24 run for real from the three languages: the shipped binary serving a shell that makes a `Camera3D` and a `View` of the
//! scene of `libgnoblin-std`, places them and mounts them under the root. What is asserted is what the shell logs it read back, and
//! what the tree holds: a `camera` node and a `view` node, named as the shell named them, at the places the shell gave them, under
//! the root. One core and one std are mapped.

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells, wait_for_nodes};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

type Node = gnoblin_proto::tree::snapshot_node::Properties;

/// The node of `class` the shell named `name`.
fn named<'a>(nodes: &'a [Node], class: &str, name: &str) -> Option<&'a Node> {
    nodes.iter().find(|node| {
        node.class == class
            && node
                .props
                .iter()
                .any(|prop| prop.key == "name" && prop.value_text == name)
    })
}

#[track_caller]
fn assert_the_scene_is_placed_and_read_back(session: &mut Gnoblin) {
    let line = session.wait_log("the shell placed its nodes", |line| {
        line.contains("scene: the view panel is node")
    });
    assert!(line.text.contains("at 0.5 0.2 -1.4"), "{}", line.text);
    assert!(line.text.contains("the camera cam is node"), "{}", line.text);
    assert!(line.text.contains("with fov 70"), "{}", line.text);

    // The shell logged the line before the tree held its nodes: it sends what it wrote from its first turn after it started, and the
    // compositor applies that on its own thread. The tree is read when it holds them.
    let nodes = wait_for_nodes(session, "the tree holds the shell's camera and view", |nodes| {
        named(nodes, "camera", "cam").is_some() && named(nodes, "view", "panel").is_some()
    });
    let number = |node: &Node, key: &str| -> f64 {
        node.props
            .iter()
            .find(|prop| prop.key == key)
            .map_or(0.0, |prop| prop.value_double)
    };
    let view = named(&nodes, "view", "panel").expect("the view was in the tree that was read");
    let camera = named(&nodes, "camera", "cam").expect("the camera was in the tree that was read");
    assert_eq!(
        [number(view, "x"), number(view, "y"), number(view, "z")],
        [0.5, 0.2, -1.4]
    );
    assert_eq!(
        [number(camera, "x"), number(camera, "y"), number(camera, "z")],
        [1.0, 2.0, 3.0]
    );
    assert_eq!(number(camera, "fov"), 70.0);
    assert_eq!(number(view, "width"), 400.0);
    assert_eq!(view.parent, camera.parent, "both hang from the scene's root");
    assert_eq!(
        view.props
            .iter()
            .find(|prop| prop.key == "camera")
            .map(|prop| prop.value_node),
        Some(camera.id),
        "the view draws through the camera"
    );
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_24_a_std_shell_places_a_view_and_a_camera_and_the_tree_holds_them() {
    let mut session = start_std(&std_shells().scene);
    assert_the_scene_is_placed_and_read_back(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_24_a_typescript_shell_places_a_view_and_a_camera_and_the_tree_holds_them() {
    let (_dir, mut session) = script_session("24-native-scene-ts", "shell.ts");
    assert_the_scene_is_placed_and_read_back(&mut session);
}

#[test]
fn example_24_a_python_shell_places_a_view_and_a_camera_and_the_tree_holds_them() {
    let (_dir, mut session) = script_session("24-native-scene-py", "shell.py");
    assert_the_scene_is_placed_and_read_back(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
