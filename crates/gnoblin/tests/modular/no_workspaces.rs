//! Example 8, a shell with no dependency on workspaces or scene graphs, in a real session and in both languages it exists in: the Rust
//! tiler (`examples/08-no-workspaces`) and the Python one (`examples/08-no-workspaces/shell.py`).
//!
//! The tiler writes the rectangle of each raw window of the tree and nothing else. What these tests assert is the claim of the spec: the
//! tree of such a session holds no node of any class the compositor's own session does not (no workspace, camera, canvas, view, tween
//! or effect: a shell makes those, and this one makes none), no std library is mapped, and the windows still tile as they come and go.

use std::collections::BTreeSet;
use std::path::Path;

use crate::support::native_shell::{Gnoblin, Start, shells, start_script, wait_for_window};
use crate::support::{Mapper, snapshot};
use crate::{copy_example, screen_of, serving};

/// The classes of the nodes the tree of `session` holds.
fn classes(session: &Gnoblin) -> BTreeSet<String> {
    snapshot(&session.ipc()).into_iter().map(|node| node.class).collect()
}

/// The classes of a session whose shell holds nothing and places nothing, with one window open: what the compositor itself makes.
fn compositor_classes() -> BTreeSet<String> {
    let mut session = Gnoblin::start(Start::Compositor, &shells().idle, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("baseline", "Baseline");
    wait_for_window(&mut session, "the window is in the tree", "baseline", |_| true);
    classes(&session)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

/// One window fills the screen less the gaps, two share it master and stack, the master closing leaves the other filling it, and the tree
/// never holds a class of node the compositor's own does not.
#[track_caller]
fn tiles_raw_windows_and_makes_no_node(session: &mut Gnoblin, screen: (f64, f64)) {
    let baseline = compositor_classes();
    let gap = 10.0;
    let (inner_width, inner_height) = (screen.0 - 2.0 * gap, screen.1 - 2.0 * gap);
    let half = (inner_width - gap) / 2.0;

    let mut mapper = Mapper::connect(&session.wayland());
    mapper.map_window("a", "A");
    wait_for_window(session, "one window fills the screen less the gaps", "a", |window| {
        near(window.x, gap)
            && near(window.y, gap)
            && near(window.width, inner_width)
            && near(window.height, inner_height)
    });
    mapper.map_window("b", "B");
    wait_for_window(session, "the master takes the left half", "a", |window| {
        near(window.x, gap) && near(window.width, half) && near(window.height, inner_height)
    });
    wait_for_window(session, "the other takes the right half", "b", |window| {
        near(window.x, gap + half + gap) && near(window.width, half) && near(window.height, inner_height)
    });

    let held = classes(session);
    let extra: Vec<&String> = held.difference(&baseline).collect();
    assert!(
        extra.is_empty(),
        "the shell made nodes of classes the compositor does not ({extra:?}); the tree holds {held:?} and the compositor alone {baseline:?}"
    );
    assert!(
        session
            .mapped_files()
            .iter()
            .all(|path| !path.contains("gnoblin-std") && !path.contains("gnoblin_std")),
        "no std library is mapped: {:#?}",
        session.mapped_files()
    );

    mapper.close_window(0);
    wait_for_window(session, "the window left fills the screen again", "b", |window| {
        near(window.x, gap) && near(window.width, inner_width) && near(window.height, inner_height)
    });
    assert!(session.exited().is_none(), "{}", session.log_text());
}

#[test]
fn example_8_the_rust_tiler_tiles_raw_windows_and_the_tree_holds_no_node_of_its_own() {
    let mut session = Gnoblin::start(Start::Compositor, &shells().tiler, &[], &[]);
    session.wait_serving();
    session.wait_log("the native shell serves in process", |line| {
        line.contains("serves in process")
    });
    let screen = crate::support::native_shell::screen_size(&session.ipc());
    tiles_raw_windows_and_makes_no_node(&mut session, screen);
}

#[test]
fn example_8_the_python_tiler_tiles_raw_windows_and_the_tree_holds_no_node_of_its_own() {
    let dir = copy_example("08-no-workspaces");
    let entry: &Path = &dir.path().join("shell.py");
    let mut session = start_script(Start::Compositor, entry, &[]);
    serving(&mut session);
    let screen = screen_of(&session);
    tiles_raw_windows_and_makes_no_node(&mut session, screen);
}
