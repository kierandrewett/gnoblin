//! The shell over a link that fails, on the shared fake compositor of `tests/common`, which can refuse a transaction for
//! a reason that is the script's (`Fault::Refuse`), answer late (`Fault::Late`), die (`Fault::Disconnect`), lose a
//! batch of changes (`FakeCompositor::drop_batches`) and refuse every nth transaction (`refuse_every`).
//!
//! `tests/write_refusals.rs` is the other half: it covers the refusals a newer base cures (`stale-base`, `unknown-node`).
//! What is asserted here is what a script and its callers may rely on when the link does not behave:
//!
//! - a refusal no newer base cures reports the compositor's word for it and the op it names, gives that op up (the
//!   script then reads what the compositor holds, and nothing sends the same refused value again), and costs the frame
//!   nothing else: the writes and calls beside the bad op are not lost, and a call that is given up is answered;
//! - a compositor that answers late loses no write and keeps the order of the writes;
//! - a link that dies mid-transaction ends the shell with a link error, ending the generation on a dead link is safe,
//!   and the shell that takes its place starts from what the compositor holds with no claim twice;
//! - a lost batch is a gap in the serials that the shell reports, and the shell does not apply what comes after it;
//! - whatever the order of stale refusals, the layout's placements converge on what a link with no refusals gives.

use std::path::Path;

use gnoblin_js::{JsShell, JsShellError};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxOp};

mod common;
use common::policy::{exec, settle, world};
use common::*;

/// The `x` of window `node` as the script reads it.
fn script_x(rig: &mut Rig, node: u64) -> f64 {
    rig.eval(&format!("String(gnoblin.clients.get('{node}').x)"))
        .parse()
        .expect("a number")
}

/// How many `set` ops of `key` on `node` the shell sent, refused attempts included.
fn sets_of(rig: &Rig, node: u64, key: &str) -> usize {
    rig.comp
        .sent_since(0)
        .iter()
        .flatten()
        .filter(|op| matches!(op, TxOp::Set { node: n, key: k, .. } if *n == node && k == key))
        .count()
}

// ---- a refusal no newer base cures ------------------------------------------------------------------------------

#[test]
fn a_refused_write_is_reported_with_the_compositors_word_and_not_kept_by_the_script() {
    let mut rig = world(2, false, "new Layout()", "");
    let held = rig.comp.number(3, "x");
    // The compositor refuses the first op for its value: no newer base cures that, whatever the shell does.
    rig.comp.fault(Fault::Refuse {
        code: "not-finite",
        failed_op: 0,
    });
    exec(&mut rig, "gnoblin.clients.get('3').x = 777;");
    let told = rig.settle();

    assert_eq!(rig.comp.refused(), 1);
    assert!(
        told.iter().any(|line| line.contains("refused at op 0: not-finite")
            && line.contains("set node 3")
            && line.contains("property x")),
        "the report names the code, the op and the write: {told:?}"
    );
    assert_eq!(rig.comp.number(3, "x"), held, "the compositor applied nothing of it");
    assert_eq!(
        script_x(&mut rig, 3),
        held,
        "the script reads what the compositor holds and not the value it was refused"
    );
    assert_eq!(sets_of(&rig, 3, "x"), 1, "the refused write is not sent again");
}

#[test]
fn a_refusal_costs_the_frame_its_bad_op_and_not_the_writes_beside_it() {
    let mut rig = world(3, false, "new Layout()", "");
    let (y4, x5) = (rig.comp.number(4, "y"), rig.comp.number(5, "x"));
    rig.comp.fault(Fault::Refuse {
        code: "read-only",
        failed_op: 1,
    });
    // Three writes in one frame, so one transaction: the compositor refuses the middle one and applies nothing.
    exec(
        &mut rig,
        "gnoblin.clients.get('3').x = 31; gnoblin.clients.get('4').y = 41; gnoblin.clients.get('5').x = 51;",
    );
    let told = rig.settle();

    assert_eq!(rig.comp.refused(), 1, "{told:?}");
    assert_eq!(
        rig.comp.number(3, "x"),
        31.0,
        "the write before the bad op landed after all"
    );
    assert_eq!(rig.comp.number(5, "x"), 51.0, "and so did the one after it");
    assert_eq!(rig.comp.number(4, "y"), y4, "the bad op is the only one given up");
    assert_ne!(rig.comp.number(5, "x"), x5);
    assert_eq!(script_x(&mut rig, 5), 51.0);
    assert_eq!(rig.eval("String(gnoblin.clients.get('4').y)"), y4.to_string());
    let attempts: Vec<Vec<TxOp>> = rig.comp.sent_since(0);
    let last = attempts.last().expect("a transaction landed");
    assert!(
        last.iter()
            .all(|op| !matches!(op, TxOp::Set { node: 4, key, .. } if key == "y")),
        "the refused op is not in the one that landed: {last:?}"
    );
}

#[test]
fn a_call_beside_a_refused_op_goes_again_and_is_answered() {
    let mut rig = world(2, true, "new Layout()", "");
    // The write at op 0 is refused; the screen's `apply` at op 1 is not at fault. Its caller waits for the answer.
    rig.comp.fault(Fault::Refuse {
        code: "not-finite",
        failed_op: 0,
    });
    exec(
        &mut rig,
        "gnoblin.clients.get('3').x = 777; g.app = gnoblin.outputs.at(1).apply({ scale: 1.25 });",
    );
    rig.settle();
    let sent = rig.comp.applies();
    assert_eq!(
        sent.len(),
        2,
        "the apply went out, was refused with its neighbour and went again: {sent:?}"
    );
    assert_eq!(sent[0], sent[1], "the same request under the same token");

    rig.comp.answer(sent[1].2, "ok", "");
    settle(&mut rig);
    assert_eq!(
        rig.eval("`${g.app.status} ${g.app.value === gnoblin.outputs.at(1)}`"),
        "succeeded true",
        "the operation hears the compositor's answer"
    );
}

#[test]
fn a_call_the_compositor_refuses_is_answered_failed_and_not_left_waiting() {
    let mut rig = world(0, true, "new Layout()", "");
    rig.comp.fault(Fault::Refuse {
        code: "unknown-member",
        failed_op: 0,
    });
    exec(&mut rig, "g.app = gnoblin.outputs.at(1).apply({ scale: 1.25 });");
    let told = rig.settle();
    assert_eq!(
        rig.comp.applies().len(),
        1,
        "a call that is wrong is not sent again: {told:?}"
    );
    assert_eq!(
        rig.eval("g.app.status"),
        "failed",
        "the operation is settled, and says it failed"
    );
    assert_eq!(
        rig.eval("g.app.error.message"),
        "failed: the transaction was refused unknown-member and the call was not sent again"
    );
}

#[test]
fn a_refusal_that_names_no_op_gives_the_whole_frame_up_and_answers_its_calls() {
    let mut rig = world(2, true, "new Layout()", "");
    let held = rig.comp.number(3, "x");
    // `too-large` and its kind refuse the batch as a whole: no op is named, so none can be told from the others.
    rig.comp.fault(Fault::Refuse {
        code: "too-large",
        failed_op: -1,
    });
    exec(
        &mut rig,
        "gnoblin.clients.get('3').x = 777; g.app = gnoblin.outputs.at(1).apply({ scale: 1.25 });",
    );
    let told = rig.settle();
    assert!(told.iter().any(|line| line.contains("too-large")), "{told:?}");
    assert_eq!(rig.comp.number(3, "x"), held, "nothing of the frame landed");
    assert_eq!(
        script_x(&mut rig, 3),
        held,
        "and the script reads what the compositor holds"
    );
    assert_eq!(
        rig.eval("g.app.status"),
        "failed",
        "the apply is settled, not waiting for an answer"
    );
    assert_eq!(
        rig.eval("`${g.app.error.code}: ${g.app.error.message}`"),
        "denied: failed: the transaction was refused too-large and the call was not sent again",
        "with the compositor's word for the refusal"
    );
    assert_eq!(rig.comp.refused(), 1, "nothing was sent again to be refused again");
}

// ---- a compositor that answers late ------------------------------------------------------------------------------

#[test]
fn a_reply_that_comes_late_loses_no_write_and_keeps_their_order() {
    let mut rig = world(2, false, "new Layout()", "");
    rig.comp.fault(Fault::Late { polls: 3 });
    exec(&mut rig, "gnoblin.clients.get('3').x = 101;");
    rig.frame(); // sends the transaction; the compositor applies it and says so late

    // While the answer is on its way the gate is shut: later writes wait, and the script reads its newest.
    exec(
        &mut rig,
        "gnoblin.clients.get('3').x = 102; gnoblin.clients.get('4').y = 9;",
    );
    rig.frame();
    assert_eq!(
        script_x(&mut rig, 3),
        102.0,
        "the script reads its newest write before the answer"
    );
    assert_eq!(
        rig.comp.sent_count(),
        1,
        "one transaction in flight, and the next waits behind it"
    );

    for _ in 0..12 {
        rig.frame();
    }
    assert_eq!(rig.comp.number(3, "x"), 102.0, "the newer write lands last");
    assert_eq!(rig.comp.number(4, "y"), 9.0);
    assert_eq!(rig.comp.sent_count(), 2, "two transactions in all, in order");
    assert_eq!(script_x(&mut rig, 3), 102.0);
}

// ---- a link that dies ----------------------------------------------------------------------------------------------

/// A shell of the same config over the same compositor, as the guardian's restart makes one.
fn restart(rig: &Rig) -> JsShell {
    let root = std::fs::canonicalize(rig._dir.path()).expect("the temp dir exists");
    let (config, _clock) = timed_config(&root);
    let mut shell = JsShell::new(Box::new(rig.comp.clone()), config).expect("the shell boots on the new link");
    shell.load_entry(Path::new("shell.ts")).expect("shell.ts loads");
    shell
}

#[test]
fn a_link_that_dies_mid_transaction_ends_the_shell_with_a_link_error_and_the_next_one_starts_clean() {
    let mut rig = world(
        2,
        false,
        "new Layout()",
        "gnoblin.input.primary.bind('Super+t', () => {});",
    );
    let claimed = rig.comp.claimed();
    assert!(!claimed.is_empty(), "the binding made a claim");

    rig.comp.fault(Fault::Disconnect);
    exec(&mut rig, "gnoblin.clients.get('3').x = 55;");
    let ended = rig.shell.frame().expect_err("the transaction could not be sent");
    assert!(
        matches!(ended, JsShellError::Link(_)),
        "a link error and no other: {ended:?}"
    );
    assert_eq!(rig.comp.number(3, "x"), 10.0, "the write never reached the compositor");

    // The role drops a shell whose link failed after ending its generation. Ending it on a dead link is safe: the
    // script is told (its cancel hooks run) and nothing hangs or panics for want of a link.
    let retired = rig.shell.retire();
    assert!(
        retired.is_ok() || matches!(retired, Err(JsShellError::Link(_))),
        "ending the generation on a dead link is a link error at worst: {retired:?}"
    );
    let again = rig.shell.frame().expect_err("a dead link stays dead");
    assert!(matches!(again, JsShellError::Link(_)), "{again:?}");

    // The compositor drops what the dead connection held; the shell that takes its place claims once and sees the
    // windows as they are, the failed write included as not made.
    rig.comp.reconnect();
    assert!(
        rig.comp.claimed().is_empty(),
        "the compositor released the dead shell's claims"
    );
    let mut second = restart(&rig);
    for _ in 0..6 {
        second.frame().expect("the second shell's frame");
    }
    assert_eq!(
        rig.comp.claimed(),
        claimed,
        "each claim is held once, by the shell that holds the tree"
    );
    assert_eq!(
        second
            .eval("String(gnoblin.clients.get('3').x)")
            .expect("the script runs")
            .as_deref(),
        Some("10"),
        "the new shell reads what the compositor holds"
    );
}

#[test]
fn a_link_that_is_dead_before_the_first_frame_is_a_link_error_and_nothing_else() {
    let mut rig = world(1, false, "new Layout()", "");
    rig.comp.disconnect();
    exec(&mut rig, "gnoblin.clients.get('3').x = 5;");
    let ended = rig.shell.frame().expect_err("the link is gone");
    assert!(matches!(ended, JsShellError::Link(_)), "{ended:?}");
}

// ---- a lost batch --------------------------------------------------------------------------------------------------

#[test]
fn a_batch_the_link_lost_is_a_gap_the_shell_reports_and_does_not_apply_around() {
    let mut rig = world(1, false, "new Layout()", "");
    rig.comp.drop_batches(1);
    rig.comp.external_set(3, "title", text("lost"));
    rig.comp.external_set(3, "title", text("after the gap"));
    let ended = rig.shell.frame().expect_err("a serial was skipped");
    assert!(
        matches!(ended, JsShellError::Mirror(_)),
        "the mirror refuses a batch that skips a serial: {ended:?}"
    );
    assert_eq!(
        rig.eval("gnoblin.clients.get('3').title"),
        "win1",
        "the batch after the gap was not applied around it"
    );
}

// ---- whatever the order of the refusals --------------------------------------------------------------------------

/// A deterministic generator: the sweep is the same run on every machine.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

/// A layout that gives every window the whole area it is offered.
const FILL: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
}\n";

/// A shell over one screen and the workspace `1` with the layout `Fill`, and no window.
fn tiled() -> Rig {
    let init = format!("{FILL}gnoblin.workspaces.add(new Workspace({{ id: \"1\", layout: new Fill() }}));\n");
    let mut rig = rig_setup(&common::policy::imports(), &init, |comp| {
        comp.add_screen(20, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
        comp.add_node(
            21,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
    });
    settle(&mut rig);
    rig
}

/// Windows that open at the frames `opens` names, run for `frames` frames; every window ends where the layout puts it.
fn run_tiled(seed: u64, refuse: impl FnOnce(&Rig, &mut Lcg)) -> Rig {
    let mut rig = tiled();
    let mut rng = Lcg(seed);
    refuse(&rig, &mut rng);
    let mut next = 100;
    for frame in 0..40 {
        if frame % 3 == 0 && next < 105 {
            rig.comp.create_window(next, "w", "foot");
            next += 1;
        }
        rig.frame();
    }
    rig
}

#[test]
fn placements_converge_whatever_stale_refusals_come_in_whatever_order() {
    let fine = run_tiled(0, |_, _| {});
    let want: Vec<String> = (100..105).map(|id| fine.comp.rect(id)).collect();
    assert!(
        want.iter().all(|rect| rect == "0,0,1000,800"),
        "the layout places every window with no refusal: {want:?}"
    );
    // The core's registry is one per process: a rig is gone before the next is made.
    drop(fine);
    for seed in 1..=60 {
        let rig = run_tiled(seed, |rig, rng| {
            for round in 0..rng.below(7) {
                let failed_op = match rng.below(3) {
                    0 => Some(-1),
                    1 => None,
                    _ => Some(rng.below(4) as i64),
                };
                // The racing batch writes the seat, which is there from the start; the op the refusal names is the
                // seed's choice, as the real check names the first op on a node that batch changed.
                rig.comp.fault(Fault::StaleBase {
                    failed_op,
                    racing: vec![set_change(21, "name", text(&format!("race{round}")))],
                });
            }
        });
        let got: Vec<String> = (100..105).map(|id| rig.comp.rect(id)).collect();
        assert_eq!(
            got,
            want,
            "seed {seed}: refusals {} of the frames' writes",
            rig.comp.refused()
        );
    }
}

#[test]
fn placements_converge_when_every_third_transaction_is_refused() {
    let mut rig = tiled();
    rig.comp.refuse_every(3);
    for id in 100..105 {
        rig.comp.create_window(id, "w", "foot");
        for _ in 0..4 {
            rig.frame();
        }
    }
    for _ in 0..12 {
        rig.frame();
    }
    assert!(rig.comp.refused() > 0, "the mode refused something");
    for id in 100..105 {
        assert_eq!(
            rig.comp.rect(id),
            "0,0,1000,800",
            "window {id} after {} refusals",
            rig.comp.refused()
        );
    }
}
