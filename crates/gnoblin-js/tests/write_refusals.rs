//! The shell's writes under the compositor's refusals, over the shared fake compositor of `tests/common`, which can
//! answer a transaction `stale-base` (at an op, or with no op index when the shell's base is older than the history the
//! compositor keeps) and `unknown-node` as the real one does (`gnoblin_render::tree_revision`).
//!
//! A refused transaction applies nothing, so a write the shell sent in it is lost unless the shell sends it again. Each
//! test is a way a write was lost, and what it asserts is that the write reaches the compositor anyway: a text call
//! (`Output.apply`), the whole frame behind a refusal that names no op, a placement the layout has no reason to compute
//! again, and the password check of the lock screen.

use gnoblin_js::STALE_RETRIES;
use gnoblin_paint::{PaintOp, decode};
use gnoblin_tree_client::tree_mirror::{MirrorChange, MirrorChangeKind, MirrorValue, TxOp};

mod common;
use common::*;

const IMPORTS: &str = "import * as libCore from \"gnoblin-core\"; import * as libStd from \"gnoblin-std\"; const lib = { ...libCore, ...libStd };\nimport { PasswordEntry } from \"gnoblin-std/lock\";\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n(globalThis as any).PasswordEntry = PasswordEntry;\n";

const SEAT: u64 = 21;
const A: u64 = 20;
const B: u64 = 22;

/// A layout that gives every window the whole area it is offered.
const FILL: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
}\n";

/// Runs a statement in the script. `Rig::eval` wants a value back; a statement has none.
fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

fn settle(rig: &mut Rig) {
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// A shell over two screens, `A` (1000 by 800, primary) and `B` (600 by 400), and the workspace `1` with `layout`.
fn world(windows: u64, layout: &str) -> Rig {
    let init = format!("{FILL}gnoblin.workspaces.add(new Workspace({{ id: \"1\", layout: {layout} }}));\n");
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_screen(A, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
        comp.add_screen(B, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
        comp.add_node(
            SEAT,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
        for i in 1..=windows {
            comp.add_window(2 + i, &format!("win{i}"), "foot");
        }
    });
    settle(&mut rig);
    rig
}

// ---- a call whose text is content --------------------------------------------------------------------------------

#[test]
fn an_output_apply_refused_stale_base_is_sent_again_and_its_answer_settles_the_operation() {
    let mut rig = world(0, "new Layout()");
    // The compositor writes the screen just before the shell's transaction lands: stale at the apply, which is op 0.
    rig.comp.stale(1, None, B, "name");
    exec(
        &mut rig,
        "g.app = gnoblin.outputs.at(1).apply({ mode: '1920x1080@60', scale: 1.25 });",
    );
    let told = rig.settle();
    assert_eq!(rig.comp.refused(), 1, "the compositor refused the transaction once");
    let sent = rig.comp.applies();
    assert_eq!(
        sent.len(),
        2,
        "the apply went out, was refused and went out again: {sent:?}; told {told:?}"
    );
    assert_eq!(sent[0], sent[1], "the same request under the same token");
    assert!(
        told.iter().all(|line| !line.contains("dropped")),
        "nothing was dropped: {told:?}"
    );

    rig.comp.answer(sent[1].2, "ok", "");
    settle(&mut rig);
    assert_eq!(
        rig.eval("`${g.app.status} ${g.app.value === gnoblin.outputs.at(1)}`"),
        "succeeded true",
        "the operation hears the compositor's answer"
    );
}

#[test]
fn an_output_apply_on_a_screen_that_is_gone_is_answered_with_a_refusal_and_not_left_waiting() {
    let mut rig = world(0, "new Layout()");
    // The screen is unplugged while the apply is on its way: the compositor refuses `unknown-node`, and the node is
    // gone for good, so the call cannot be sent again. The script is told, instead of waiting for an answer for ever.
    rig.comp.fault(Fault::UnknownNode {
        failed_op: 0,
        destroyed: B,
    });
    exec(&mut rig, "g.app = gnoblin.outputs.at(1).apply({ scale: 1.25 });");
    rig.settle();
    assert_eq!(
        rig.comp.applies().len(),
        1,
        "a call on a node that is gone is not sent again"
    );
    assert_eq!(
        rig.eval("g.app.status"),
        "failed",
        "the operation is settled, and says it failed"
    );
    assert_eq!(rig.eval("g.app.error.code"), "not_found", "because the screen is gone");
}

#[test]
fn an_apply_whose_screen_is_destroyed_while_it_waits_to_go_again_is_answered_failed() {
    let mut rig = world(0, "new Layout()");
    // The racing batch that makes the base stale is the screen's destruction: the refusal is `stale-base`, and by the
    // time the apply could go again its node is gone, so it is dropped on the way out and its caller is told.
    rig.comp.fault(Fault::StaleBase {
        failed_op: None,
        racing: vec![MirrorChange {
            removed: vec![B],
            ..change(MirrorChangeKind::Destroyed, B)
        }],
    });
    exec(&mut rig, "g.app = gnoblin.outputs.at(1).apply({ scale: 1.25 });");
    rig.settle();
    assert_eq!(rig.comp.applies().len(), 1, "the apply was never sent a second time");
    assert_eq!(
        rig.eval("g.app.status"),
        "failed",
        "the operation is settled, and says it failed"
    );
    assert_eq!(rig.eval("g.app.error.code"), "not_found", "because the screen is gone");
}

// ---- a refusal that names no op ----------------------------------------------------------------------------------

#[test]
fn a_refusal_with_no_op_index_sends_the_whole_frame_again_on_the_new_base() {
    let mut rig = world(2, "new Layout()");
    // The shell's base is older than the history the compositor keeps: `stale-base` with `failed_op` -1.
    rig.comp.stale(1, Some(-1), 3, "title");
    let mark = rig.comp.sent_count();
    exec(
        &mut rig,
        "gnoblin.clients.get('3').x = 7; gnoblin.clients.get('4').y = 8;",
    );
    let told = rig.settle();
    assert_eq!(rig.comp.refused(), 1);
    assert_eq!(
        rig.comp.number(3, "x"),
        7.0,
        "the first write landed after all: {told:?}"
    );
    assert_eq!(rig.comp.number(4, "y"), 8.0, "and so did the second");
    let attempts: Vec<Vec<TxOp>> = rig.comp.sent_since(mark);
    assert_eq!(
        attempts.len(),
        2,
        "one refused attempt and one that landed: {attempts:?}"
    );
    assert_eq!(attempts[0], attempts[1], "the whole frame is sent again, op for op");
    assert!(told.iter().all(|line| !line.contains("op not recorded")), "{told:?}");
}

// ---- a placement the layout has no reason to compute again -------------------------------------------------------

#[test]
fn a_placement_dropped_after_its_resends_is_laid_out_again() {
    let mut rig = world(0, "new Fill()");
    // The compositor writes the new window before every landing of the shell's placement, one refusal more than the
    // shell resends for: the write is dropped, and nothing marks the workspace for a new layout.
    rig.comp.stale(STALE_RETRIES as usize + 1, None, 5, "title");
    rig.comp.create_window(5, "late", "foot");
    for _ in 0..12 {
        rig.frame();
    }
    assert_eq!(
        rig.comp.refused(),
        STALE_RETRIES as usize + 1,
        "every attempt of the placement was refused"
    );
    assert_eq!(
        rig.comp.rect(5),
        "0,0,1000,800",
        "the layout wrote the window's place again once the refusals stopped"
    );
}

// ---- the lock screen's password check ----------------------------------------------------------------------------

const LOCK: u64 = 2;
const WINDOW: u64 = 5;

/// A shell on a locked session with its lock screen up: a lock, a seat that focuses a window, and a screen.
fn locked() -> Rig {
    let mut rig = rig_setup(IMPORTS, "", |comp| {
        comp.add_node(LOCK, ROOT, "lock", vec![("state", text("locked"))]);
        comp.add_node(SEAT, ROOT, "seat", vec![("focus", MirrorValue::Node(WINDOW))]);
        comp.add_window(WINDOW, "Editor", "foot");
        comp.add_screen(A, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
    });
    settle(&mut rig);
    rig
}

/// The lock screen's entry and message canvases: the scrim under the lock, the card, then clock, entry and message.
fn entry_and_message(rig: &Rig) -> (u64, u64) {
    let scrim = *rig.comp.children(LOCK).first().expect("the lock screen is up");
    let card = *rig.comp.children(scrim).first().expect("the card");
    let parts = rig.comp.children(card);
    let [_clock, entry, message] = parts[..] else {
        panic!("the card holds a clock, an entry and a message: {parts:?}");
    };
    (entry, message)
}

fn message_of(rig: &Rig) -> String {
    let (_, message) = entry_and_message(rig);
    let bytes = rig.comp.content(message).expect("the message is painted");
    decode(&bytes)
        .expect("a paint list")
        .ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn type_and_enter(rig: &mut Rig, password: &str) {
    let (entry, _) = entry_and_message(rig);
    for character in password.chars() {
        let name = character.to_string();
        rig.comp.key(entry, &name, &name, true);
        rig.comp.key(entry, &name, "", false);
    }
    rig.comp.key(entry, "Return", "", true);
    rig.comp.key(entry, "Return", "", false);
}

#[test]
fn a_password_check_a_stale_refusal_dropped_is_answered_and_the_screen_takes_another_try() {
    let mut rig = locked();
    assert!(
        message_of(&rig).starts_with("Enter your password"),
        "{:?}",
        message_of(&rig)
    );

    // The compositor writes the lock node just as Enter is pressed: the transaction that carries the check is refused
    // `stale-base`. Its text is a password, which is not kept for a resend, so the check is lost.
    rig.comp.stale(1, None, LOCK, "note");
    type_and_enter(&mut rig, "hunter2");
    rig.settle();
    assert_eq!(rig.comp.refused(), 1, "the transaction with the check was refused");
    assert_eq!(rig.comp.checks().len(), 1, "the check went out once and was refused");
    assert!(
        !rig.comp.holds("hunter2"),
        "the password is nowhere in the compositor's tree"
    );
    assert_ne!(
        message_of(&rig),
        "Checking...",
        "the screen is not left waiting for an answer that cannot come"
    );

    // And Enter is an attempt again: the screen is not stuck on the check it lost.
    type_and_enter(&mut rig, "hunter2");
    rig.settle();
    assert_eq!(rig.comp.checks().len(), 2, "a second attempt reaches the compositor");
}

#[test]
fn a_check_the_compositor_never_answers_fails_once_its_deadline_has_passed_and_the_screen_takes_another() {
    let mut rig = locked();
    type_and_enter(&mut rig, "hunter2");
    rig.settle();
    assert_eq!(message_of(&rig), "Checking...", "the compositor has not answered");
    rig.advance(29_000);
    rig.settle();
    assert_eq!(message_of(&rig), "Checking...", "the check is still within its time");
    rig.advance(1_000);
    rig.settle();
    assert_eq!(
        message_of(&rig),
        "The password could not be checked.",
        "the screen is told the check failed and is not left waiting"
    );
    type_and_enter(&mut rig, "hunter2");
    rig.settle();
    assert_eq!(rig.comp.checks().len(), 2, "and Enter is an attempt again");
}

/// How many `heartbeat` calls the shell has made on the lock node.
fn beats(rig: &Rig) -> usize {
    rig.comp
        .calls()
        .iter()
        .filter(|(node, method)| *node == LOCK && method == "heartbeat")
        .count()
}

#[test]
fn the_lock_screen_renews_its_lease_twice_a_second_while_it_is_up_and_not_after_it_comes_down() {
    let mut rig = locked();
    let first = beats(&rig);
    assert!(first >= 1, "a screen that is up renews its lease at once");
    rig.settle();
    assert_eq!(beats(&rig), first, "no time has passed: one beat");
    rig.advance(499);
    rig.settle();
    assert_eq!(beats(&rig), first, "a beat every half second, no oftener");
    rig.advance(1);
    rig.settle();
    assert_eq!(beats(&rig), first + 1);
    rig.advance(5_000);
    rig.settle();
    assert_eq!(
        beats(&rig),
        first + 2,
        "a long wait is one beat, not a burst to make up for it"
    );

    // The session unlocks: the screen comes down, and a screen that is not up has no lease to renew.
    rig.comp.external_set(LOCK, "state", text("not-locked"));
    rig.settle();
    let down = beats(&rig);
    rig.advance(5_000);
    rig.settle();
    assert_eq!(beats(&rig), down, "the screen is down and renews nothing");
}

#[test]
fn a_lock_frame_that_throws_renews_no_lease_and_the_lease_comes_back_when_it_runs_again() {
    let mut rig = locked();
    let before = beats(&rig);
    assert!(before >= 1, "the screen is up and renews its lease");

    // The screen's frame throws from now on: the toolkit reports it and the shell goes on, but a frame that did not run
    // to its end says nothing about the screen being alive. The compositor's card takes the password when the beats stop.
    exec(&mut rig, "lockScreen().frame = () => { throw new Error('broken'); };");
    let mut told = Vec::new();
    for _ in 0..4 {
        rig.advance(600);
        told.extend(rig.settle());
    }
    assert_eq!(beats(&rig), before, "a screen whose frame throws renews nothing");
    assert_eq!(
        told.iter()
            .filter(|line| line.contains("LockScreen.frame threw"))
            .count(),
        1,
        "and the failure is reported once, not once a frame: {told:?}"
    );

    // It is mended: the next frame runs to its end and the lease is renewed again.
    exec(&mut rig, "delete lockScreen().frame;");
    rig.advance(600);
    rig.settle();
    assert!(
        beats(&rig) > before,
        "the screen renews its lease once its frame runs again"
    );
}

#[test]
fn a_screen_with_a_canvas_that_fails_to_draw_renews_no_lease_and_the_lease_comes_back_when_it_draws_again() {
    let mut rig = locked();
    let before = beats(&rig);
    assert!(before >= 1, "the screen is up and renews its lease");

    // One canvas of the screen cannot draw from now on. The toolkit reports it and draws the rest, and the screen's own
    // frame runs to its end: only the toolkit's pass over the screen failed. A turn that failed to draw is not a healthy
    // one, so no lease is renewed while it goes on and the compositor's card takes the password.
    exec(
        &mut rig,
        "globalThis.drawn = Text.prototype.draw; Text.prototype.draw = function () { throw new Error('cannot draw'); };",
    );
    let mut told = Vec::new();
    for _ in 0..4 {
        rig.advance(600);
        told.extend(rig.settle());
    }
    assert_eq!(beats(&rig), before, "a screen that cannot draw renews nothing");
    assert!(
        told.iter().any(|line| line.contains("draw threw")),
        "the failure is reported: {told:?}"
    );

    // It is mended: the first turn that draws every canvas renews the lease again.
    exec(&mut rig, "Text.prototype.draw = globalThis.drawn;");
    rig.advance(600);
    rig.settle();
    assert!(
        beats(&rig) > before,
        "the screen renews its lease once its canvases draw again"
    );
}

#[test]
fn a_screen_whose_canvases_the_compositor_destroyed_renews_no_lease() {
    let mut rig = locked();
    let before = beats(&rig);
    assert!(before >= 1);
    // The compositor destroyed the screen's nodes: what the shell holds is no screen, whatever its frame says.
    let scrim = *rig.comp.children(LOCK).first().expect("the lock screen is up");
    rig.comp.destroy(scrim);
    for _ in 0..4 {
        rig.advance(600);
        rig.settle();
    }
    assert_eq!(
        beats(&rig),
        before,
        "a screen that is no longer in the tree renews nothing"
    );
}

#[test]
fn a_key_handler_that_throws_is_reported_and_is_not_a_turn_that_failed_to_draw() {
    let mut rig = locked();
    let before = beats(&rig);
    // A handler acts on one key and reports its own failure; the frame that drew the screen is a healthy one.
    exec(
        &mut rig,
        "globalThis.keyed = PasswordEntry.prototype.onKey; PasswordEntry.prototype.onKey = function () { throw new Error('key handler'); };",
    );
    let (entry, _) = entry_and_message(&rig);
    rig.comp.key(entry, "a", "a", true);
    rig.comp.key(entry, "a", "", false);
    rig.advance(600);
    let told = rig.settle();
    exec(&mut rig, "PasswordEntry.prototype.onKey = globalThis.keyed;");
    assert!(
        beats(&rig) > before,
        "a handler's failure does not stop the lease: {told:?}"
    );
}

/// The seat's focus, as the compositor holds it.
fn focus(rig: &Rig) -> Option<u64> {
    match rig.comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(id)) if id != 0 => Some(id),
        _ => None,
    }
}

const ARRIVAL: u64 = 6;

#[test]
fn a_window_that_takes_the_focus_while_the_screen_is_up_loses_it_to_the_screen_and_has_it_back_when_it_comes_down() {
    let mut rig = locked();
    let (entry, _) = entry_and_message(&rig);
    assert_eq!(focus(&rig), Some(entry), "the entry holds the keyboard");

    // A window arrives under the lock and the stock policy gives it the focus: the seat's focus names it. The screen is
    // modal, so the next frame takes the keyboard back: a password typed now must have a node that reads it.
    rig.comp.create_window(ARRIVAL, "Arrival", "foot");
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(ARRIVAL));
    settle(&mut rig);
    assert_eq!(
        focus(&rig),
        Some(entry),
        "the screen takes the keyboard back from a window that took it"
    );

    // The session unlocks: the window that was given the focus under the lock is the one that has it, as it would
    // have had it without the lock, and not the one that had it before.
    rig.comp.external_set(LOCK, "state", text("not-locked"));
    settle(&mut rig);
    assert_eq!(
        focus(&rig),
        Some(ARRIVAL),
        "the window that took the focus under the lock has it when the screen comes down"
    );
}

#[test]
fn a_refusal_at_a_serial_the_mirror_has_already_reached_does_not_hold_the_resend_back() {
    let mut rig = world(1, "new Layout()");
    // The shell's base is ahead of the compositor's serial: it refuses with no op named and no batch follows, so the
    // mirror has nothing to catch up on and the gate must not wait for the serial the refused transaction was stamped
    // with, which the compositor never reaches.
    rig.comp.fault(Fault::StaleBase {
        failed_op: Some(-1),
        racing: Vec::new(),
    });
    exec(&mut rig, "gnoblin.clients.get('3').x = 7; 0");
    let told = rig.settle();
    assert_eq!(rig.comp.refused(), 1);
    assert_eq!(
        rig.comp.number(3, "x"),
        7.0,
        "the write went again and landed: {told:?}"
    );
}

// ---- a password check beside other writes ------------------------------------------------------------------------

fn is_check(op: &TxOp) -> bool {
    matches!(op, TxOp::Call { method, .. } if method == "authenticate")
}

#[test]
fn a_password_check_goes_in_a_transaction_of_its_own_and_the_frames_other_writes_follow_it() {
    let mut rig = locked();
    let mark = rig.comp.sent_count();
    // The frame that carries the check also carries a write of the script's, and the lock screen's own repaint.
    exec(&mut rig, "gnoblin.clients.get('5').x = 7;");
    type_and_enter(&mut rig, "hunter2");
    rig.frame();
    let sent = rig.comp.sent_since(mark);
    let at = sent
        .iter()
        .position(|ops| ops.iter().any(is_check))
        .expect("the check was sent");
    assert!(
        sent[at].iter().all(is_check),
        "the check is alone in its transaction, so a refusal of what the frame wrote cannot cost it: {:?}",
        sent[at]
    );
    assert_eq!(
        at, 0,
        "and it goes first, without waiting behind the frame's other writes"
    );

    rig.settle();
    assert_eq!(
        rig.comp.number(WINDOW, "x"),
        7.0,
        "the frame's other writes landed behind the check"
    );
    assert_eq!(rig.comp.checks().len(), 1, "the check was sent once");
}

#[test]
fn a_stale_refusal_of_the_writes_beside_a_password_check_does_not_cost_the_check() {
    let mut rig = locked();
    exec(&mut rig, "gnoblin.clients.get('5').x = 7;");
    type_and_enter(&mut rig, "hunter2");
    rig.frame();
    // The compositor writes the window just before the shell's next transaction lands: that one, which carries only
    // what the frame wrote beside the check, is refused and goes again.
    rig.comp.stale(1, None, WINDOW, "title");
    rig.settle();
    assert_eq!(
        rig.comp.refused(),
        1,
        "the transaction beside the check was refused once"
    );
    assert_eq!(
        rig.comp.number(WINDOW, "x"),
        7.0,
        "and its writes landed on the second try"
    );
    assert_eq!(rig.comp.checks().len(), 1, "the check went out once");
    assert_eq!(
        message_of(&rig),
        "Checking...",
        "the check is waiting for the compositor's answer, not failed"
    );
}
