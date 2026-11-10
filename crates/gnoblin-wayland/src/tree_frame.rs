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
    Animation, ClassInfo, Code, PropertyInfo, TreeApplied, TreeCall, TreeChange, TreeRefusal, TreeTransaction,
    validate_and_apply,
};
use gnoblin_render::tree_revision::TreeRevisions;
use gnoblin_render::{Actor, Scene};

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
    /// The nodes each recent published batch changed, for the stale-base check.
    revisions: TreeRevisions,
    /// Transactions the budget deferred. Each is checked against its base when it applies, and
    /// runs first in the next frame.
    deferred: VecDeque<Submission>,
    /// Changes the compositor made itself since the last frame, announced in the next batch ahead of the shell's changes.
    compositor: Vec<TreeChange>,
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
            serial: 0,
            deferred: VecDeque::new(),
            compositor: Vec::new(),
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

    /// Runs the frame's tree work against `scene`: reads, then transactions in arrival
    /// order until the budget is spent, then one batch if anything changed, then the
    /// replies. `now` is read once at the start, and once before each transaction after
    /// the first, so a test can drive the budget with a clock of its own.
    pub fn drain(&mut self, scene: &mut Scene, now: impl Fn() -> Instant) {
        let start = now();
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
        // The compositor's own changes come first, so the shell sees a node before any change to it.
        let mut batch = TreeBatch {
            changes: std::mem::take(&mut self.compositor),
            ..TreeBatch::default()
        };
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
                Ok(done) => {
                    changed |= !(done.changes.is_empty() && done.animations.is_empty() && done.calls.is_empty());
                    batch.changes.extend(done.changes.iter().cloned());
                    batch.animations.extend(done.animations.iter().cloned());
                    batch.calls.extend(done.calls.iter().cloned());
                    replies.push((submission.reply, Ok(done)));
                }
                Err(refusal) => replies.push((submission.reply, Err(refusal))),
            }
        }

        if changed {
            self.serial += 1;
            batch.serial = self.serial;
            self.revisions.record(self.serial, &batch.changes);
            (self.publish)(batch);
        }
        for (reply, result) in replies {
            reply(self.serial, result);
        }
    }
}
