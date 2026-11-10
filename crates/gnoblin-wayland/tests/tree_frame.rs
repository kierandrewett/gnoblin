//! The tree frame boundary: queued transactions are applied at the start of a frame,
//! whole or not at all, in arrival order, and one change batch is published per frame.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gnoblin_render::transaction::{
    ClassInfo, Code, PropertyInfo, TreeApplied, TreeChange, TreeOp, TreeRefusal, TreeTransaction, TreeValue, ValueType,
};
use gnoblin_render::{Actor, NodeSpec, Scene};
use gnoblin_types::{NodeId, Rect};
use gnoblin_wayland::{TreeBatch, TreeFrame, TreeFrameInput, TreeHooks, TreeInput, TreeQueue};

/// A schema with one class, `node`, holding one animatable double.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "seat")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        (class == "node" && key == "opacity").then_some(PropertyInfo {
            kind: ValueType::Double,
            readonly: false,
            animated: true,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        matches!(class, "seat" | "node") && matches!(method, "claim" | "unclaim")
    }
}

const ROOT: NodeId = NodeId::new(1);
const FIRST: NodeId = NodeId::new(0x100);
const SECOND: NodeId = NodeId::new(0x101);
const THIRD: NodeId = NodeId::new(0x102);

/// A scene with one compositor-owned root, as the compositor leaves it.
fn scene() -> Scene {
    let mut scene = Scene::new();
    scene
        .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
        .expect("root inserts");
    scene.set_root(Some(ROOT)).expect("root names");
    scene
}

/// A transaction that creates `id` and places it under the root.
fn add(id: NodeId) -> TreeTransaction {
    TreeTransaction::new(vec![
        TreeOp::New {
            class: "node".to_owned(),
            id,
        },
        TreeOp::Insert {
            node: id,
            parent: ROOT,
            index: None,
        },
    ])
}

/// A transaction that is refused: its second op names a class the schema does not have.
fn refused_add(id: NodeId) -> TreeTransaction {
    TreeTransaction::new(vec![
        TreeOp::New {
            class: "node".to_owned(),
            id,
        },
        TreeOp::New {
            class: "missing".to_owned(),
            id: NodeId::new(0x900),
        },
    ])
}

/// A clock that moves `step` per reading, so a frame's elapsed time is a count of readings.
fn clock(step: Duration) -> impl Fn() -> Instant {
    let origin = Instant::now();
    let readings = Cell::new(0u32);
    move || {
        let reading = readings.get();
        readings.set(reading + 1);
        origin + step * reading
    }
}

type Replies = Arc<Mutex<Vec<(u64, Result<TreeApplied, TreeRefusal>)>>>;

/// Submits `tx` on `frame`'s queue and records its reply into the returned list.
fn submit(frame: &TreeFrame, base: u64, tx: TreeTransaction, replies: &Replies) {
    let replies = Arc::clone(replies);
    frame.queue().submit(
        base,
        tx,
        || true,
        move |serial, result| replies.lock().unwrap().push((serial, result)),
    );
}

thread_local! {
    /// The batches this thread's frames published, in order, until a test takes them.
    static PUBLISHED: RefCell<Vec<TreeBatch>> = const { RefCell::new(Vec::new()) };
}

/// Hooks whose publish records each batch on this thread, for [`take_batches`].
fn hooks(queue: TreeQueue) -> TreeHooks {
    TreeHooks {
        queue,
        classes: Arc::new(Table),
        publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
        ..TreeHooks::default()
    }
}

/// Takes the batches published on this thread since the last take, in publication order.
fn take_batches() -> Vec<TreeBatch> {
    PUBLISHED.with(|published| std::mem::take(&mut *published.borrow_mut()))
}

fn new_frame() -> TreeFrame {
    TreeFrame::with_hooks(hooks(TreeQueue::default()))
}

#[test]
fn seat_claim_calls_are_captured_in_op_order_and_others_are_not() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    let seat = NodeId::new(0x200);
    let other = NodeId::new(0x201);
    submit(
        &frame,
        0,
        TreeTransaction::new(vec![
            TreeOp::New {
                class: "seat".to_owned(),
                id: seat,
            },
            TreeOp::Insert {
                node: seat,
                parent: ROOT,
                index: None,
            },
            TreeOp::New {
                class: "node".to_owned(),
                id: other,
            },
            TreeOp::Insert {
                node: other,
                parent: ROOT,
                index: None,
            },
            TreeOp::TreeCall {
                node: seat,
                method: "claim".to_owned(),
                args: vec![7],
                text: "Ctrl+Shift+P".to_owned(),
            },
            TreeOp::TreeCall {
                node: other,
                method: "claim".to_owned(),
                args: vec![9],
                text: "Ctrl+Q".to_owned(),
            },
            TreeOp::TreeCall {
                node: seat,
                method: "unclaim".to_owned(),
                args: vec![7],
                text: String::new(),
            },
        ]),
        &replies,
    );

    frame.drain(&mut scene, clock(Duration::ZERO));

    let claims = frame.take_claims();
    assert_eq!(claims.len(), 2, "only seat claim and unclaim calls are kept");
    assert_eq!(claims[0].method, "claim");
    assert_eq!(claims[0].text, "Ctrl+Shift+P");
    assert_eq!(claims[1].method, "unclaim");
    assert_eq!(claims[1].args, vec![7]);
    assert!(frame.take_claims().is_empty(), "taking drains the claims");
    assert!(replies.lock().unwrap()[0].1.is_ok());
}

#[test]
fn a_transaction_queued_before_the_frame_draws_in_that_frame() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, add(FIRST), &replies);

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(scene.contains(FIRST), "the frame's scene holds the transaction's node");
    let batches = take_batches();
    assert_eq!(batches.len(), 1, "one batch for the frame that changed");
    assert_eq!(batches[0].serial, 1);
    assert_eq!(frame.serial(), 1);
    let replies = replies.lock().unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].0, 1, "the reply carries the serial after the frame");
    assert!(replies[0].1.is_ok());
}

#[test]
fn a_transaction_queued_after_the_drain_draws_one_frame_later() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();

    // Frame one: the drain has run; the shell's transaction arrives while the frame composes.
    frame.drain(&mut scene, clock(Duration::ZERO));
    submit(&frame, 0, add(FIRST), &replies);
    assert!(
        !scene.contains(FIRST),
        "a late transaction is not half-drawn into the current frame"
    );
    assert!(take_batches().is_empty());
    assert!(
        replies.lock().unwrap().is_empty(),
        "no reply before the frame applies it"
    );

    // Frame two applies it, whole, and publishes its batch.
    frame.drain(&mut scene, clock(Duration::ZERO));
    assert!(scene.contains(FIRST));
    assert_eq!(take_batches().len(), 1);
    assert_eq!(replies.lock().unwrap().len(), 1);
}

#[test]
fn a_refused_transaction_changes_nothing_in_the_frame() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, refused_add(FIRST), &replies);

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(
        !scene.contains(FIRST),
        "the first op of a refused transaction did not apply"
    );
    assert!(take_batches().is_empty(), "a refused transaction publishes no batch");
    assert_eq!(frame.serial(), 0, "the serial does not move");
    let replies = replies.lock().unwrap();
    let refusal = replies[0].1.as_ref().expect_err("the transaction is refused");
    assert_eq!(refusal.failed_op, 1);
    assert_eq!(replies[0].0, 0);
}

#[test]
fn a_stale_base_is_refused_and_changes_nothing() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 7, add(FIRST), &replies);

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(!scene.contains(FIRST));
    assert!(take_batches().is_empty());
    let replies = replies.lock().unwrap();
    assert!(replies[0].1.is_err());
}

#[test]
fn two_transactions_in_one_frame_publish_one_batch() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, add(FIRST), &replies);
    submit(&frame, 0, add(SECOND), &replies);

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(scene.contains(FIRST) && scene.contains(SECOND));
    let batches: Vec<TreeBatch> = take_batches();
    assert_eq!(batches.len(), 1, "one batch per changed frame, not one per transaction");
    assert_eq!(batches[0].serial, 1, "the serial moves once for the frame");
    assert_eq!(
        batches[0].changes.len(),
        4,
        "both transactions' changes, in arrival order"
    );
    assert_eq!(replies.lock().unwrap().len(), 2);
}

#[test]
fn over_budget_work_defers_whole_transactions_in_order() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, add(FIRST), &replies);
    submit(&frame, 0, add(SECOND), &replies);
    submit(&frame, 0, add(THIRD), &replies);

    // Each reading is 5 ms later than the last, so the budget is spent after the first transaction.
    frame.drain(&mut scene, clock(Duration::from_millis(5)));

    assert!(scene.contains(FIRST));
    assert!(!scene.contains(SECOND), "the over-budget transaction waits, whole");
    assert!(
        !scene.contains(THIRD),
        "a transaction behind a deferred one does not jump it"
    );
    assert_eq!(take_batches().len(), 1);
    assert_eq!(
        replies.lock().unwrap().len(),
        1,
        "only applied transactions are answered"
    );

    // The next frame applies the deferred two, in order, against the serial the first frame left.
    frame.drain(&mut scene, clock(Duration::ZERO));
    assert!(scene.contains(SECOND) && scene.contains(THIRD));
    let batches = take_batches();
    assert_eq!(batches.len(), 1);
    let created: Vec<NodeId> = batches[0]
        .changes
        .iter()
        .filter_map(|change| match change {
            gnoblin_render::transaction::TreeChange::Created { node, .. } => Some(*node),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![SECOND, THIRD]);
}

#[test]
fn reads_see_the_last_published_serial_before_the_frame_transactions() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    frame.drain(&mut scene, clock(Duration::ZERO));
    submit(&frame, 0, add(FIRST), &replies);

    let seen = Arc::new(Mutex::new(None));
    let seen_in_read = Arc::clone(&seen);
    frame.queue().read(move |scene, serial| {
        *seen_in_read.lock().unwrap() = Some((scene.contains(FIRST), serial));
    });

    frame.drain(&mut scene, clock(Duration::ZERO));

    let (contains_first, serial) = seen.lock().unwrap().expect("the read ran");
    assert!(!contains_first, "a read runs before the frame's transactions");
    assert_eq!(serial, 0, "the read reports the last published serial");
}

#[test]
fn a_frame_draws_with_no_shell_attached() {
    let mut frame = new_frame();
    let mut scene = scene();

    frame.drain(&mut scene, clock(Duration::ZERO));
    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(take_batches().is_empty(), "no shell means no batches");
    assert_eq!(frame.serial(), 0);
    assert!(scene.contains(ROOT), "the scene is untouched and the frame still runs");
}

/// A transaction that sets `FIRST`'s opacity to `value`.
fn set_first(value: f64) -> TreeTransaction {
    TreeTransaction::new(vec![TreeOp::Set {
        node: FIRST,
        key: "opacity".to_owned(),
        value: TreeValue::Double(value),
        timing: None,
    }])
}

/// The opacity targets published in `batches`, in order.
fn published_opacities(batches: &[TreeBatch]) -> Vec<f64> {
    batches
        .iter()
        .flat_map(|batch| batch.changes.iter())
        .filter_map(|change| match change {
            TreeChange::Set {
                key,
                value: TreeValue::Double(value),
                ..
            } if key == "opacity" => Some(*value),
            _ => None,
        })
        .collect()
}

#[test]
fn a_deferred_transaction_made_stale_by_an_earlier_one_is_refused_at_apply() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    // Frame one applies the add and defers the two sets behind it: the budget is spent.
    submit(&frame, 0, add(FIRST), &replies);
    submit(&frame, 1, set_first(0.25), &replies);
    submit(&frame, 1, set_first(0.75), &replies);
    frame.drain(&mut scene, clock(Duration::from_millis(5)));
    assert_eq!(frame.serial(), 1);

    // Frame two applies the earlier set, so the deferred set's base no longer holds for FIRST.
    frame.drain(&mut scene, clock(Duration::ZERO));

    assert_eq!(frame.serial(), 2, "only the earlier set publishes");
    let batches = take_batches();
    assert_eq!(
        published_opacities(&batches),
        vec![0.25],
        "the stale set publishes nothing"
    );
    let replies = replies.lock().unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(
        replies[2].1,
        Err(TreeRefusal {
            failed_op: 0,
            code: Code::StaleBase
        }),
        "the stale write to FIRST is refused at its first conflicting op"
    );
}

#[test]
fn a_deferred_transaction_that_stays_non_conflicting_applies_in_order() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, add(FIRST), &replies);
    submit(&frame, 1, set_first(0.25), &replies);
    submit(&frame, 1, add(SECOND), &replies);
    frame.drain(&mut scene, clock(Duration::from_millis(5)));

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(scene.contains(SECOND), "a stale base that names only new nodes applies");
    assert_eq!(
        frame.serial(),
        2,
        "the earlier set and the add publish in one frame batch"
    );
    let batches = take_batches();
    assert_eq!(published_opacities(&batches), vec![0.25]);
    assert!(replies.lock().unwrap().iter().all(|(_, result)| result.is_ok()));
}

#[test]
fn two_stale_writes_to_one_node_in_one_frame_apply_once() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    submit(&frame, 0, add(FIRST), &replies);
    frame.drain(&mut scene, clock(Duration::ZERO));

    // Both are fresh at arrival (base 1 is the current serial); the first applies and moves FIRST.
    submit(&frame, 1, set_first(0.25), &replies);
    submit(&frame, 1, set_first(0.75), &replies);
    frame.drain(&mut scene, clock(Duration::ZERO));

    assert_eq!(frame.serial(), 2);
    let batches = take_batches();
    assert_eq!(published_opacities(&batches), vec![0.25]);
    let replies = replies.lock().unwrap();
    assert!(replies[1].1.is_ok());
    assert_eq!(
        replies[2].1,
        Err(TreeRefusal {
            failed_op: 0,
            code: Code::StaleBase
        })
    );
}

#[test]
fn a_submission_and_a_read_each_wake_the_frame_loop() {
    let wakes = Arc::new(Mutex::new(0_usize));
    let queue = TreeQueue::default();
    let counted = Arc::clone(&wakes);
    queue.on_submit(move || *counted.lock().unwrap() += 1);

    queue.submit(0, add(FIRST), || true, |_, _| {});
    queue.read(|_, _| {});

    assert_eq!(*wakes.lock().unwrap(), 2);
}

#[test]
fn a_frame_built_on_a_shared_queue_drains_what_a_clone_submits() {
    let queue = TreeQueue::default();
    let mut frame = TreeFrame::with_hooks(hooks(queue.clone()));
    let replies = Arc::new(Mutex::new(Vec::new()));

    submit(&frame, 0, add(FIRST), &replies);
    let mut scene = scene();
    frame.drain(&mut scene, Instant::now);
    frame.drain(&mut scene, Instant::now);

    assert_eq!(
        replies.lock().unwrap().len(),
        1,
        "the clone's submission is applied once"
    );
    assert!(scene.contains(FIRST));
}

#[test]
fn a_transaction_whose_holder_detached_before_the_frame_is_refused_not_privileged() {
    let mut frame = new_frame();
    let mut scene = scene();
    let replies = Replies::default();
    // The holder is still attached when the transaction is queued, and detaches before the frame.
    let holding = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let admitted = Arc::clone(&holding);
    let sink = Arc::clone(&replies);
    frame.queue().submit(
        0,
        add(FIRST),
        move || admitted.load(std::sync::atomic::Ordering::SeqCst),
        move |serial, result| sink.lock().unwrap().push((serial, result)),
    );
    holding.store(false, std::sync::atomic::Ordering::SeqCst);

    frame.drain(&mut scene, clock(Duration::ZERO));

    assert!(!scene.contains(FIRST), "queued work of a detached holder never lands");
    assert!(take_batches().is_empty(), "a refused transaction publishes no batch");
    assert_eq!(frame.serial(), 0, "a refused transaction does not advance the serial");
    let replies = replies.lock().unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].0, 0);
    let refusal = replies[0].1.as_ref().expect_err("the transaction is refused");
    assert_eq!(refusal.failed_op, -1);
    assert_eq!(refusal.code, Code::NotPrivileged);
}

#[test]
fn pointer_edges_reach_the_shell_in_the_order_they_arrive_in_one_batch() {
    let mut frame = new_frame();
    let mut scene = scene();
    frame.input(FIRST, TreeInput::Enter { x: 1.0, y: 2.0 });
    frame.input(FIRST, TreeInput::Motion { x: 3.0, y: 4.0 });
    frame.input(
        FIRST,
        TreeInput::Press {
            x: 3.0,
            y: 4.0,
            button: 272,
        },
    );
    frame.input(
        FIRST,
        TreeInput::Release {
            x: 3.0,
            y: 4.0,
            button: 272,
        },
    );
    frame.input(
        FIRST,
        TreeInput::Scroll {
            x: 3.0,
            y: 4.0,
            dx: 0.0,
            dy: 15.0,
        },
    );
    frame.input(FIRST, TreeInput::Leave);

    frame.drain(&mut scene, clock(Duration::ZERO));

    let batches = take_batches();
    assert_eq!(batches.len(), 1, "an input-only frame still publishes its batch");
    assert_eq!(batches[0].serial, 0, "pointer edges are not scene changes");
    assert_eq!(frame.serial(), 0, "pointer edges do not advance the serial");
    let events = &batches[0].events;
    let names: Vec<&str> = events
        .iter()
        .map(|event| match &event.event {
            TreeFrameInput::Input(input) => input.name(),
            TreeFrameInput::Request(_) => "request",
            TreeFrameInput::Ready => "ready",
            TreeFrameInput::Chord(_) => "chord",
        })
        .collect();
    assert_eq!(names, ["enter", "motion", "press", "release", "scroll", "leave"]);
    assert!(events.iter().all(|event| event.node == FIRST && event.frame == 1));
    assert!(
        events.windows(2).all(|pair| pair[0].seq < pair[1].seq),
        "sequence numbers increase"
    );
}
