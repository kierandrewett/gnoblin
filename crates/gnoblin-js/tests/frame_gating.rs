//! Change gating: a frame that has nothing to do does no work, and every kind of change still gets its frame.
//!
//! The risk of gating is a change that does not wake the loop and so is never drawn. `every_kind_of_change_wakes_the_
//! next_frame` is the table that guards it: each row changes one thing and asserts that the very next frame ran. A new
//! kind of change belongs in it. What a frame costs when it does not run is `frame_cost.rs`.

use std::time::Duration;

use gnoblin_js::JsFrameStats;
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::policy::*;
use common::*;

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

#[test]
fn an_idle_shell_runs_no_frame_at_any_size() {
    for windows in [0, 25, 100] {
        let mut rig = world(windows, false, "new Layout()", "");
        quiet(&mut rig);
        let before = stats(&rig);
        for _ in 0..50 {
            rig.frame();
        }
        let after = stats(&rig);
        assert_eq!(after.run, before.run, "{windows} windows: no frame ran");
        assert_eq!(
            after.idle,
            before.idle + 50,
            "{windows} windows: every turn was found quiet"
        );
        assert_eq!(rig.shell.next_wake(), None, "{windows} windows: nothing is due");
    }
}

type Change = (&'static str, fn(&mut Rig));

const CHANGES: &[Change] = &[
    ("a window moved", |rig| {
        rig.comp.external_set(3, "x", MirrorValue::Double(500.0))
    }),
    ("a client committed a title", |rig| {
        rig.comp.external_set(3, "title", text("renamed"))
    }),
    ("a window appeared", |rig| rig.comp.create_window(100, "new", "foot")),
    ("a window went", |rig| rig.comp.destroy(4)),
    ("an output changed size", |rig| {
        rig.comp.external_set(20, "width", MirrorValue::Double(1200.0));
    }),
    ("the pointer entered a window", |rig| {
        rig.comp.pointer(3, "enter", 5.0, 5.0, 0)
    }),
    ("an animation ended", |rig| rig.comp.track_ended(3, "x", true, None)),
    ("the seat's keyboard focus moved", |rig| {
        rig.comp.external_set(SEAT, "focus", MirrorValue::Node(3));
    }),
    ("script ran outside a frame", |rig| exec(rig, "globalThis.touched = 1")),
    ("a property was written from script", |rig| {
        exec(rig, "gnoblin.clients.at(0).x = 321")
    }),
    ("the session lock changed state", |rig| {
        rig.comp.external_set(LOCK, "state", text("pending"))
    }),
];

#[test]
fn every_kind_of_change_wakes_the_next_frame() {
    let mut rig = world(3, false, "new Layout()", "");
    for (name, change) in CHANGES {
        quiet(&mut rig);
        change(&mut rig);
        assert!(next_frame_runs(&mut rig), "{name}: the next frame did not run");
    }
}

#[test]
fn a_write_from_script_reaches_the_compositor_on_the_next_frame() {
    let mut rig = world(1, false, "new Layout()", "");
    quiet(&mut rig);
    let mark = rig.comp.sent_count();
    exec(&mut rig, "gnoblin.clients.at(0).x = 321");
    rig.frame();
    assert!(
        !rig.comp.ops_since(mark).is_empty(),
        "the write was sent by the frame it woke"
    );
}

#[test]
fn a_timer_wakes_the_frame_at_its_deadline_and_not_before() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.timers.after(100, () => { globalThis.fired = 'yes'; })",
    );
    quiet(&mut rig);
    let wait = rig.shell.next_wake().expect("a timer is waiting");
    assert!(
        wait > Duration::from_millis(90) && wait <= Duration::from_millis(100),
        "{wait:?}"
    );
    rig.advance(99);
    assert!(!next_frame_runs(&mut rig), "the timer is not due yet");
    rig.advance(1);
    assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO), "now it is due");
    assert!(next_frame_runs(&mut rig), "the timer's frame ran");
    assert_eq!(rig.eval("globalThis.fired"), "yes");
    quiet(&mut rig);
    assert_eq!(rig.shell.next_wake(), None, "a spent timer leaves nothing to wait for");
}

#[test]
fn a_card_that_fades_with_the_clock_keeps_the_loop_running_until_it_ends() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "toast('hello')");
    for _ in 0..5 {
        assert!(next_frame_runs(&mut rig), "a toast is up, so every turn draws");
        assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO));
    }
    rig.advance(10_000);
    quiet(&mut rig);
    assert_eq!(rig.shell.next_wake(), None, "the toast ended and the loop went quiet");
}
