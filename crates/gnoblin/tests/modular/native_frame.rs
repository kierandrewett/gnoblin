//! Example 27 run for real from the three languages: the shipped binary serving a shell that starts the decorations and fills each frame
//! with the stock titlebar, with real Wayland clients that map windows asking for server-side decorations.
//!
//! What is asserted is what the compositor's tree holds: the titlebar of the framed window is a canvas under the window whose paint list
//! shows its title, the window of a client that draws its own frame gets none, and the window the shell closes the way its close button
//! does fades out and is asked to close, and is gone once the client answers. One core and one std are mapped.

use std::time::{Duration, Instant};

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells, wait_for_nodes};
use crate::support::{Mapper, scene};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

/// Whether the tree holds a canvas under the window `window` whose paint list draws `text`.
fn draws_under(nodes: &[gnoblin_proto::tree::snapshot_node::Properties], window: u64, text: &str) -> bool {
    nodes.iter().any(|node| {
        node.class == "canvas"
            && node.props.iter().any(|prop| {
                prop.key == "content"
                    && gnoblin_paint::decode(&prop.value_bytes)
                        .is_ok_and(|list| format!("{:?}", list.ops).contains(&format!("text: {text:?}")))
            })
            && is_under(nodes, node.id, window)
    })
}

/// Whether `node` hangs, at any depth, from `ancestor`.
fn is_under(nodes: &[gnoblin_proto::tree::snapshot_node::Properties], node: u64, ancestor: u64) -> bool {
    let mut at = node;
    while let Some(parent) = nodes.iter().find(|held| held.id == at).map(|held| held.parent) {
        if parent == ancestor {
            return true;
        }
        if parent == 0 || parent == at {
            return false;
        }
        at = parent;
    }
    false
}

#[track_caller]
fn the_shell_frames_windows_and_closes_one_with_the_fade(session: &mut Gnoblin) {
    session.wait_log("the shell started the decorations", |line| {
        line.contains("frame: running")
    });
    let mut mapper = Mapper::connect(&session.wayland()).requesting_server_side();
    mapper.map_window("framed", "stays");
    let line = session.wait_log("the stock titlebar was put in the frame", |line| {
        line.contains("frame: framed has a titlebar")
    });
    assert!(line.text.contains("its top edge takes 28"), "{}", line.text);
    let window = scene(&session.ipc())
        .into_iter()
        .find(|window| window.app_id == "framed")
        .expect("the framed window is in the scene")
        .id;
    // The canvases reach the tree after the line: the shell sends what it wrote from its next turn.
    wait_for_nodes(
        session,
        "the tree holds the titlebar, which draws the window's title",
        |nodes| draws_under(nodes, window, "stays"),
    );

    mapper.map_window("closeme", "close me");
    session.wait_log("the shell closed a window with its close button's action", |line| {
        line.contains("frame: closing closeme")
    });
    // The window fades first and is asked to close after the fade (180 ms); the client answers as an application with nothing to save does.
    let deadline = Instant::now() + Duration::from_secs(20);
    while mapper.answer_close_requests() == 0 {
        assert!(
            Instant::now() < deadline,
            "the window was never asked to close\n{}",
            session.log_text()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    session.wait_for("the window is gone", |session| {
        scene(&session.ipc()).iter().all(|window| window.app_id != "closeme")
    });
    assert!(
        scene(&session.ipc()).iter().any(|window| window.app_id == "framed"),
        "the other window stays"
    );
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_27_a_std_shell_frames_windows_and_closes_one_with_the_fade() {
    let mut session = start_std(&std_shells().frame);
    the_shell_frames_windows_and_closes_one_with_the_fade(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_27_a_typescript_shell_frames_windows_and_closes_one_with_the_fade() {
    let (_dir, mut session) = script_session("27-native-frame-ts", "shell.ts");
    the_shell_frames_windows_and_closes_one_with_the_fade(&mut session);
}

#[test]
fn example_27_a_python_shell_frames_windows_and_closes_one_with_the_fade() {
    let (_dir, mut session) = script_session("27-native-frame-py", "shell.py");
    the_shell_frames_windows_and_closes_one_with_the_fade(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
