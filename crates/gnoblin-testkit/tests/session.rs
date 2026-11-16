//! The tree session over the fake compositor: attach, the events a batch raises, writes flushed as one transaction, and a
//! refusal sent again. These are the rules every shell runtime shares (`gnoblin_tree_client::session`), proved without a
//! language on top.

use std::time::Duration;

use gnoblin_testkit::compositor::{FakeCompositor, ROOT, text};
use gnoblin_tree_client::session::{NodeEvent, TreeSession};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TreeIncoming, TxAnimation};

const WINDOW: u64 = 40;

fn session(comp: &FakeCompositor) -> TreeSession {
    TreeSession::attach(Box::new(comp.clone()), Duration::from_secs(2)).expect("the session attaches")
}

fn instant() -> TxAnimation {
    TxAnimation::default()
}

#[test]
fn attach_reads_the_snapshot_through_the_builder() {
    let comp = FakeCompositor::new();
    comp.add_window(WINDOW, "Terminal", "foot");
    let session = session(&comp);
    assert_eq!(session.root(), ROOT);
    assert_eq!(session.class_of(WINDOW).as_deref(), Some("window"));
    assert_eq!(session.get(WINDOW, "title"), Some(&text("Terminal")));
    assert_eq!(session.children_of(ROOT), Some(&[WINDOW][..]));
    assert!(!session.exists(999));
}

#[test]
fn a_batch_is_applied_whole_before_any_of_its_events_are_returned() {
    let comp = FakeCompositor::new();
    let mut session = session(&comp);
    comp.create_window(WINDOW, "Files", "nautilus");
    let pumped = session.pump().expect("a pump");
    // Created is raised, the properties of the batch are in the mirror already, and the batch's `Set`s on the new node raise
    // nothing: the node is born with its final values.
    assert!(matches!(
        pumped.changes.first(),
        Some(NodeEvent::Created { node: WINDOW, class }) if class == "window"
    ));
    assert_eq!(session.get(WINDOW, "title"), Some(&text("Files")));
    assert!(
        !pumped
            .changes
            .iter()
            .any(|event| matches!(event, NodeEvent::Changed { .. })),
        "{:?}",
        pumped.changes
    );

    comp.external_set(WINDOW, "title", text("Files (2)"));
    let pumped = session.pump().expect("a pump");
    assert_eq!(
        pumped.changes,
        [NodeEvent::Changed {
            node: WINDOW,
            class: "window".to_owned(),
            key: "title".to_owned(),
            value: text("Files (2)"),
        }]
    );

    comp.destroy(WINDOW);
    let pumped = session.pump().expect("a pump");
    assert_eq!(
        pumped.changes,
        [NodeEvent::Destroyed {
            node: WINDOW,
            class: "window".to_owned(),
            removed: Vec::new(),
        }]
    );
    assert!(!session.exists(WINDOW), "a destroyed node is gone from the mirror");
}

#[test]
fn a_write_is_readable_at_once_and_goes_out_as_one_transaction_on_flush() {
    let comp = FakeCompositor::new();
    comp.add_window(WINDOW, "Terminal", "foot");
    let mut session = session(&comp);

    session
        .set(WINDOW, "x", MirrorValue::Double(10.0), instant())
        .expect("the node exists");
    session
        .set(WINDOW, "x", MirrorValue::Double(20.0), instant())
        .expect("the node exists");
    assert_eq!(
        session.get(WINDOW, "x"),
        Some(&MirrorValue::Double(20.0)),
        "read through the tick"
    );
    assert_eq!(comp.number(WINDOW, "x"), 0.0, "the compositor has heard nothing yet");

    assert!(session.flush().expect("a flush"));
    assert_eq!(comp.sent_count(), 1, "one transaction");
    assert_eq!(
        comp.number(WINDOW, "x"),
        20.0,
        "the last of the two writes of one property"
    );
    assert_eq!(session.transactions_sent(), 1);

    // The reply and the change batch come on the next pump; then the gate is open again.
    let pumped = session.pump().expect("a pump");
    assert!(pumped.effects.is_empty(), "{:?}", pumped.effects);
    assert!(session.is_settled());
    assert_eq!(session.mirror().value(WINDOW, "x"), Some(&MirrorValue::Double(20.0)));
}

#[test]
fn a_node_the_shell_makes_gets_an_id_of_its_own_and_lands_in_the_compositor() {
    let comp = FakeCompositor::new();
    let mut session = session(&comp);
    let canvas = session.create("canvas").expect("an id");
    assert_ne!(canvas, 0);
    assert_eq!(
        session.class_of(canvas).as_deref(),
        Some("canvas"),
        "visible in the tick before it is sent"
    );
    session.insert(canvas, ROOT, -1).expect("the root is there");
    session.flush().expect("a flush");
    assert_eq!(comp.children(ROOT), [canvas]);
    // The compositor's ids and the shell's cannot meet: the shell's carry the epoch in the high half.
    let other = session.create("canvas").expect("an id");
    assert_eq!(other, canvas + 1);
}

#[test]
fn a_stale_refusal_is_sent_again_on_the_next_base() {
    let comp = FakeCompositor::new();
    comp.add_window(WINDOW, "Terminal", "foot");
    let mut session = session(&comp);
    comp.stale(1, None, WINDOW, "title");

    session
        .set(WINDOW, "x", MirrorValue::Double(5.0), instant())
        .expect("the node exists");
    session
        .set(WINDOW, "title", text("mine"), instant())
        .expect("the node exists");
    session.flush().expect("a flush");
    assert_eq!(comp.number(WINDOW, "x"), 0.0, "a refused transaction applies nothing");

    // The racing batch and the refusal arrive; the gate keeps the ops and the next flush sends them on the new base.
    session.pump().expect("a pump");
    assert!(!session.is_settled(), "the ops wait to go again");
    assert!(session.flush().expect("a flush"), "sent again");
    assert_eq!(comp.number(WINDOW, "x"), 5.0);
    assert_eq!(comp.sent_count(), 2);
    session.pump().expect("a pump");
    assert!(session.is_settled());
}

#[test]
fn a_call_is_answered_by_its_token_and_a_lost_call_is_answered_failed() {
    let comp = FakeCompositor::new();
    comp.add_window(WINDOW, "Terminal", "foot");
    let mut session = session(&comp);
    let token = session
        .call(WINDOW, "close", Vec::new(), String::new())
        .expect("the node exists");
    session.flush().expect("a flush");
    comp.answer(token, "closed", "");
    let pumped = session.pump().expect("a pump");
    assert!(pumped.incoming.iter().any(
        |message| matches!(message, TreeIncoming::Answer { token: t, error, .. } if *t == token && error.is_empty())
    ));

    // A refusal no base cures gives the call up: it comes back as an answer that failed, so its caller is not left waiting.
    let doomed = session
        .call(WINDOW, "close", Vec::new(), String::new())
        .expect("the node exists");
    comp.fault(gnoblin_testkit::compositor::Fault::Refuse {
        code: "bad-value",
        failed_op: 0,
    });
    session.pump().expect("a pump");
    session.flush().expect("a flush");
    let pumped = session.pump().expect("a pump");
    assert!(
        pumped
            .incoming
            .iter()
            .any(|message| matches!(message, TreeIncoming::Answer { token: t, error, .. } if *t == doomed && error.contains("bad-value"))),
        "{:?}",
        pumped.incoming
    );
}
