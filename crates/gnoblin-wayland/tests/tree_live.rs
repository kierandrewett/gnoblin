//! The tree on the composer's live scene. A node the composer creates is visible to a read
//! at the frame boundary, a shell transaction applies in that frame and is answered after
//! it, a refused one changes nothing, and a change the compositor recorded is announced in
//! the batch ahead of the shell's own changes.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gnoblin_backend::render::compose::Composer;
use gnoblin_render::transaction::{
    ClassInfo, PropertyInfo, ShellTrackEnd, Timing, TreeChange, TreeOp, TreeTransaction, TreeValue, ValueType,
};
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

/// A plain `node` with `input` off, which a seat's `focus` refuses: it is not a window and not a shell input node.
const PLAIN_NODE: NodeId = NodeId::new(0x501);

/// A plain `node` with `input` on, a shell node that a seat may focus to receive keys.
const SHELL_NODE: NodeId = NodeId::new(0x502);

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
            ("node", "input") => Some(PropertyInfo {
                kind: ValueType::Bool,
                readonly: false,
                animated: false,
            }),
            ("seat", "focus") => Some(PropertyInfo {
                kind: ValueType::Node("node".to_owned()),
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
        TreeOp::Set {
            node: PLAIN_NODE,
            key: "input".to_owned(),
            value: TreeValue::Bool(false),
            timing: None,
        },
        TreeOp::New {
            class: "node".to_owned(),
            id: SHELL_NODE,
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
fn a_seat_focus_on_a_shell_node_with_input_on_is_accepted_and_keeps_the_node() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    submit(&frame, 1, set_focus(Some(SHELL_NODE)), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(2, true)));
    assert_eq!(focus_of(&mut composer), Some(SHELL_NODE));
}

#[test]
fn a_shell_node_that_holds_the_seat_focus_is_the_key_target_until_it_is_released() {
    let mut frame = focus_frame();
    let mut composer = Composer::new();
    let main = composer.add_main_seat("main").expect("a main seat");
    let replies = Replies::default();
    submit(&frame, 0, create_focus_scene(), &replies);
    drain(&mut frame, &mut composer);
    let focus_main = |target| {
        TreeTransaction::new(vec![TreeOp::Set {
            node: main,
            key: "focus".to_owned(),
            value: TreeValue::Node(target),
            timing: None,
        }])
    };
    submit(&frame, 1, focus_main(Some(SHELL_NODE)), &replies);
    drain(&mut frame, &mut composer);
    assert_eq!(replies.lock().unwrap().last(), Some(&(2, true)));
    assert!(composer.main_seat_focus_written_by_shell());
    composer.hold_seat_focus_for_shell();
    assert_eq!(composer.shell_key_target(), Some(SHELL_NODE));

    composer.set_main_seat_focus(None);
    assert_eq!(
        composer.main_seat_focus(),
        Some(SHELL_NODE),
        "a publish of no window does not take the shell's focus back"
    );
    assert_eq!(composer.shell_key_target(), Some(SHELL_NODE));

    composer.release_shell_focus();
    composer.set_main_seat_focus(None);
    assert_eq!(composer.main_seat_focus(), None);
    assert_eq!(composer.shell_key_target(), None);
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

/// A schema whose `node` class has the animated doubles the shell tweens (`x`, `opacity` and `width`), and a `tween`
/// with the play state of the tracks that join it.
struct Animated;

impl ClassInfo for Animated {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "tween")
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        match (class, key) {
            ("node", "x" | "y" | "opacity" | "width") => Some(PropertyInfo {
                kind: ValueType::Double,
                readonly: false,
                animated: true,
            }),
            ("tween", "progress") => Some(PropertyInfo {
                kind: ValueType::Double,
                readonly: false,
                animated: false,
            }),
            ("tween", "paused" | "reverse") => Some(PropertyInfo {
                kind: ValueType::Bool,
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

/// A tween's value is interpolated in single precision, so a midpoint is compared within a hair.
fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-3, "{what}: {actual} is not {expected}");
}

/// A composer and a frame over [`Animated`], as the compositor drives them: the frame drains, then the
/// composer applies the motion the drain left.
struct Live {
    composer: Composer,
    frame: TreeFrame,
    replies: Replies,
    serial: u64,
}

impl Live {
    fn new() -> Self {
        Self {
            composer: composer_with_window(),
            frame: TreeFrame::with_hooks(TreeHooks {
                classes: Arc::new(Animated),
                publish: Arc::new(|batch| PUBLISHED.with(|published| published.borrow_mut().push(batch))),
                ..TreeHooks::default()
            }),
            replies: Arc::default(),
            serial: 0,
        }
    }

    /// One frame in which the shell writes `key` on the window: tweened over `seconds` when given,
    /// at once otherwise. Whether the write applied.
    fn write(&mut self, key: &str, value: f64, seconds: Option<f64>) -> bool {
        let timing = seconds.map(|seconds| Timing::new(seconds, vec![0.0, 0.5, 1.0]));
        self.apply(vec![TreeOp::Set {
            node: WINDOW,
            key: key.to_owned(),
            value: TreeValue::Double(value),
            timing,
        }])
    }

    /// One frame in which the shell sends `ops` as one transaction. Whether it applied.
    fn apply(&mut self, ops: Vec<TreeOp>) -> bool {
        submit(&self.frame, self.serial, TreeTransaction::new(ops), &self.replies);
        let Self { composer, frame, .. } = self;
        composer.with_scene(|scene| frame.drain(scene, Instant::now));
        let motion = frame.take_motion();
        composer.apply_shell_motion(motion);
        let (serial, applied) = *self
            .replies
            .lock()
            .expect("the reply lock is not poisoned")
            .last()
            .expect("the frame answered");
        self.serial = serial;
        applied
    }

    fn x(&mut self) -> f64 {
        self.composer
            .with_scene(|scene| scene.transform(WINDOW).map(|transform| transform.base.offset.x))
            .expect("the window node has a transform")
    }

    fn width(&mut self) -> u32 {
        self.composer
            .with_scene(|scene| scene.bounds(WINDOW).map(|bounds| bounds.width))
            .expect("the window node has bounds")
    }
}

#[test]
fn with_reduced_motion_on_a_timed_set_lands_at_once_and_registers_no_track() {
    let mut live = Live::new();
    live.composer.set_reduced_motion(true);

    assert!(live.write("x", 40.0, Some(0.5)));

    assert_eq!(live.x(), 40.0, "the node holds the final value on the next frame");
    assert_eq!(live.composer.shell_tracks_running(), 0, "no track was registered");
    live.composer.advance_motion(500);
    assert_eq!(live.x(), 40.0, "and nothing moves it afterwards");
    take_batches();
}

#[test]
fn every_animated_property_lands_at_once_under_reduced_motion() {
    let mut live = Live::new();
    live.composer.set_reduced_motion(true);

    assert!(live.write("opacity", 0.25, Some(1.0)));
    assert!(live.write("width", 80.0, Some(1.0)));
    assert!(live.write("y", 12.0, Some(1.0)));

    assert_eq!(opacity_of(&mut live.composer), 0.25);
    assert_eq!(live.width(), 80);
    let y = live
        .composer
        .with_scene(|scene| scene.transform(WINDOW).map(|transform| transform.base.offset.y));
    assert_eq!(y, Some(12.0));
    assert_eq!(live.composer.shell_tracks_running(), 0);
    take_batches();
}

#[test]
fn with_reduced_motion_off_a_timed_set_runs_a_tween_to_its_target() {
    let mut live = Live::new();

    assert!(live.write("x", 40.0, Some(0.5)));

    assert_eq!(live.x(), 0.0, "the node starts where it was");
    assert_eq!(live.composer.shell_tracks_running(), 1, "one track runs");
    live.composer.advance_motion(250);
    assert_near(live.x(), 20.0, "halfway through, the node is halfway there");
    live.composer.advance_motion(250);
    assert_eq!(live.x(), 40.0, "the tween lands on its target");
    assert_eq!(live.composer.shell_tracks_running(), 0, "and its track is gone");
    take_batches();
}

#[test]
fn a_size_tweens_in_whole_pixels_and_lands_on_its_target() {
    let mut live = Live::new();
    let start = live.width();
    let target = start + 100;

    assert!(live.write("width", f64::from(target), Some(0.5)));

    assert_eq!(live.width(), start, "the node starts at its size");
    assert_eq!(live.composer.shell_tracks_running(), 1, "a width is a track");
    live.composer.advance_motion(250);
    assert_eq!(live.width(), start + 50, "halfway through the size is halfway there");
    live.composer.advance_motion(250);
    assert_eq!(live.width(), target, "and it lands on its target");
    assert_eq!(live.composer.shell_tracks_running(), 0);
    take_batches();
}

#[test]
fn toggling_reduced_motion_at_runtime_changes_the_next_write_only() {
    let mut live = Live::new();
    assert!(live.write("x", 40.0, Some(1.0)));
    live.composer.advance_motion(500);
    assert_near(live.x(), 20.0, "mid-tween");

    live.composer.set_reduced_motion(true);
    live.composer.advance_motion(250);
    assert_near(live.x(), 30.0, "a track already running finishes");
    assert_eq!(live.composer.shell_tracks_running(), 1);

    assert!(live.write("x", 100.0, Some(1.0)));
    assert_eq!(live.x(), 100.0, "the next write lands at once and ends the old track");
    assert_eq!(live.composer.shell_tracks_running(), 0);
    live.composer.advance_motion(1000);
    assert_eq!(live.x(), 100.0, "the old track does not come back");

    live.composer.set_reduced_motion(false);
    assert!(live.write("x", 0.0, Some(1.0)));
    assert_eq!(live.x(), 100.0, "the write after the toggle back tweens again");
    assert_eq!(live.composer.shell_tracks_running(), 1);
    live.composer.advance_motion(1000);
    assert_eq!(live.x(), 0.0);
    take_batches();
}

#[test]
fn a_set_without_a_duration_ends_the_tween_running_on_its_property() {
    let mut live = Live::new();
    assert!(live.write("x", 40.0, Some(1.0)));
    live.composer.advance_motion(250);

    assert!(live.write("x", 7.0, None));

    assert_eq!(live.x(), 7.0, "the plain set lands");
    assert_eq!(
        live.composer.shell_tracks_running(),
        0,
        "and the tween no longer owns the property"
    );
    live.composer.advance_motion(1000);
    assert_eq!(live.x(), 7.0, "the tween does not overwrite it");
    take_batches();
}

#[test]
fn a_second_timed_set_replaces_the_tween_and_starts_from_where_the_node_is() {
    let mut live = Live::new();
    assert!(live.write("x", 40.0, Some(1.0)));
    live.composer.advance_motion(500);
    assert_near(live.x(), 20.0, "mid-tween");

    assert!(live.write("x", 0.0, Some(1.0)));

    assert_near(live.x(), 20.0, "the new tween starts from the drawn value");
    assert_eq!(live.composer.shell_tracks_running(), 1, "one track owns the property");
    live.composer.advance_motion(500);
    assert_near(live.x(), 10.0, "mid-tween");
    take_batches();
}

/// The `tween` node the scrub tests make.
const TWEEN: NodeId = NodeId::new(0x600);

/// The ops that make [`TWEEN`], optionally held, and a timed write of the window's `x` that joins it.
fn tween_and_track(paused: bool, to: f64, seconds: f64, delay: f64) -> Vec<TreeOp> {
    let mut ops = vec![TreeOp::New {
        class: "tween".to_owned(),
        id: TWEEN,
    }];
    if paused {
        ops.push(tween_set("paused", TreeValue::Bool(true)));
    }
    ops.push(TreeOp::Set {
        node: WINDOW,
        key: "x".to_owned(),
        value: TreeValue::Double(to),
        timing: Some(Timing {
            seconds,
            delay,
            group: Some(TWEEN),
            samples: vec![0.0, 0.5, 1.0],
        }),
    });
    ops
}

fn tween_set(key: &str, value: TreeValue) -> TreeOp {
    TreeOp::Set {
        node: TWEEN,
        key: key.to_owned(),
        value,
        timing: None,
    }
}

#[test]
fn a_scrub_moves_the_presented_position_and_the_end_is_reported_once() {
    let mut live = Live::new();
    assert!(live.apply(tween_and_track(true, 100.0, 1.0, 0.0)));
    assert_eq!(
        live.x(),
        0.0,
        "a track that joins a held tween starts held at its start value"
    );
    assert_eq!(live.composer.shell_tracks_running(), 1);
    // The scrub: 0 -> 0.4 -> 0.1 -> 1. Each is one write, and the pose follows it at once.
    for (progress, expected) in [(0.4, 40.0), (0.1, 10.0), (1.0, 100.0)] {
        assert!(live.apply(vec![tween_set("progress", TreeValue::Double(progress))]));
        assert_near(live.x(), expected, "the presented position at the scrubbed progress");
    }
    // Time does not move a held track, and a held track at 1 has not ended.
    live.composer.advance_motion(5000);
    assert_near(live.x(), 100.0, "held at the target");
    assert_eq!(live.composer.shell_tracks_running(), 1);
    assert!(live.composer.take_shell_track_ends().is_empty(), "nothing ended yet");
    // Let it go: it ends on the next frame, once.
    assert!(live.apply(vec![tween_set("paused", TreeValue::Bool(false))]));
    live.composer.advance_motion(16);
    assert_eq!(
        live.composer.take_shell_track_ends(),
        [ShellTrackEnd {
            node: WINDOW,
            key: "x".to_owned(),
            group: Some(TWEEN),
            completed: true,
        }]
    );
    live.composer.advance_motion(1000);
    assert!(live.composer.take_shell_track_ends().is_empty(), "reported once");
    take_batches();
}

#[test]
fn a_reversed_group_runs_back_to_the_start_and_reports_cancelled() {
    let mut live = Live::new();
    assert!(live.apply(tween_and_track(false, 100.0, 1.0, 0.0)));
    live.composer.advance_motion(600);
    assert_near(live.x(), 60.0, "forward");
    assert!(live.apply(vec![tween_set("reverse", TreeValue::Bool(true))]));
    live.composer.advance_motion(300);
    assert_near(live.x(), 30.0, "back along the same path");
    live.composer.advance_motion(400);
    assert_eq!(live.x(), 0.0, "the start value, exactly");
    let ends = live.composer.take_shell_track_ends();
    assert_eq!(ends.len(), 1);
    assert!(
        !ends[0].completed,
        "a track run back to its start is cancelled, not completed"
    );
    take_batches();
}

#[test]
fn a_delayed_write_holds_its_start_value_and_a_plain_write_ends_it_cancelled() {
    let mut live = Live::new();
    assert!(live.apply(tween_and_track(false, 100.0, 1.0, 0.5)));
    live.composer.advance_motion(400);
    assert_eq!(live.x(), 0.0, "still in the delay");
    live.composer.advance_motion(200);
    assert_near(live.x(), 10.0, "100 ms into the animation");
    assert!(live.write("x", 7.0, None));
    let ends = live.composer.take_shell_track_ends();
    assert_eq!(ends.len(), 1, "the shell's own write ends the track once: {ends:?}");
    assert!(!ends[0].completed);
    assert_eq!(live.x(), 7.0);
    take_batches();
}
