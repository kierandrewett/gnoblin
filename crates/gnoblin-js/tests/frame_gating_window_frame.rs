//! Change gating (`tests/frame_gating.rs`, `data/js/gnoblin-core/wake.ts`) for the window frame: a frame with nothing to do
//! runs no frame, and each thing the window frame changes still gets the very next one.
//!
//! The risk is a change that does not wake the loop and so is never drawn. Every case here goes quiet first (a frame that
//! finds nothing to do), changes one thing, and asserts that the next frame ran and that its effect is already there:
//! a frame mounted or unmounted, a titlebar's title, a canvas's `placement` written by script, a pointer press held on a
//! node of the shell until release, a resize handle's drag, and the close fade that the clock alone ends.

use std::time::Duration;

use gnoblin_js::JsFrameStats;
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::policy::*;
use common::*;

const LEFT: u64 = 272;

fn stats(rig: &Rig) -> JsFrameStats {
    rig.shell.frame_stats()
}

/// Runs frames until the shell is quiet: the frame after the last one that did work finds nothing and stops.
fn quiet(rig: &mut Rig) {
    for _ in 0..12 {
        let before = stats(rig).run;
        rig.frame();
        if stats(rig).run == before {
            return;
        }
    }
    panic!("the shell never went quiet: {:?}", stats(rig));
}

/// Whether the next frame did its work.
fn next_frame_runs(rig: &mut Rig) -> bool {
    let before = stats(rig).run;
    rig.frame();
    stats(rig).run > before
}

/// Goes quiet, makes `change`, and asserts that the next frame ran and `effect` holds after that one frame alone.
fn wakes(rig: &mut Rig, name: &str, change: impl FnOnce(&mut Rig), effect: impl FnOnce(&Rig) -> bool) {
    quiet(rig);
    change(rig);
    assert!(next_frame_runs(rig), "{name}: the next frame did not run");
    assert!(effect(rig), "{name}: the frame ran and its effect is not there");
}

/// A shell with two server-decorated windows (nodes 3 and 4) that have the stock titlebar.
fn stock() -> Rig {
    world(2, false, "new Layout()", "Decorations.get().start();")
}

/// A shell whose frames hold a drag area, a handle on the right side, and an underlay, kept by the window's title in
/// `globalThis.kept`: window 3 is `win1`, window 4 is `win2`.
fn custom() -> Rig {
    world(
        2,
        false,
        "new Layout()",
        "Decorations.get().frame = (client: any, frame: any) => {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            frame.right.size = 8;\n\
            const bar = new DragArea(frame, { grow: 1, direction: 'row', fill: '#333333' });\n\
            frame.top.add(bar);\n\
            const handle = new ResizeHandle(frame, 'right', { grow: 1 });\n\
            frame.right.add(handle);\n\
            frame.underlay.add(new Box({ x: 0, y: 0, width: 10, height: 10, fill: '#102030' }));\n\
            globalThis.kept ??= {};\n\
            globalThis.kept[client.title] = { bar: bar.node, handle: handle.node, under: frame.underlay };\n\
            return true;\n\
        };\n\
        Decorations.get().start();",
    )
}

/// The tree node of a part kept for the window titled `title`.
fn kept(rig: &mut Rig, title: &str, part: &str) -> u64 {
    rig.ids(&format!("[globalThis.kept['{title}'].{part}]"))[0]
}

fn content_width(rig: &Rig, node: u64) -> f64 {
    rig.comp.number(node, "width")
}

#[test]
#[ignore = "vrnw: needs Decorations to drop a window's frame when it becomes client-decorated"]
fn a_frame_that_mounts_unmounts_or_retitles_wakes_the_next_frame() {
    let mut rig = stock();
    assert_eq!(rig.comp.children(3).len(), 1, "the window starts with its frame");
    let frame = rig.comp.children(3)[0];
    wakes(
        &mut rig,
        "a window became client-decorated, so its frame went",
        |rig| rig.comp.external_set(3, "decoration", text("client")),
        |rig| rig.comp.children(3).is_empty() && rig.comp.destroys_of(frame) == 1,
    );
    wakes(
        &mut rig,
        "a window became server-decorated, so a frame came",
        |rig| rig.comp.external_set(3, "decoration", text("server")),
        |rig| rig.comp.children(3).len() == 1,
    );
    wakes(
        &mut rig,
        "a titlebar's title changed",
        |rig| rig.comp.external_set(4, "title", text("renamed")),
        |rig| !rig.comp.canvases_showing("renamed").is_empty() && rig.comp.canvases_showing("win2").is_empty(),
    );
    wakes(
        &mut rig,
        "a window with a frame went",
        |rig| rig.comp.destroy(4),
        |_| true,
    );
    wakes(
        &mut rig,
        "a window with a frame appeared",
        |rig| rig.comp.create_window(9, "fresh", "foot"),
        |rig| rig.comp.children(9).len() == 1,
    );
    quiet(&mut rig);
    assert!(
        !rig.comp.canvases_showing("fresh").is_empty(),
        "and the title is painted once the frames after it have run"
    );
}

#[test]
#[ignore = "vrnw: needs a write made by a gnoblin.timers.after callback to be sent by the frame it runs in (placement stays `frame`, not `under`)"]
fn a_canvas_whose_placement_script_wrote_is_sent_on_the_next_frame() {
    let mut rig = custom();
    let under = kept(&mut rig, "win1", "under.node");
    assert_eq!(rig.comp.value(under, "placement"), Some(text("under")));
    wakes(
        &mut rig,
        "script moved a layer over the window",
        |rig| exec(rig, "globalThis.kept.win1.under.placement = 'over'"),
        |rig| rig.comp.value(under, "placement") == Some(text("over")),
    );
    wakes(
        &mut rig,
        "script put the layer in the frame's shape",
        |rig| exec(rig, "globalThis.kept.win1.under.placement = 'frame'"),
        |rig| rig.comp.value(under, "placement") == Some(text("frame")),
    );
    // A write from a listener, which is a frame itself, is sent by that frame.
    wakes(
        &mut rig,
        "a timer wrote it",
        |rig| {
            exec(
                rig,
                "gnoblin.timers.after(50, () => { globalThis.kept.win1.under.placement = 'under'; })",
            )
        },
        |_| true,
    );
    rig.advance(50);
    assert!(next_frame_runs(&mut rig), "the timer's frame ran");
    assert_eq!(
        rig.comp.value(under, "placement"),
        Some(text("under")),
        "and sent the write it made"
    );
}

#[test]
fn a_press_that_holds_a_node_wakes_a_frame_for_each_motion_and_the_release() {
    let mut rig = custom();
    let bar = kept(&mut rig, "win1", "bar");
    let start = (rig.comp.number(3, "x"), rig.comp.number(3, "y"));
    wakes(
        &mut rig,
        "a press on the drag area",
        |rig| rig.comp.pointer(bar, "press", 20.0, 10.0, LEFT),
        |_| true,
    );
    // The press is held and the pointer leaves: nothing arrives until the compositor tells the node of the motion, and
    // the shell does no work meanwhile.
    quiet(&mut rig);
    let before = stats(&rig);
    for _ in 0..20 {
        rig.frame();
    }
    assert_eq!(
        stats(&rig).run,
        before.run,
        "a held press keeps no frame running by itself"
    );
    assert_eq!(rig.shell.next_wake(), None, "and asks for no wake-up");
    wakes(
        &mut rig,
        "the pointer moved far from the node while the press was held",
        |rig| rig.comp.pointer(bar, "motion", 420.0, 210.0, 0),
        |rig| (rig.comp.number(3, "x") - start.0, rig.comp.number(3, "y") - start.1) == (400.0, 200.0),
    );
    wakes(
        &mut rig,
        "the press was released",
        |rig| rig.comp.pointer(bar, "release", 420.0, 210.0, LEFT),
        |_| true,
    );
    quiet(&mut rig);
    let held = (rig.comp.number(3, "x"), rig.comp.number(3, "y"));
    rig.comp.pointer(bar, "motion", 500.0, 300.0, 0);
    rig.frame();
    assert_eq!(
        (rig.comp.number(3, "x"), rig.comp.number(3, "y")),
        held,
        "after the release a motion no longer drags the window"
    );
}

#[test]
fn a_resize_handle_drag_wakes_a_frame_for_each_step() {
    let mut rig = custom();
    let handle = kept(&mut rig, "win1", "handle");
    let width = content_width(&rig, 3);
    wakes(
        &mut rig,
        "a press on the handle",
        |rig| rig.comp.pointer(handle, "press", 0.0, 50.0, LEFT),
        |_| true,
    );
    wakes(
        &mut rig,
        "the handle was dragged",
        |rig| rig.comp.pointer(handle, "motion", 25.0, 50.0, 0),
        |rig| content_width(rig, 3) == width + 25.0,
    );
    wakes(
        &mut rig,
        "the handle was dragged on: it moved with the window, so the pointer 100 more to the right is at 100 in it",
        |rig| rig.comp.pointer(handle, "motion", 100.0, 50.0, 0),
        |rig| content_width(rig, 3) == width + 125.0,
    );
    wakes(
        &mut rig,
        "the handle was released",
        |rig| rig.comp.pointer(handle, "release", 100.0, 50.0, LEFT),
        |_| true,
    );
}

/// The close button of the stock titlebar of window `window`.
fn close_button(rig: &Rig, window: u64) -> u64 {
    let bar = titlebar(rig, window);
    *rig.comp.children(bar).last().expect("the bar has buttons")
}

#[test]
#[ignore = "vrnw: needs the Decorations close fade to register a wake with the shell (next_wake is None)"]
fn the_close_fade_ends_on_the_clock_and_a_window_that_stayed_comes_back_on_it() {
    let mut rig = stock();
    quiet(&mut rig);
    rig.comp.pointer(close_button(&rig, 3), "press", 4.0, 4.0, LEFT);
    quiet(&mut rig);
    assert!(
        rig.comp.calls().is_empty(),
        "the window fades before it is asked to close"
    );
    let wait = rig.shell.next_wake().expect("the fade is waiting on the clock");
    assert!(
        wait > Duration::from_millis(170) && wait <= Duration::from_millis(180),
        "{wait:?}"
    );
    rig.advance(179);
    assert!(!next_frame_runs(&mut rig), "the fade is not over yet");
    rig.advance(1);
    assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO), "now it is due");
    assert!(next_frame_runs(&mut rig), "the fade's end ran a frame");
    assert_eq!(
        rig.comp.calls(),
        [(3, "close".to_owned())],
        "and asked the window to close"
    );
    quiet(&mut rig);
    let wait = rig.shell.next_wake().expect("a window that stays is waited for");
    assert!(
        wait > Duration::from_millis(1490) && wait <= Duration::from_millis(1500),
        "{wait:?}"
    );
    let timed = rig.comp.timed_sets();
    rig.advance(1500);
    assert!(
        next_frame_runs(&mut rig),
        "the time to bring the window back ran a frame"
    );
    quiet(&mut rig);
    assert!(rig.comp.timed_sets() > timed, "the window fades back in");
    assert_eq!(rig.shell.next_wake(), None, "and nothing is left to wait for");
}

#[test]
#[ignore = "vrnw: needs the Decorations close fade to register a wake with the shell (next_wake is None)"]
fn a_close_under_reduced_motion_still_waits_to_bring_a_window_back_that_stayed() {
    let mut rig = stock();
    rig.comp.external_set(SEAT, "reduced_motion", MirrorValue::Bool(true));
    quiet(&mut rig);
    rig.comp.pointer(close_button(&rig, 3), "press", 4.0, 4.0, LEFT);
    quiet(&mut rig);
    assert_eq!(rig.comp.calls(), [(3, "close".to_owned())], "asked at once");
    let wait = rig.shell.next_wake().expect("a window that stays is waited for");
    assert!(
        wait > Duration::from_millis(1490) && wait <= Duration::from_millis(1500),
        "{wait:?}"
    );
    rig.advance(1500);
    assert!(
        next_frame_runs(&mut rig),
        "the window that stayed is brought back by a frame"
    );
    quiet(&mut rig);
    assert_eq!(rig.shell.next_wake(), None);
}

#[test]
fn an_idle_shell_with_frames_runs_no_frame() {
    for windows in [0, 25, 100] {
        let mut rig = world(windows, false, "new Layout()", "Decorations.get().start();");
        quiet(&mut rig);
        let before = stats(&rig);
        for _ in 0..50 {
            rig.frame();
        }
        let after = stats(&rig);
        assert_eq!(after.run, before.run, "{windows} windows with frames: no frame ran");
        assert_eq!(
            after.idle,
            before.idle + 50,
            "{windows} windows: every turn was found quiet"
        );
        assert_eq!(rig.shell.next_wake(), None, "{windows} windows: nothing is due");
    }
}
