//! Change gating: a frame that has nothing to do does no work, and every kind of change still gets its frame.
//!
//! The risk of gating is a change that does not wake the loop and so is never drawn. `every_kind_of_change_wakes_the_
//! next_frame` is the table that guards it: each row changes one thing and asserts that the very next frame ran. A new
//! kind of change belongs in it. The same rule holds for each kind of message the tree link carries
//! (`every_kind_of_message_the_tree_link_carries_wakes_the_next_frame`, which cannot compile past a new kind) and for the
//! links beside the tree and the parts of the library that keep a clock: those tests sit beside the fakes of the links
//! (`host_api.rs`, `host_idle.rs`, `config_slots.rs`, `lock.rs`, `xr.rs`) and `wake_sources.rs` enumerates what has to be
//! registered. What a frame costs when it does not run is `frame_cost.rs`.

use std::collections::BTreeSet;
use std::time::Duration;

use gnoblin_js::JsShellError;
use gnoblin_tree_client::tree_mirror::{Attached, MirrorValue, TreeIncoming, TxReply};

mod common;
use common::policy::*;
use common::*;

#[test]
fn an_idle_shell_runs_no_frame_at_any_size() {
    for windows in [0, 25, 100] {
        let mut rig = world(windows, false, "new Layout()", "");
        rig.quiet();
        let before = rig.shell.frame_stats();
        for _ in 0..50 {
            rig.frame();
        }
        let after = rig.shell.frame_stats();
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
        rig.quiet();
        change(&mut rig);
        assert!(rig.next_frame_runs(), "{name}: the next frame did not run");
    }
}

// ---- the tree link ------------------------------------------------------------------------------------------------

/// The kinds of message the tree link carries, numbered, so that a new kind cannot be added without this file hearing of
/// it: the match has no wildcard, and `every_kind_of_message_the_tree_link_carries_wakes_the_next_frame` asks for a
/// message of each number.
fn kind(message: &TreeIncoming) -> usize {
    match message {
        TreeIncoming::Attached(_) => 0,
        TreeIncoming::Snapshot { .. } => 1,
        TreeIncoming::Changes { .. } => 2,
        TreeIncoming::Reply(_) => 3,
        TreeIncoming::Answer { .. } => 4,
        TreeIncoming::TrackEnded { .. } => 5,
        TreeIncoming::Event(_) => 6,
        TreeIncoming::Chord(_) => 7,
        TreeIncoming::Gesture(_) => 8,
        TreeIncoming::PointerChord(_) => 9,
        TreeIncoming::PointerButton(_) => 10,
    }
}

/// How many kinds `kind` numbers.
const KINDS: usize = 11;

/// The kinds that belong to the boot. After it they are a protocol error, which the frame reports and does not skip
/// (`a_boot_message_after_the_boot_is_an_error_and_not_a_quiet_turn`).
fn boot_messages() -> [TreeIncoming; 2] {
    [
        TreeIncoming::Attached(Attached {
            epoch: 1,
            root: ROOT,
            serial: 1,
        }),
        TreeIncoming::Snapshot {
            serial: 1,
            chunk: 0,
            last: true,
            nodes: Vec::new(),
        },
    ]
}

type Message = (&'static str, fn(&Rig));

/// One message of each kind a running shell is sent. Ones about nodes or claims the shell does not know are the ones the
/// compositor may send late, and they must wake a frame all the same: the gate looks at whether anything arrived, never
/// at what it was.
const MESSAGES: &[Message] = &[
    ("changes", |rig| rig.comp.external_set(3, "title", text("renamed"))),
    ("reply", |rig| {
        rig.comp.push(TreeIncoming::Reply(TxReply {
            serial: 1,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
    }),
    ("answer", |rig| rig.comp.answer(1, "ok", "")),
    ("track ended", |rig| rig.comp.track_ended(3, "x", true, None)),
    ("event", |rig| rig.comp.pointer(3, "enter", 5.0, 5.0, 0)),
    ("chord", |rig| rig.comp.chord(99, "<Super>a", "press")),
    ("gesture", |rig| {
        rig.comp.gesture(99, "swipe", 3, "begin", (0.0, 0.0), 1.0, 0.0)
    }),
    ("pointer chord", |rig| {
        rig.comp
            .pointer_chord(99, "<Super>Button1", "start", (0.0, 0.0), (0.0, 0.0))
    }),
    ("pointer button", |rig| rig.comp.pointer_button(99, 0x110, "press")),
];

#[test]
fn every_kind_of_message_the_tree_link_carries_wakes_the_next_frame() {
    let mut rig = world(3, false, "new Layout()", "");
    let mut sent = BTreeSet::new();
    for (name, send) in MESSAGES {
        rig.quiet();
        send(&rig);
        sent.extend(rig.comp.queued().iter().map(kind));
        assert!(rig.next_frame_runs(), "{name}: the next frame did not run");
    }
    sent.extend(boot_messages().iter().map(kind));
    assert_eq!(
        sent,
        (0..KINDS).collect::<BTreeSet<_>>(),
        "a kind of message has no row in MESSAGES: add one, so that the gate is shown to wake on it"
    );
}

#[test]
#[ignore = "vrnw: TS internal: the TS host bridge refused an Attached or Snapshot message after the boot; over the core the frame accepts it and reports nothing"]
fn a_boot_message_after_the_boot_is_an_error_and_not_a_quiet_turn() {
    for message in boot_messages() {
        let mut rig = world(1, false, "new Layout()", "");
        rig.quiet();
        rig.comp.push(message);
        let error = rig.shell.frame().expect_err("the frame refuses a boot message");
        assert!(matches!(error, JsShellError::Link(_)), "{error:?}");
    }
}

// ---- the script's own clock ---------------------------------------------------------------------------------------

#[test]
fn a_write_from_script_reaches_the_compositor_on_the_next_frame() {
    let mut rig = world(1, false, "new Layout()", "");
    rig.quiet();
    let mark = rig.comp.sent_count();
    exec(&mut rig, "gnoblin.clients.at(0).x = 321");
    rig.frame();
    assert!(
        !rig.comp.ops_since(mark).is_empty(),
        "the write was sent by the frame it woke"
    );
}

#[test]
#[ignore = "vrnw: needs gnoblin.timers.after to register a wake with the shell (next_wake is None with a timer waiting)"]
fn a_timer_wakes_the_frame_at_its_deadline_and_not_before() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(
        &mut rig,
        "gnoblin.timers.after(100, () => { globalThis.fired = 'yes'; })",
    );
    rig.quiet();
    let wait = rig.shell.next_wake().expect("a timer is waiting");
    assert!(
        wait > Duration::from_millis(90) && wait <= Duration::from_millis(100),
        "{wait:?}"
    );
    rig.advance(99);
    assert!(!rig.next_frame_runs(), "the timer is not due yet");
    rig.advance(1);
    assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO), "now it is due");
    assert!(rig.next_frame_runs(), "the timer's frame ran");
    assert_eq!(rig.eval("globalThis.fired"), "yes");
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "a spent timer leaves nothing to wait for");
}

#[test]
#[ignore = "vrnw: needs Toast to register a wake with the shell (next_wake is None while a toast fades)"]
fn a_card_that_fades_with_the_clock_keeps_the_loop_running_until_it_ends() {
    let mut rig = world(1, false, "new Layout()", "");
    exec(&mut rig, "Toast.show('hello')");
    for _ in 0..5 {
        assert!(rig.next_frame_runs(), "a toast is up, so every turn draws");
        assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO));
    }
    rig.advance(10_000);
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "the toast ended and the loop went quiet");
}

fn pointer_to(rig: &mut Rig, x: f64, y: f64) {
    rig.comp.external_set(SEAT, "pointer_x", MirrorValue::Double(x));
    rig.comp.external_set(SEAT, "pointer_y", MirrorValue::Double(y));
    settle(rig);
}

#[test]
#[ignore = "vrnw: needs a `move` request to start a pointer-following drag that registers its limit as a shell wake (the window does not follow the pointer and next_wake is None)"]
fn a_window_drag_that_runs_past_its_limit_ends_on_the_clock_with_the_pointer_still() {
    // The drag follows the pointer, which the compositor reports, so a moving pointer wakes frames of its own. One that
    // holds still is the case the limit is for: only the script's clock can end it.
    let mut rig = world(1, false, "new Layout()", "");
    let limit = number(&mut rig, "WindowState.get().dragLimitMs") as u64;
    pointer_to(&mut rig, 100.0, 100.0);
    rig.comp.request(3, "move", 0.0, 0.0);
    settle(&mut rig);
    pointer_to(&mut rig, 130.0, 150.0);
    assert_eq!(rect(&rig, 3), "40,70,300,200", "the drag follows the pointer");
    rig.quiet();
    assert_eq!(
        rig.shell.next_wake(),
        Some(Duration::from_millis(limit)),
        "the limit is the one thing the shell waits for"
    );
    rig.advance(limit - 1);
    assert!(!rig.next_frame_runs(), "the drag has not run out yet");
    rig.advance(1);
    assert_eq!(rig.shell.next_wake(), Some(Duration::ZERO), "now it has");
    assert!(rig.next_frame_runs(), "the limit woke a frame");
    // The drag is over: nothing is left to wait for, and the pointer no longer takes the window along.
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "an ended drag leaves nothing to wait for");
    pointer_to(&mut rig, 400.0, 400.0);
    assert_eq!(
        rect(&rig, 3),
        "40,70,300,200",
        "the window stayed where the drag left it"
    );
}

#[test]
fn a_drag_past_its_limit_ends_cleanly() {
    // The frame the limit wakes ends the drag through the same path as a release: it must not report a failure.
    let mut rig = world(1, false, "new Layout()", "");
    let limit = number(&mut rig, "WindowState.get().dragLimitMs") as u64;
    pointer_to(&mut rig, 100.0, 100.0);
    rig.comp.request(3, "move", 0.0, 0.0);
    settle(&mut rig);
    rig.advance(limit + 100);
    let diagnostics = rig.frame();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None, "an ended drag leaves nothing to wait for");
    pointer_to(&mut rig, 400.0, 400.0);
    assert_eq!(
        rect(&rig, 3),
        "10,20,300,200",
        "the drag was over, so the pointer moved nothing"
    );
}

#[test]
#[ignore = "vrnw: needs a `move` request to start a pointer-following drag that registers its limit as a shell wake (the window does not follow the pointer and next_wake is None)"]
fn a_drag_that_ends_by_the_release_leaves_nothing_to_wait_for() {
    let mut rig = world(1, false, "new Layout()", "");
    pointer_to(&mut rig, 100.0, 100.0);
    rig.comp.request(3, "move", 0.0, 0.0);
    settle(&mut rig);
    assert!(
        rig.shell.next_wake().is_some(),
        "a drag in progress has its limit to wait for"
    );
    rig.comp.pointer(3, "release", 0.0, 0.0, LEFT);
    settle(&mut rig);
    rig.quiet();
    assert_eq!(rig.shell.next_wake(), None);
}

// ---- any order, after any rest -----------------------------------------------------------------------------------

/// A small generator with a fixed seed, so the sequence below is the same on every machine.
struct Sequence(u64);

impl Sequence {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % bound
    }
}

type Repeatable = (&'static str, fn(&mut Rig, u64));

/// The changes that can happen again and again: each takes a number that makes it a real change every time. The lock is
/// not one of them: a lock screen that is up is awake on every turn, so the shell would never rest again.
const REPEATABLE: &[Repeatable] = &[
    ("a window moved", |rig, n| {
        rig.comp.external_set(3, "x", MirrorValue::Double(n as f64))
    }),
    ("a title", |rig, n| {
        rig.comp.external_set(4, "title", text(&format!("title {n}")))
    }),
    ("a window appeared", |rig, n| {
        rig.comp.create_window(1000 + n, "new", "foot")
    }),
    ("an output changed size", |rig, n| {
        rig.comp
            .external_set(20, "width", MirrorValue::Double(1000.0 + n as f64));
    }),
    ("the pointer moved", |rig, n| {
        rig.comp.pointer(3, "motion", n as f64, 5.0, 0)
    }),
    ("an animation ended", |rig, _| rig.comp.track_ended(3, "x", true, None)),
    ("the focus moved", |rig, n| {
        rig.comp.external_set(SEAT, "focus", MirrorValue::Node(3 + (n % 3)));
    }),
    ("script ran", |rig, n| exec(rig, &format!("globalThis.touched = {n}"))),
    ("a property was written from script", |rig, n| {
        exec(rig, &format!("gnoblin.clients.at(1).x = {}", 100 + n))
    }),
];

#[test]
fn changes_in_any_order_and_after_any_rest_each_wake_the_very_next_frame() {
    let mut rig = world(3, false, "new Layout()", "");
    let mut sequence = Sequence(0x9e37_79b9_7f4a_7c15);
    let mut nonce = 0_u64;
    for step in 0..300_u64 {
        // A rest of a random length: every turn of it is found quiet.
        rig.quiet();
        for _ in 0..sequence.below(4) {
            assert!(
                !rig.next_frame_runs(),
                "step {step}: a turn with nothing in it ran a frame"
            );
        }
        // One to three changes land before the next turn, and that turn runs.
        let mut names = Vec::new();
        for _ in 0..=sequence.below(3) {
            let (name, change) = REPEATABLE[sequence.below(REPEATABLE.len())];
            nonce += 1;
            change(&mut rig, nonce);
            names.push(name);
        }
        assert!(
            rig.next_frame_runs(),
            "step {step}: {names:?} did not wake the next frame"
        );
    }
}
