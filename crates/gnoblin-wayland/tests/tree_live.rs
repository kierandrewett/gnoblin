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
use gnoblin_wayland::{TreeBatch, TreeFrame, TreeHooks};

/// A schema with one class, `node`, holding one writable double, `opacity`.
struct Table;

impl ClassInfo for Table {
    fn has_class(&self, class: &str) -> bool {
        class == "node"
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        (class == "node" && key == "opacity").then_some(PropertyInfo {
            kind: ValueType::Double,
            readonly: false,
            animated: false,
        })
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
    frame.queue().submit(base, transaction, move |serial, result| {
        replies
            .lock()
            .expect("the reply lock is not poisoned")
            .push((serial, result.is_ok()));
    });
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
