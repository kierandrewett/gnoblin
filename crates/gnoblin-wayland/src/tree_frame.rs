//! The tree's frame boundary: queued transactions are applied at the start of a frame,
//! whole or not at all, in arrival order, and each frame that changes the tree publishes
//! one batch.
//!
//! The frame path owns the order. [`TreeFrame::drain`] runs first, against the scene the
//! composer draws from. It takes the queue as it stands at the start of the drain: reads
//! see the last published serial, then transactions apply until the frame's budget is
//! spent. Anything that arrives after that snapshot waits for the next frame, so it is
//! never half-drawn. A transaction the budget defers is kept, whole, and is
//! applied first in the next frame.
//!
//! The sending side is [`TreeQueue`]: it is `Send` and cloneable, so the socket that
//! carries the shell's transactions can hold it on its own thread. The frame never waits
//! on it; the lock is held only to move queued work out or back.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use gnoblin_render::transaction::{
    Animation, ClassInfo, Code, PropertyInfo, SeatInput, TreeApplied, TreeCall, TreeChange, TreeRefusal,
    TreeTransaction, validate_and_apply,
};
use gnoblin_render::tree_revision::TreeRevisions;
use gnoblin_render::{Actor, Scene};
use gnoblin_types::{Keysym, NodeId};
use smithay::input::keyboard::xkb;

/// How long one frame may spend applying tree transactions. A transaction that starts
/// after the budget is spent waits for the next frame; the first transaction of a frame
/// always runs, so a transaction longer than the budget still lands.
pub const TREE_FRAME_BUDGET: Duration = Duration::from_millis(4);

/// What a transaction's reply receives: the serial after the frame it was answered in,
/// and what it changed or why it was refused.
type Reply = Box<dyn FnOnce(u64, Result<TreeApplied, TreeRefusal>) + Send>;

/// Whether the submitter may still change the tree, asked when its transaction applies.
type Admission = Box<dyn Fn() -> bool + Send>;

/// A read that runs at the start of a frame, before its transactions.
type Read = Box<dyn FnOnce(&mut Scene, u64) + Send>;

struct Submission {
    /// The serial the shell built the transaction against.
    base: u64,
    transaction: TreeTransaction,
    /// Asked when the transaction applies, not when it was queued: a holder that detached in
    /// between has lost the right to change the tree, and its queued work must not land.
    admitted: Admission,
    reply: Reply,
}

#[derive(Default)]
struct TreeQueued {
    reads: Vec<Read>,
    submissions: VecDeque<Submission>,
}

/// The changes of one frame that changed the tree, for the shell's mirror.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeBatch {
    /// The serial after the frame.
    pub serial: u64,
    /// Every applied change, in the order the frame applied them.
    pub changes: Vec<TreeChange>,
    /// Animated sets, for the compositor to play.
    pub animations: Vec<Animation>,
    /// Method calls, for the compositor to run.
    pub calls: Vec<TreeCall>,
    /// Events the frame delivers to the shell, in the order they arrived.
    pub events: Vec<FrameTreeEvent>,
}

/// A request a client made of its window, which the shell decides on instead of the window
/// manager. Its wire name is the `request` field of an `event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeRequest {
    Move,
    Resize,
    Maximize,
    Unmaximize,
    Fullscreen,
    Unfullscreen,
    Minimize,
    Menu,
}

impl TreeRequest {
    /// The name the shell reads in the `request` field.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Resize => "resize",
            Self::Maximize => "maximize",
            Self::Unmaximize => "unmaximize",
            Self::Fullscreen => "fullscreen",
            Self::Unfullscreen => "unfullscreen",
            Self::Minimize => "minimize",
            Self::Menu => "menu",
        }
    }
}

/// A pointer event for a shell node, in that node's local space. The shell decides what a press
/// or a scroll means; the compositor only reports where the ray landed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TreeInput {
    /// A button went down on the node.
    Press { x: f64, y: f64, button: u32 },
    /// A button came up on the node.
    Release { x: f64, y: f64, button: u32 },
    /// The ray moved over the node.
    Motion { x: f64, y: f64 },
    /// A wheel or touchpad scroll; `dx` and `dy` are pixel distances.
    Scroll { x: f64, y: f64, dx: f64, dy: f64 },
    /// The ray entered the node.
    Enter { x: f64, y: f64 },
    /// The ray left the node it was over.
    Leave,
    /// A key went down or up while the node holds seat keyboard focus. The keysym is the
    /// layout-resolved symbol; the shell reads it as `keysym` and `pressed` as the edge.
    Key { pressed: bool, keysym: Keysym },
}

impl TreeInput {
    /// The name the shell reads in the `kind` field.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Press { .. } => "press",
            Self::Release { .. } => "release",
            Self::Motion { .. } => "motion",
            Self::Scroll { .. } => "scroll",
            Self::Enter { .. } => "enter",
            Self::Leave => "leave",
            Self::Key { .. } => "key",
        }
    }
}

/// The xkbcommon name of a keysym (`Escape`, `a`, `KP_Enter`), the `key` a shell reads from a key
/// event. A keysym xkbcommon does not name reads as its hex value, as xkbcommon itself spells it.
#[must_use]
pub fn keysym_name(keysym: Keysym) -> String {
    xkb::keysym_get_name(xkb::Keysym::new(keysym.get()))
}

/// What a delivered event says: a client request that needs policy, or a pointer edge.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeFrameInput {
    /// A client request the shell decides on.
    Request(TreeRequest),
    /// A pointer edge over the node.
    Input(TreeInput),
    /// The node first has content. Sent once per window node, when it is created.
    Ready,
    /// A claimed chord changed state. The claim is the shell's, named by its `id`.
    Chord(ChordEvent),
}

/// The state a claimed chord changed to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordState {
    /// The chord was pressed.
    Press,
    /// The chord was released.
    Release,
}

/// A claimed chord changed state, for the shell that claimed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChordEvent {
    /// The claim's id, as the shell allocated it.
    pub id: i64,
    /// The chord, canonicalised by the compositor.
    pub chord: String,
    /// What changed.
    pub state: ChordState,
}

/// One event for a node. `frame` is the frame that delivered it; `seq` numbers the events of
/// one tree monotonically.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTreeEvent {
    pub frame: u64,
    pub seq: u64,
    pub node: NodeId,
    pub event: TreeFrameInput,
}

/// The sending side of the tree queue. Clones share one queue.
#[derive(Clone, Default)]
pub struct TreeQueue {
    inner: Arc<TreeShared>,
}

/// The queue's state, and the hook that wakes the frame loop when work is queued.
#[derive(Default)]
struct TreeShared {
    queued: Mutex<TreeQueued>,
    wake: OnceLock<Box<dyn Fn() + Send + Sync>>,
}

impl TreeQueue {
    /// Sets the hook that wakes the frame loop when work is queued. The first hook set stays;
    /// the session sets it once, before the socket starts queuing.
    pub fn on_submit(&self, wake: impl Fn() + Send + Sync + 'static) {
        let _ = self.inner.wake.set(Box::new(wake));
    }

    /// Queues `transaction` for the next frame that drains the queue. `base` is the serial
    /// the transaction was built against; `admitted` is asked when the transaction applies and
    /// refuses it `not-privileged` when it answers false; `reply` receives the serial after the
    /// frame that answered it, and the result. A transaction is answered in the frame that applies
    /// or refuses it; within a frame, replies keep arrival order.
    pub fn submit(
        &self,
        base: u64,
        transaction: TreeTransaction,
        admitted: impl Fn() -> bool + Send + 'static,
        reply: impl FnOnce(u64, Result<TreeApplied, TreeRefusal>) + Send + 'static,
    ) {
        self.lock().submissions.push_back(Submission {
            base,
            transaction,
            admitted: Box::new(admitted),
            reply: Box::new(reply),
        });
        self.wake();
    }

    /// Queues `read` to run at the start of the next frame, before that frame's
    /// transactions, with the scene as the last published batch left it and that serial.
    pub fn read(&self, read: impl FnOnce(&mut Scene, u64) + Send + 'static) {
        self.lock().reads.push(Box::new(read));
        self.wake();
    }

    fn wake(&self) {
        if let Some(wake) = self.inner.wake.get() {
            wake();
        }
    }

    fn lock(&self) -> MutexGuard<'_, TreeQueued> {
        self.inner
            .queued
            .lock()
            .expect("the tree queue lock is never held across a callback")
    }
}

/// What a compositor hands the tree when it starts: the queue the socket feeds, the schema
/// the transactions are checked against, and where each published batch goes.
#[derive(Clone)]
pub struct TreeHooks {
    /// The queue the shell's transactions and reads wait in.
    pub queue: TreeQueue,
    /// The schema the transactions are checked against.
    pub classes: Arc<dyn ClassInfo + Send + Sync>,
    /// Whether a holder is attached. Compositor changes drained while none is publish nothing.
    pub attached: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Receives each batch a frame publishes, in publication order, on the frame's thread.
    pub publish: Arc<dyn Fn(TreeBatch) + Send + Sync>,
}

impl std::fmt::Debug for TreeHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeHooks").finish_non_exhaustive()
    }
}

impl Default for TreeHooks {
    fn default() -> Self {
        Self {
            queue: TreeQueue::default(),
            classes: Arc::new(NoClasses),
            attached: Arc::new(|| true),
            publish: Arc::new(|_| {}),
        }
    }
}

/// The schema of a compositor that has not been configured: no class exists, so every
/// `new` is refused.
struct NoClasses;

impl ClassInfo for NoClasses {
    fn has_class(&self, _class: &str) -> bool {
        false
    }

    fn property(&self, _class: &str, _key: &str) -> Option<PropertyInfo> {
        None
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

/// The compositor's side of the tree: the queue, the schema the transactions are checked
/// against, the serial, and the hook each published batch goes to.
pub struct TreeFrame {
    queue: TreeQueue,
    classes: Arc<dyn ClassInfo + Send + Sync>,
    serial: u64,
    /// Receives each batch a frame publishes, before that frame's replies are sent.
    publish: Arc<dyn Fn(TreeBatch) + Send + Sync>,
    /// Whether a holder is attached; see [`TreeHooks::attached`].
    attached: Arc<dyn Fn() -> bool + Send + Sync>,
    /// The nodes each recent published batch changed, for the stale-base check.
    revisions: TreeRevisions,
    /// Transactions the budget deferred. Each is checked against its base when it applies, and
    /// runs first in the next frame.
    deferred: VecDeque<Submission>,
    /// Changes the compositor made itself since the last frame, announced in the next batch ahead of the shell's changes.
    compositor: Vec<TreeChange>,
    /// Request events recorded since the last frame, delivered in the next batch.
    events: Vec<FrameTreeEvent>,
    /// Pointer inputs the applied transactions sent since the compositor last took them, in op order.
    inputs: Vec<SeatInput>,
    /// The sequence number of the last event recorded.
    next_seq: u64,
    /// The number of frames drained; an event carries the frame that delivered it.
    frames: u64,
    /// Seat `claim` and `unclaim` calls the applied transactions made, in op order, for the
    /// compositor to apply.
    claims: Vec<TreeCall>,
    /// Nodes the published batches destroyed, with the subtrees they took, since the compositor last
    /// took them. A destroyed seat takes its claims with it.
    destroyed: Vec<NodeId>,
}

impl Default for TreeFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeFrame {
    /// An empty tree at serial 0 with the hooks the compositor configured.
    #[must_use]
    pub fn with_hooks(hooks: TreeHooks) -> Self {
        Self {
            queue: hooks.queue,
            classes: hooks.classes,
            publish: hooks.publish,
            attached: hooks.attached,
            serial: 0,
            deferred: VecDeque::new(),
            compositor: Vec::new(),
            events: Vec::new(),
            inputs: Vec::new(),
            next_seq: 0,
            frames: 0,
            claims: Vec::new(),
            destroyed: Vec::new(),
            revisions: TreeRevisions::new(),
        }
    }

    /// An empty tree at serial 0 that refuses every class and publishes nowhere.
    #[must_use]
    pub fn new() -> Self {
        Self::with_hooks(TreeHooks::default())
    }

    /// A handle for queuing reads and transactions. It may be held off the frame's thread.
    #[must_use]
    pub fn queue(&self) -> TreeQueue {
        self.queue.clone()
    }

    /// The serial of the last published batch.
    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    /// Records a change the compositor made to the scene itself. It is published in the next
    /// batch that a frame builds, ahead of the shell's changes in that frame.
    pub fn record(&mut self, change: TreeChange) {
        self.compositor.push(change);
    }

    /// Records a request event for `node`, delivered to the shell in the next batch.
    pub fn event(&mut self, node: NodeId, request: TreeRequest) {
        self.push_event(node, TreeFrameInput::Request(request));
    }

    /// Records that `node` has content for the first time, delivered to the shell in the next batch.
    pub fn ready(&mut self, node: NodeId) {
        self.push_event(node, TreeFrameInput::Ready);
    }

    /// Records a claimed chord changing state on the seat `node`, delivered to the shell in the
    /// next batch.
    pub fn chord(&mut self, node: NodeId, event: ChordEvent) {
        self.push_event(node, TreeFrameInput::Chord(event));
    }

    /// Records a pointer edge over `node`, delivered to the shell in the next batch.
    pub fn input(&mut self, node: NodeId, input: TreeInput) {
        self.push_event(node, TreeFrameInput::Input(input));
    }

    fn push_event(&mut self, node: NodeId, event: TreeFrameInput) {
        self.next_seq += 1;
        self.events.push(FrameTreeEvent {
            frame: 0,
            seq: self.next_seq,
            node,
            event,
        });
    }

    /// Take the pointer inputs the applied transactions sent, in op order, for the compositor to
    /// deliver to clients.
    pub fn take_inputs(&mut self) -> Vec<SeatInput> {
        std::mem::take(&mut self.inputs)
    }

    /// Take the seat `claim` and `unclaim` calls the applied transactions made, in op order, for
    /// the compositor to apply.
    pub fn take_claims(&mut self) -> Vec<TreeCall> {
        std::mem::take(&mut self.claims)
    }

    /// Take the nodes the published batches destroyed since the last take, for the compositor to
    /// drop the seat claims they held.
    pub fn take_destroyed(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.destroyed)
    }

    /// Runs the frame's tree work against `scene`: reads, then transactions in arrival
    /// order until the budget is spent, then one batch if anything changed, then the
    /// replies. `now` is read once at the start, and once before each transaction after
    /// the first, so a test can drive the budget with a clock of its own.
    pub fn drain(&mut self, scene: &mut Scene, now: impl Fn() -> Instant) {
        let start = now();
        self.frames += 1;
        let frame = self.frames;
        // The compositor's own changes (seat and screen nodes) are already in the scene. With a
        // holder attached they are announced in this frame's batch, ahead of the shell's changes,
        // so the holder hears of them once. With none, nobody has heard of them: the attach
        // snapshot read carries them, so they are dropped here and bump no serial.
        let announced = std::mem::take(&mut self.compositor);
        let announced = if (self.attached)() { announced } else { Vec::new() };
        let frame_serial = self.serial;
        let (reads, arrived) = {
            let mut queue = self.queue.lock();
            (std::mem::take(&mut queue.reads), std::mem::take(&mut queue.submissions))
        };
        for read in reads {
            read(&mut *scene, frame_serial);
        }

        // Deferred work came first, then arrivals in arrival order. Every transaction is checked
        // when it applies, against the batches published since its base and the changes this
        // frame has already applied. A stale one that conflicts is refused here; one that does not
        // conflict still lands.
        let mut queue: VecDeque<Submission> = self.deferred.drain(..).chain(arrived).collect();
        // Compositor changes lead the batch, ahead of the shell's transaction changes.
        let mut batch = TreeBatch {
            changes: announced,
            ..TreeBatch::default()
        };
        batch.events = std::mem::take(&mut self.events)
            .into_iter()
            .map(|event| FrameTreeEvent { frame, ..event })
            .collect();
        let mut changed = !batch.changes.is_empty();
        let mut attempted = 0usize;
        let mut spent = false;
        let mut replies: Vec<(Reply, Result<TreeApplied, TreeRefusal>)> = Vec::new();
        while let Some(submission) = queue.pop_front() {
            // The first transaction of a frame always runs; after it, the budget decides.
            spent = spent || (attempted > 0 && now().saturating_duration_since(start) >= TREE_FRAME_BUDGET);
            if spent {
                self.deferred.push_back(submission);
                continue;
            }
            attempted += 1;
            let result = if !(submission.admitted)() {
                Err(TreeRefusal {
                    failed_op: -1,
                    code: Code::NotPrivileged,
                })
            } else {
                match self
                    .revisions
                    .check(submission.base, self.serial, &batch.changes, &submission.transaction)
                {
                    Ok(()) => validate_and_apply(scene, self.classes.as_ref(), Actor::Shell, &submission.transaction),
                    Err(refusal) => Err(refusal),
                }
            };
            match result {
                Ok(mut done) => {
                    changed |= !(done.changes.is_empty() && done.animations.is_empty() && done.calls.is_empty());
                    self.inputs.extend(done.inputs.iter().copied());
                    // The calls move into the batch and the seat claims out to the compositor: each
                    // call has exactly one owner from here, so a password is never copied out of it.
                    let (claims, calls): (Vec<TreeCall>, Vec<TreeCall>) =
                        std::mem::take(&mut done.calls).into_iter().partition(|call| {
                            scene.class(call.node) == Some("seat")
                                && matches!(call.method.as_str(), "claim" | "unclaim")
                        });
                    self.claims.extend(claims);
                    batch.changes.extend(done.changes.iter().cloned());
                    batch.animations.extend(done.animations.iter().cloned());
                    batch.calls.extend(calls);
                    replies.push((submission.reply, Ok(done)));
                }
                Err(refusal) => replies.push((submission.reply, Err(refusal))),
            }
        }
        // The compositor announced its own destroys and the shell's applied ones alike; both
        // leave the seat claims they held with nothing to fire. An event for a node this frame
        // destroyed is dropped at send (F30): the shell hears of the destroy, not of the input.
        let mut gone: Vec<NodeId> = Vec::new();
        for change in &batch.changes {
            if let TreeChange::Destroyed { removed, .. } = change {
                gone.extend(removed.iter().cloned());
            }
        }
        if !gone.is_empty() {
            batch.events.retain(|event| !gone.contains(&event.node));
        }
        self.destroyed.extend(gone);
        if changed {
            self.serial += 1;
            self.revisions.record(self.serial, &batch.changes);
        }
        if changed || !batch.events.is_empty() {
            batch.serial = self.serial;
            (self.publish)(batch);
        }
        for (reply, result) in replies {
            reply(self.serial, result);
        }
    }
}
