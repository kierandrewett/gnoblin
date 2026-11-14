//! Keyboard move and resize (`data/js/gnoblin/reposition.ts`, `Client.moveWithKeys` and `resizeWithKeys`), over the fake
//! compositor of `tests/common`.
//!
//! The cases are the retired shell's `the retired retired corpus`, with the same inputs and the same
//! expected values. A key goes to the node that holds the seat's focus, as the real path delivers it; the real
//! compositor's own proof is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::policy::*;
use common::*;

const STEP: f64 = 16.0;

fn start(windows: u64) -> Rig {
    world(windows, false, "new Layout()", "")
}

/// The node that holds the keyboard while the mode runs, or 0.
fn grab(rig: &mut Rig) -> u64 {
    rig.eval("String(repositioning.grab?.node ?? 0)")
        .parse()
        .expect("a node")
}

fn key(rig: &mut Rig, name: &str) {
    let at = grab(rig);
    assert_ne!(at, 0, "a key needs a running mode");
    rig.comp.key(at, name, "", true);
    settle(rig);
}

fn running(rig: &mut Rig) -> bool {
    rig.eval("String(repositioning.current !== null)") == "true"
}

fn window(rig: &mut Rig, id: u64) -> (f64, f64, f64, f64) {
    let at: Vec<f64> = rect(rig, id).split(',').map(|n| n.parse().expect("a number")).collect();
    (at[0], at[1], at[2], at[3])
}

#[test]
fn an_arrow_key_moves_the_window_by_a_step_return_keeps_it_and_the_keyboard_goes_back_to_the_window() {
    // Before: "an arrow key moves the window by a step, Return keeps it and the keyboard goes back to the window".
    let mut rig = start(1);
    let (x, y, ..) = window(&mut rig, 3);
    assert_eq!(rig.eval("String(gnoblin.clients.get('3').moveWithKeys())"), "true");
    settle(&mut rig);
    let at = grab(&mut rig);
    assert_ne!(at, 0, "the mode has a node");
    assert_eq!(focus(&rig), at, "the mode holds the keyboard");
    key(&mut rig, "Right");
    key(&mut rig, "Down");
    let (nx, ny, ..) = window(&mut rig, 3);
    assert_eq!((nx, ny), (x + STEP, y + STEP), "the window moved one step on each axis");
    key(&mut rig, "Return");
    assert!(!running(&mut rig), "the mode is over");
    assert_eq!(grab(&mut rig), 0);
    assert!(
        rig.comp.canvases().into_iter().all(|id| id != at),
        "and its node is destroyed"
    );
    assert_eq!(window(&mut rig, 3).0, x + STEP, "the new place is kept");
    assert_eq!(focus(&rig), 3, "the keyboard is the window's");
}

#[test]
fn escape_puts_the_window_back_where_it_was() {
    // Before: "Escape puts the window back where it was".
    let mut rig = start(1);
    let before = window(&mut rig, 3);
    exec(&mut rig, "gnoblin.clients.get('3').resizeWithKeys()");
    settle(&mut rig);
    key(&mut rig, "Left");
    key(&mut rig, "Up");
    let (_, _, w, h) = window(&mut rig, 3);
    assert_eq!((w, h), (before.2 - STEP, before.3 - STEP), "Left and Up shrink");
    key(&mut rig, "Escape");
    assert_eq!(window(&mut rig, 3), before, "Escape restores the rectangle");
    assert!(!running(&mut rig));
}

#[test]
fn space_and_enter_keep_the_place_too_and_a_release_does_nothing() {
    // New: the the retired shell `on_key` names Return, KP_Enter and space and ignores `pressed == false`.
    let mut rig = start(1);
    let (x, ..) = window(&mut rig, 3);
    for name in ["space", "KP_Enter"] {
        exec(&mut rig, "gnoblin.clients.get('3').moveWithKeys()");
        settle(&mut rig);
        key(&mut rig, "Right");
        let at = grab(&mut rig);
        rig.comp.key(at, "Escape", "", false);
        settle(&mut rig);
        assert!(running(&mut rig), "a release of Escape ends nothing");
        key(&mut rig, name);
        assert!(!running(&mut rig), "{name} keeps the place");
    }
    assert_eq!(window(&mut rig, 3).0, x + 2.0 * STEP);
}

#[test]
fn a_move_stays_inside_the_work_area_and_a_resize_keeps_a_minimum_size() {
    // Before: "a move stays inside the work area and a resize keeps a minimum size".
    let mut rig = start(1);
    exec(&mut rig, "gnoblin.clients.get('3').moveWithKeys()");
    settle(&mut rig);
    for _ in 0..40 {
        key(&mut rig, "Left");
    }
    assert!(window(&mut rig, 3).0 >= 0.0, "the window does not leave the left edge");
    for _ in 0..80 {
        key(&mut rig, "Down");
    }
    let (_, y, _, h) = window(&mut rig, 3);
    assert!(y + h <= 800.0, "nor the bottom edge, got {y}+{h}");
    key(&mut rig, "Return");
    exec(&mut rig, "gnoblin.clients.get('3').resizeWithKeys()");
    settle(&mut rig);
    for _ in 0..40 {
        key(&mut rig, "Left");
        key(&mut rig, "Up");
    }
    let (_, _, w, h) = window(&mut rig, 3);
    assert_eq!((w, h), (120.0, 80.0), "the window keeps its least size");
    for _ in 0..200 {
        key(&mut rig, "Right");
    }
    let (x, _, w, _) = window(&mut rig, 3);
    assert!(x + w <= 1000.0, "and never grows past the work area");
}

#[test]
fn a_maximised_fullscreen_or_tiled_window_is_refused() {
    // Before: "a maximised, fullscreen or tiled window is refused".
    let mut rig = start(1);
    rig.comp.external_set(3, "maximised", MirrorValue::Bool(true));
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveWithKeys())"),
        "false",
        "a maximised window is not moved"
    );
    rig.comp.external_set(3, "maximised", MirrorValue::Bool(false));
    rig.comp.external_set(3, "fullscreen", MirrorValue::Bool(true));
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveWithKeys())"),
        "false",
        "nor a fullscreen one"
    );
    assert!(!running(&mut rig), "and no mode started");

    let mut tiled = world(1, false, "new Fill()", "");
    assert_eq!(tiled.eval("String(gnoblin.clients.get('3').tiled)"), "true");
    assert_eq!(
        tiled.eval("String(gnoblin.clients.get('3').resizeWithKeys())"),
        "false",
        "a tiled window is not resized"
    );
    assert!(!running(&mut tiled), "and no mode started");
}

#[test]
fn a_snapped_window_is_unsnapped_to_move_and_escape_snaps_it_back() {
    // Before: "a snapped window is unsnapped to move, and Escape snaps it back".
    let mut rig = start(1);
    let floating = window(&mut rig, 3);
    assert_eq!(
        rig.eval(
            "String(snapClient(gnoblin.clients.get('3'), 'left', { edgeTiling: true, area: { x: 0, y: 0, width: 1000, height: 800 } }))"
        ),
        "true"
    );
    settle(&mut rig);
    let snapped = window(&mut rig, 3);
    assert_eq!(snapped, (0.0, 0.0, 500.0, 800.0), "the window tiled to the left half");
    exec(&mut rig, "gnoblin.clients.get('3').moveWithKeys()");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(snappedZone(gnoblin.clients.get('3')))"),
        "null",
        "moving unsnaps"
    );
    assert_eq!(window(&mut rig, 3), floating, "the window is where it floated");
    key(&mut rig, "Escape");
    assert_eq!(
        rig.eval("String(snappedZone(gnoblin.clients.get('3')))"),
        "left",
        "Escape puts the snap back"
    );
    assert_eq!(window(&mut rig, 3), snapped, "and the snapped rectangle");
    exec(&mut rig, "unsnapClient(gnoblin.clients.get('3'))");
    settle(&mut rig);
    assert_eq!(
        window(&mut rig, 3),
        floating,
        "the floating rectangle survives the round trip"
    );
}

#[test]
fn the_mode_ends_when_the_window_goes_and_when_the_session_locks() {
    // Before: "the mode ends when the window goes and when the session locks".
    let mut rig = start(2);
    exec(&mut rig, "gnoblin.clients.get('3').moveWithKeys()");
    settle(&mut rig);
    let at = grab(&mut rig);
    rig.comp.destroy(3);
    settle(&mut rig);
    assert!(!running(&mut rig), "the window going ends it");
    assert!(rig.comp.canvases().into_iter().all(|id| id != at), "and its node goes");
    exec(&mut rig, "gnoblin.clients.get('4').moveWithKeys()");
    settle(&mut rig);
    assert!(running(&mut rig));
    rig.comp.external_set(LOCK, "state", text("pending"));
    settle(&mut rig);
    assert!(!running(&mut rig), "the lock ends it");
}

#[test]
fn only_one_window_is_repositioned_at_a_time_and_a_window_not_showing_is_refused() {
    // New: the retired shell `begin` ends a running mode first and refuses a window not on the desk's active workspace.
    let mut rig = start(2);
    exec(&mut rig, "gnoblin.clients.get('3').moveWithKeys()");
    settle(&mut rig);
    let first = grab(&mut rig);
    key(&mut rig, "Right");
    exec(&mut rig, "gnoblin.clients.get('4').resizeWithKeys()");
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(repositioning.current === gnoblin.clients.get('4'))"),
        "true"
    );
    assert!(
        rig.comp.canvases().into_iter().all(|id| id != first),
        "the first mode's node is gone"
    );
    assert_eq!(
        window(&mut rig, 3).0,
        10.0 + STEP,
        "and the first window keeps what it was given"
    );
    key(&mut rig, "Return");
    exec(
        &mut rig,
        "gnoblin.clients.get('3').moveToWorkspace(gnoblin.workspaces.get('2'))",
    );
    settle(&mut rig);
    assert_eq!(
        rig.eval("String(gnoblin.clients.get('3').moveWithKeys())"),
        "false",
        "a window on a workspace no output shows is refused"
    );
}

#[test]
fn the_window_menu_offers_move_and_resize_for_a_floating_window() {
    // New: the retired menu enabled both because `Window` had the methods; the JavaScript `Client` has them now.
    let mut rig = start(1);
    rig.comp.request(3, "menu", 100.0, 100.0);
    settle(&mut rig);
    assert_eq!(
        rig.eval("windowMenu.current.panel.rows.filter((r) => r.label === 'Move' || r.label === 'Resize').map((r) => r.enabled).join(',')"),
        "true,true"
    );
    exec(
        &mut rig,
        "windowMenu.current.activate(windowMenu.current.panel.rows.find((r) => r.label === 'Move'))",
    );
    settle(&mut rig);
    assert!(running(&mut rig), "the menu's Move starts the mode");
    assert_ne!(grab(&mut rig), 0);
    assert_eq!(rig.eval("String(windowMenu.current)"), "null");
}
