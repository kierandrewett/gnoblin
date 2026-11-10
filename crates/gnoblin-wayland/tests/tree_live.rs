//! The tree on the composer's live scene. A node the composer creates is visible to a read
//! at the frame boundary, a shell transaction applies in that frame and is answered after
//! it, a refused one changes nothing, and a change the compositor recorded is announced in
//! the batch ahead of the shell's own changes.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gnoblin_backend::render::compose::Composer;
use gnoblin_render::transaction::{ClassInfo, PropertyInfo, TreeChange, TreeOp, TreeTransaction, TreeValue, ValueType};
use gnoblin_render::{Actor, NodeSpec};
use gnoblin_types::{NodeId, Rect};
use gnoblin_wayland::{FrameTreeEvent, TreeBatch, TreeFrame, TreeFrameInput, TreeHooks};

/// A schema with two classes: `node`, holding one writable double, `opacity`, and `lock`,
/// holding the read-only text `state`.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "lock")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        match (class, key) {
            ("node", "opacity") => Some(PropertyInfo {
                kind: ValueType::Double,
                readonly: false,
                animated: false,
            }),
            ("lock", "state") => Some(PropertyInfo {
                kind: ValueType::Text,
                readonly: true,
                animated: false,
            }),
            _ => None,
        }
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

/// A node the composer creates, standing in for a compositor-owned window.
const WINDOW: NodeId = NodeId::new(0x300);

/// A second composer-created node, which the shell writes, so the shell's change does not
/// conflict with the compositor's change to `WINDOW` in the same frame.
const OTHER: NodeId = NodeId::new(0x301);

/// The reply a transaction received: the serial after the frame, and whether it applied.
type Replies = Arc<Mutex<Vec<(u64, bool)>>>;

/// What a read at the frame boundary saw: whether the node exists, its class, and the serial.
type Observed = Arc<Mutex<Option<(bool, Option<String>, u64)>>>;

/// A composer whose scene holds one compositor-created node of class `node`.
fn composer_with_window() -> Composer {
    let mut composer = Composer::new();
    composer.with_scene(|scene| {
        scene
            .insert_as(Actor::Compositor, WINDOW, NodeSpec::new(Rect::new(0, 0, 200, 100)))
            .expect("the composer creates the window node");
        scene
            .set_class(WINDOW, Some("node".to_owned()))
            .expect("the window node takes a class");
    });
    composer
}

thread_local! {
    /// The batches this thread's frames published, in order, until a test takes them.
    static PUBLISHED: RefCell<Vec<TreeBatch>> = const { RefCell::new(Vec::new()) };
}

/// Takes the batches published on this thread since the last take, in publication order.
fn take_batches() -> Vec<TreeBatch> {
    PUBLISHED.with(|published| std::mem::take(&mut *published.borrow_mut()))
}

fn new_frame() -> TreeFrame {
    TreeFrame::with_hooks(TreeHooks {
        classes: Arc::new(Table),
        publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
        ..TreeHooks::default()
    })
}

/// A transaction that writes `opacity` on the window.
fn set_opacity(opacity: f64) -> TreeTransaction {
    TreeTransaction::new(vec![TreeOp::Set {
        node: WINDOW,
        key: "opacity".to_owned(),
        value: TreeValue::Double(opacity),
        timing: None,
    }])
}

/// A transaction that is refused: the schema has no property `height`.
fn refused_set() -> TreeTransaction {
    TreeTransaction::new(vec![TreeOp::Set {
        node: WINDOW,
        key: "height".to_owned(),
        value: TreeValue::Double(50.0),
        timing: None,
    }])
}

fn submit(frame: &TreeFrame, base: u64, transaction: TreeTransaction, replies: &Replies) {
    let replies = Arc::clone(replies);
    frame.queue().submit(
        base,
        transaction,
        || true,
        move |serial, result| {
            replies
                .lock()
                .expect("the reply lock is not poisoned")
                .push((serial, result.is_ok()));
        },
    );
}

fn drain(frame: &mut TreeFrame, composer: &mut Composer) {
    composer.with_scene(|scene| frame.drain(scene, Instant::now));
}

fn opacity_of(composer: &mut Composer) -> f32 {
    composer
        .with_scene(|scene| scene.composed_opacity(WINDOW))
        .expect("the window node exists")
}

#[test]
fn a_composer_node_is_visible_to_a_read_at_the_frame_boundary() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();
    let seen: Observed = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    frame.queue().read(move |scene, serial| {
        *sink.lock().expect("the read lock is not poisoned") =
            Some((scene.contains(WINDOW), scene.class(WINDOW).map(str::to_owned), serial));
    });

    drain(&mut frame, &mut composer);

    let (present, class, serial) = seen
        .lock()
        .expect("the read lock is not poisoned")
        .take()
        .expect("the read ran in the frame");
    assert!(present, "the read sees the composer's window node");
    assert_eq!(class.as_deref(), Some("node"));
    assert_eq!(serial, 0, "the read sees the serial before any frame published");
    assert!(take_batches().is_empty(), "a read alone publishes no batch");
}

/// F21: the shell dies while the lock is engaged. The lock node stays under the root at the
/// state the compositor published, and a new shell's first read at the frame boundary, its
/// snapshot, sees `locked`.
#[test]
fn a_shell_that_attaches_while_the_lock_is_engaged_reads_the_lock_state() {
    let mut composer = composer_with_window();
    let lock = composer.add_lock_node().expect("a lock node");
    composer.set_lock_state("locked");

    let mut dead_shell = new_frame();
    drain(&mut dead_shell, &mut composer);
    drop(dead_shell);
    take_batches();

    let root = composer.with_scene(|scene| scene.root_ids()[0]);
    assert_eq!(
        composer.with_scene(|scene| scene.parent(lock)),
        Some(root),
        "the lock stays in the scene under the root after the shell is gone"
    );

    let mut new_shell = new_frame();
    type Reading = (Option<String>, Option<String>);
    let seen: Arc<Mutex<Option<Reading>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    new_shell.queue().read(move |scene, _| {
        *sink.lock().expect("the read lock is not poisoned") = Some((
            scene.class(lock).map(str::to_owned),
            scene.text(lock, "state").map(str::to_owned),
        ));
    });
    drain(&mut new_shell, &mut composer);

    let (class, state) = seen
        .lock()
        .expect("the read lock is not poisoned")
        .take()
        .expect("the new shell's read ran in its frame");
    assert_eq!(class.as_deref(), Some("lock"));
    assert_eq!(
        state.as_deref(),
        Some("locked"),
        "the new shell's snapshot shows the lock engaged"
    );
}

#[test]
fn a_transaction_applies_at_the_frame_boundary_and_is_answered_after_it() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();
    let replies: Replies = Arc::default();
    let before = opacity_of(&mut composer);

    submit(&frame, 0, set_opacity(0.25), &replies);
    assert_eq!(
        opacity_of(&mut composer),
        before,
        "nothing applies before the frame drains"
    );
    assert!(replies.lock().expect("the reply lock is not poisoned").is_empty());

    drain(&mut frame, &mut composer);

    assert_eq!(
        opacity_of(&mut composer),
        0.25,
        "the frame applied the opacity to the live scene"
    );
    assert_eq!(
        *replies.lock().expect("the reply lock is not poisoned"),
        vec![(1, true)],
        "answered with the serial after the frame"
    );
    let batches = take_batches();
    assert_eq!(batches.len(), 1, "the frame publishes one batch");
    assert_eq!(batches[0].serial, 1);
}

#[test]
fn a_refused_transaction_is_answered_after_the_frame_and_leaves_the_node() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();
    let replies: Replies = Arc::default();
    let before = opacity_of(&mut composer);

    submit(&frame, 0, refused_set(), &replies);
    drain(&mut frame, &mut composer);

    assert_eq!(
        opacity_of(&mut composer),
        before,
        "a refused transaction changes nothing"
    );
    assert_eq!(
        *replies.lock().expect("the reply lock is not poisoned"),
        vec![(0, false)]
    );
    assert!(take_batches().is_empty(), "a refusal publishes no batch");
}

#[test]
fn a_compositor_change_is_announced_ahead_of_the_shell_change_in_one_batch() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();
    let replies: Replies = Arc::default();

    composer.with_scene(|scene| {
        scene
            .insert_as(Actor::Compositor, OTHER, NodeSpec::new(Rect::new(0, 0, 50, 50)))
            .expect("the composer creates the other node");
        scene
            .set_class(OTHER, Some("node".to_owned()))
            .expect("the other node takes a class");
    });
    frame.record(TreeChange::Created {
        node: WINDOW,
        class: "node".to_owned(),
        index: 0,
    });
    let set_other = TreeTransaction::new(vec![TreeOp::Set {
        node: OTHER,
        key: "opacity".to_owned(),
        value: TreeValue::Double(0.75),
        timing: None,
    }]);
    submit(&frame, 0, set_other, &replies);
    drain(&mut frame, &mut composer);

    let batches = take_batches();
    assert_eq!(
        batches.len(),
        1,
        "the compositor's change and the shell's share one batch"
    );
    let changes = &batches[0].changes;
    assert_eq!(changes.len(), 2, "got {changes:?}");
    assert!(
        matches!(&changes[0], TreeChange::Created { node, .. } if *node == WINDOW),
        "the compositor's created change comes first, got {changes:?}"
    );
    assert!(
        matches!(&changes[1], TreeChange::Set { node, key, .. } if *node == OTHER && key == "opacity"),
        "the shell's set follows, got {changes:?}"
    );
}

#[test]
fn a_compositor_change_alone_publishes_a_batch_at_the_next_serial() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();

    frame.record(TreeChange::Created {
        node: WINDOW,
        class: "node".to_owned(),
        index: 0,
    });
    drain(&mut frame, &mut composer);

    let batches = take_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].serial, 1);
    assert_eq!(batches[0].changes.len(), 1);
    assert_eq!(frame.serial(), 1);
}

/// A frame whose holder presence is `attached`, shared with the test so it can change it.
fn attachable_frame(attached: Arc<std::sync::atomic::AtomicBool>) -> TreeFrame {
    TreeFrame::with_hooks(TreeHooks {
        classes: Arc::new(Table),
        attached: Arc::new(move || attached.load(std::sync::atomic::Ordering::SeqCst)),
        publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
        ..TreeHooks::default()
    })
}

#[test]
fn compositor_changes_drained_while_detached_publish_nothing_and_keep_serial_zero() {
    let mut composer = composer_with_window();
    let attached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut frame = attachable_frame(Arc::clone(&attached));

    frame.record(TreeChange::Created {
        node: WINDOW,
        class: "node".to_owned(),
        index: 0,
    });
    drain(&mut frame, &mut composer);
    assert!(
        take_batches().is_empty(),
        "nothing is published while no holder is attached"
    );
    assert_eq!(frame.serial(), 0, "a fresh tree stays at serial zero while detached");

    attached.store(true, std::sync::atomic::Ordering::SeqCst);
    frame.record(TreeChange::Created {
        node: WINDOW,
        class: "node".to_owned(),
        index: 0,
    });
    drain(&mut frame, &mut composer);
    let batches = take_batches();
    assert_eq!(batches.len(), 1, "a change while attached arrives once as a batch");
    assert_eq!(batches[0].serial, 1);
    assert_eq!(frame.serial(), 1);
}

/// A seat node the owner creates: `new` and `set name` in one transaction.
const SEAT: NodeId = NodeId::new(0x400);

/// A transaction that creates a `seat` node named `name` under the root in one step.
fn create_seat(name: &str) -> TreeTransaction {
    TreeTransaction::new(vec![
        TreeOp::New {
            class: "seat".to_owned(),
            id: SEAT,
        },
        TreeOp::Set {
            node: SEAT,
            key: "name".to_owned(),
            value: TreeValue::Text(name.to_owned()),
            timing: None,
        },
    ])
}

/// A schema with a read-only `seat` class whose `name` is set at construction only.
struct SeatTable;

impl ClassInfo for SeatTable {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "seat")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        (class == "seat" && key == "name").then_some(PropertyInfo {
            kind: ValueType::Text,
            readonly: true,
            animated: false,
        })
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

fn seat_frame() -> TreeFrame {
    TreeFrame::with_hooks(TreeHooks {
        classes: Arc::new(SeatTable),
        publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
        ..TreeHooks::default()
    })
}

fn seat_name(composer: &mut Composer) -> Option<String> {
    composer.with_scene(|scene| scene.seat_name(SEAT).map(str::to_owned))
}

#[test]
fn a_readonly_name_is_set_in_the_transaction_that_creates_the_seat() {
    let mut frame = seat_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_seat("xr"), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(*replies.lock().unwrap(), vec![(1, true)]);
    assert_eq!(seat_name(&mut composer).as_deref(), Some("xr"));
}

#[test]
fn a_later_set_of_a_readonly_name_is_refused_and_leaves_the_seat_named() {
    let mut frame = seat_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_seat("xr"), &replies);
    drain(&mut frame, &mut composer);
    submit(
        &frame,
        1,
        TreeTransaction::new(vec![TreeOp::Set {
            node: SEAT,
            key: "name".to_owned(),
            value: TreeValue::Text("other".to_owned()),
            timing: None,
        }]),
        &replies,
    );
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(1, false)));
    assert_eq!(seat_name(&mut composer).as_deref(), Some("xr"));
}

#[test]
fn an_empty_seat_name_is_refused_bad_value_and_nothing_is_created() {
    let mut frame = seat_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_seat(""), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(*replies.lock().unwrap(), vec![(0, false)]);
    assert_eq!(seat_name(&mut composer), None);
}

#[test]
fn a_seat_name_already_taken_by_another_seat_is_refused_bad_value() {
    let mut frame = seat_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_seat("xr"), &replies);
    drain(&mut frame, &mut composer);
    let second = NodeId::new(0x401);
    submit(
        &frame,
        1,
        TreeTransaction::new(vec![
            TreeOp::New {
                class: "seat".to_owned(),
                id: second,
            },
            TreeOp::Set {
                node: second,
                key: "name".to_owned(),
                value: TreeValue::Text("xr".to_owned()),
                timing: None,
            },
        ]),
        &replies,
    );
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(1, false)));
    composer.with_scene(|scene| assert_eq!(scene.seat_name(second), None));
}

#[test]
fn a_window_node_is_announced_ready_once_in_the_batch_that_follows_its_mapping() {
    let mut composer = composer_with_window();
    let mut frame = new_frame();
    frame.ready(WINDOW);
    drain(&mut frame, &mut composer);
    let batches = take_batches();
    let ready: Vec<&FrameTreeEvent> = batches
        .iter()
        .flat_map(|batch| batch.events.iter())
        .filter(|event| matches!(event.event, TreeFrameInput::Ready))
        .collect();
    assert_eq!(ready.len(), 1, "one ready event: {batches:?}");
    assert_eq!(ready[0].node, WINDOW);
}

/// A window node the owner creates under the root, a valid target for a seat's `focus`.
const FOCUS_WINDOW: NodeId = NodeId::new(0x500);

/// A window node that is not a window (a plain `node`), which a seat's `focus` refuses.
const PLAIN_NODE: NodeId = NodeId::new(0x501);

/// A schema with a `seat` whose `focus` is a writable reference to a `window`.
struct FocusTable;

impl ClassInfo for FocusTable {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "seat" | "window")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        match (class, key) {
            ("seat", "name") => Some(PropertyInfo {
                kind: ValueType::Text,
                readonly: true,
                animated: false,
            }),
            ("seat", "focus") => Some(PropertyInfo {
                kind: ValueType::Node("window".to_owned()),
                readonly: false,
                animated: false,
            }),
            _ => None,
        }
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

fn focus_frame() -> TreeFrame {
    TreeFrame::with_hooks(TreeHooks {
        classes: Arc::new(FocusTable),
        publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
        ..TreeHooks::default()
    })
}

/// A transaction that creates a `seat` named `xr` and a `window` and a plain `node`.
fn create_focus_scene() -> TreeTransaction {
    TreeTransaction::new(vec![
        TreeOp::New {
            class: "seat".to_owned(),
            id: SEAT,
        },
        TreeOp::Set {
            node: SEAT,
            key: "name".to_owned(),
            value: TreeValue::Text("xr".to_owned()),
            timing: None,
        },
        TreeOp::New {
            class: "window".to_owned(),
            id: FOCUS_WINDOW,
        },
        TreeOp::New {
            class: "node".to_owned(),
            id: PLAIN_NODE,
        },
    ])
}

fn set_focus(target: Option<NodeId>) -> TreeTransaction {
    TreeTransaction::new(vec![TreeOp::Set {
        node: SEAT,
        key: "focus".to_owned(),
        value: TreeValue::Node(target),
        timing: None,
    }])
}

fn focus_of(composer: &mut Composer) -> Option<NodeId> {
    composer.with_scene(|scene| scene.reference(SEAT, "focus"))
}

#[test]
fn a_seat_focus_is_set_to_a_window_node_in_the_frame() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 1, set_focus(Some(FOCUS_WINDOW)), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(2, true)));
    assert_eq!(focus_of(&mut composer), Some(FOCUS_WINDOW));
}

#[test]
fn a_seat_focus_is_cleared_by_null() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 1, set_focus(Some(FOCUS_WINDOW)), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 2, set_focus(None), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(3, true)));
    assert_eq!(focus_of(&mut composer), None);
}

#[test]
fn a_seat_focus_on_a_non_window_node_is_refused_and_keeps_the_old_focus() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 1, set_focus(Some(FOCUS_WINDOW)), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 2, set_focus(Some(PLAIN_NODE)), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(2, false)));
    assert_eq!(focus_of(&mut composer), Some(FOCUS_WINDOW));
}

#[test]
fn a_seat_focus_on_an_unknown_node_is_refused_bad_value() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 1, set_focus(Some(NodeId::new(0x7ff))), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(1, false)));
    assert_eq!(focus_of(&mut composer), None);
}

#[test]
fn a_seat_focus_with_a_non_node_value_is_refused_wrong_type() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(
        &frame,
        1,
        TreeTransaction::new(vec![TreeOp::Set {
            node: SEAT,
            key: "focus".to_owned(),
            value: TreeValue::Text("window".to_owned()),
            timing: None,
        }]),
        &replies,
    );
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(1, false)));
    assert_eq!(focus_of(&mut composer), None);
}
