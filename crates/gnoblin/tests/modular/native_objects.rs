//! Example 26 run for real from the three languages: the shipped binary serving a shell that makes a workspace through the classes of
//! `libgnoblin-std`, with real Wayland clients that map windows.
//!
//! The shell gives its workspace the library's master and stack layout, then, when a third window joins, a layout of its own (equal
//! columns), then the library's layout again with its divider moved the way a drag moves it (`find_split`, `set_split`). What is asserted
//! is what the compositor's tree holds: master and stack tiles that differ, three columns of one width and one height, and a master that
//! grew when the divider moved. One core and one std are mapped.

use std::collections::BTreeMap;

use crate::support::native_shell::{Gnoblin, Start, start_script, std_shells};
use crate::support::{Mapper, scene};
use crate::{copy_example, serving};

use super::native_std::{assert_one_core_one_std, start_std};

type Rects = BTreeMap<String, (f64, f64, f64, f64)>;

fn rects(session: &Gnoblin) -> Rects {
    scene(&session.ipc())
        .into_iter()
        .map(|window| (window.app_id, (window.x, window.y, window.width, window.height)))
        .collect()
}

#[track_caller]
fn the_workspace_is_laid_out_by_the_library_and_by_the_shell(session: &mut Gnoblin) {
    session.wait_log("the workspace is in the list", |line| {
        line.contains("objects: the workspace main is in the list at index 0")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    for name in ["a", "b"] {
        mapper.map_window(name, name);
    }
    session.wait_log("two windows joined", |line| line.contains("joined main, 2 windows"));
    // The library's master and stack: the master takes the left half, the second window the right one.
    session.wait_for("the library tiles two windows as a master and stack", |session| {
        let now = rects(session);
        now.len() == 2 && (now["a"].0 - now["b"].0).abs() > 1.0
    });

    mapper.map_window("c", "c");
    session.wait_log("the shell's own layout tiles three windows", |line| {
        line.to_lowercase()
            .contains("a layout of the shell's own tiles 3 windows: true")
    });
    session.wait_for("three columns of one width", |session| {
        let now = rects(session);
        now.len() == 3 && {
            let (a, b, c) = (now["a"], now["b"], now["c"]);
            (a.2 - b.2).abs() < 1.0 && (b.2 - c.2).abs() < 1.0 && (a.1 - c.1).abs() < 1.0 && a.0 < b.0 && b.0 < c.0
        }
    });

    mapper.map_window("d", "d");
    session.wait_log("the library's layout is back", |line| {
        line.to_lowercase()
            .contains("back to the library's layout with 4 windows: true")
    });
    mapper.map_window("e", "e");
    let moved = session.wait_log("the divider moved", |line| line.contains("the divider is at"));
    assert!(
        moved.text.contains("moves to 0.7: ") && moved.text.to_lowercase().contains(": true"),
        "{}",
        moved.text
    );
    session.wait_for("the master grew with the divider", |session| {
        let now = rects(session);
        now.len() == 5 && now["a"].2 / (now["a"].2 + now["b"].2) > 0.6
    });
    assert_one_core_one_std(session);
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_26_a_std_shell_lays_out_a_workspace_with_its_own_layout_and_moves_a_divider() {
    let mut session = start_std(&std_shells().objects);
    the_workspace_is_laid_out_by_the_library_and_by_the_shell(&mut session);
}

fn script_session(name: &str, file: &str) -> (tempfile::TempDir, Gnoblin) {
    let dir = copy_example(name);
    let mut session = start_script(Start::Compositor, &dir.path().join(file), &[]);
    serving(&mut session);
    (dir, session)
}

#[test]
fn example_26_a_typescript_shell_lays_out_a_workspace_with_its_own_layout_and_moves_a_divider() {
    let (_dir, mut session) = script_session("26-native-objects-ts", "shell.ts");
    the_workspace_is_laid_out_by_the_library_and_by_the_shell(&mut session);
}

#[test]
fn example_26_a_python_shell_lays_out_a_workspace_with_its_own_layout_and_moves_a_divider() {
    let (_dir, mut session) = script_session("26-native-objects-py", "shell.py");
    the_workspace_is_laid_out_by_the_library_and_by_the_shell(&mut session);
    assert!(!session.log_text().contains("Traceback"), "{}", session.log_text());
}
