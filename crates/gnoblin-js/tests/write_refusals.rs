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
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

mod common;
use common::*;

const IMPORTS: &str = "import * as lib from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

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
fn an_output_apply_refused_stale_base_is_sent_again_and_its_answer_reaches_the_application() {
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
        rig.eval("`${g.app.done} ${g.app.error}`"),
        "true null",
        "the application hears the compositor's answer"
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
    assert_eq!(rig.eval("g.app.done"), "true", "the application is settled");
    assert_ne!(rig.eval("String(g.app.error)"), "null", "and says it failed");
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
