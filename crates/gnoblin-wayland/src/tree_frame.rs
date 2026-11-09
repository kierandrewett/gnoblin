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
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gnoblin_render::transaction::{
    Animation, ClassInfo, Code, PropertyInfo, TreeApplied, TreeCall, TreeChange, TreeRefusal, TreeTransaction,
    validate_and_apply,
};
use gnoblin_render::{Actor, Scene};

/// How long one frame may spend applying tree transactions. A transaction that starts
/// after the budget is spent waits for the next frame; the first transaction of a frame
/// always runs, so a transaction longer than the budget still lands.
pub const TREE_FRAME_BUDGET: Duration = Duration::from_millis(4);

/// What a transaction's reply receives: the serial after the frame it was answered in,
/// and what it changed or why it was refused.
type Reply = Box<dyn FnOnce(u64, Result<TreeApplied, TreeRefusal>) + Send>;

/// A read that runs at the start of a frame, before its transactions.
type Read = Box<dyn FnOnce(&Scene, u64) + Send>;

struct Submission {
    /// The serial the shell built the transaction against.
    base: u64,
    transaction: TreeTransaction,
    reply: Reply,
}

/// One queued transaction, in the order it is answered: a refusal decided at the frame's
/// start, or a transaction the frame may apply.
enum TreeSlot {
    Refused(Reply, TreeRefusal),
    Admitted(Submission),
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
    inner: Arc<Mutex<TreeQueued>>,
}

impl TreeQueue {
    /// Queues `transaction` for the next frame that drains the queue. `base` is the serial
    /// the transaction was built against; `reply` receives the serial after the frame that
    /// answered it, and the result. A transaction is answered in the frame that applies or
    /// refuses it; within a frame, replies keep arrival order.
    pub fn submit(
        &self,
        base: u64,
        transaction: TreeTransaction,
        reply: impl FnOnce(u64, Result<TreeApplied, TreeRefusal>) + Send + 'static,
    ) {
        self.lock().submissions.push_back(Submission {
            base,
            transaction,
            reply: Box::new(reply),
        });
    }

    /// Queues `read` to run at the start of the next frame, before that frame's
    /// transactions, with the scene as the last published batch left it and that serial.
    pub fn read(&self, read: impl FnOnce(&Scene, u64) + Send + 'static) {
        self.lock().reads.push(Box::new(read));
    }

    fn lock(&self) -> MutexGuard<'_, TreeQueued> {
        self.inner
            .lock()
            .expect("the tree queue lock is never held across a callback")
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
/// against, the serial, and the batches the shell has not taken yet.
pub struct TreeFrame {
    queue: TreeQueue,
    classes: Box<dyn ClassInfo>,
    serial: u64,
    batches: Vec<TreeBatch>,
    /// Transactions the budget deferred. Each was checked against its base when it arrived,
    /// and runs first in the next frame.
    deferred: VecDeque<Submission>,
}

impl Default for TreeFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeFrame {
    /// An empty tree at serial 0 that refuses every class until [`TreeFrame::set_classes`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: TreeQueue::default(),
            classes: Box::new(NoClasses),
            serial: 0,
            batches: Vec::new(),
            deferred: VecDeque::new(),
        }
    }

    /// A handle for queuing reads and transactions. It may be held off the frame's thread.
    #[must_use]
    pub fn queue(&self) -> TreeQueue {
        self.queue.clone()
    }

    /// Sets the schema transactions are checked against.
    pub fn set_classes(&mut self, classes: Box<dyn ClassInfo>) {
        self.classes = classes;
    }

    /// The serial of the last published batch.
    #[must_use]
    pub const fn serial(&self) -> u64 {
        self.serial
    }

    /// Takes every batch published since the last take, in publication order.
    pub fn take_batches(&mut self) -> Vec<TreeBatch> {
        std::mem::take(&mut self.batches)
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
            read(scene, frame_serial);
        }

        // Deferred work came first and was checked when it arrived. Arrivals are checked now,
        // against the serial this frame starts from; a stale one is refused here and never
        // deferred, so no later frame re-checks it against a serial it was not built for.
        let mut slots: VecDeque<TreeSlot> = self.deferred.drain(..).map(TreeSlot::Admitted).collect();
        for submission in arrived {
            if submission.base == frame_serial {
                slots.push_back(TreeSlot::Admitted(submission));
            } else {
                let refusal = TreeRefusal {
                    failed_op: -1,
                    code: Code::StaleBase,
                };
                slots.push_back(TreeSlot::Refused(submission.reply, refusal));
            }
        }

        let mut batch = TreeBatch::default();
        let mut changed = false;
        let mut attempted = 0usize;
        let mut spent = false;
        let mut replies: Vec<(Reply, Result<TreeApplied, TreeRefusal>)> = Vec::new();
        for slot in slots {
            match slot {
                TreeSlot::Refused(reply, refusal) => replies.push((reply, Err(refusal))),
                TreeSlot::Admitted(submission) => {
                    // The first transaction of a frame always runs; after it, the budget decides.
                    spent = spent || (attempted > 0 && now().saturating_duration_since(start) >= TREE_FRAME_BUDGET);
                    if spent {
                        self.deferred.push_back(submission);
                        continue;
                    }
                    attempted += 1;
                    match validate_and_apply(scene, self.classes.as_ref(), Actor::Shell, &submission.transaction) {
                        Ok(done) => {
                            changed |=
                                !(done.changes.is_empty() && done.animations.is_empty() && done.calls.is_empty());
                            batch.changes.extend(done.changes.iter().cloned());
                            batch.animations.extend(done.animations.iter().cloned());
                            batch.calls.extend(done.calls.iter().cloned());
                            replies.push((submission.reply, Ok(done)));
                        }
                        Err(refusal) => replies.push((submission.reply, Err(refusal))),
                    }
                }
            }
        }

        if changed {
            self.serial += 1;
            batch.serial = self.serial;
            self.batches.push(batch);
        }
        for (reply, result) in replies {
            reply(self.serial, result);
        }
    }
}
