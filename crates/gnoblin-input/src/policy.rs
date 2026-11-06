//! The script policy queue: which input events wait for a script decision, and what
//! happens when the decision is late.
//!
//! `design/lua-input-policy.md` is explicit: the compositor never blocks waiting for
//! Lua. Events that a handler matches are copied into a bounded FIFO with their
//! physical identity retained, the script decides once, and a deadline retires
//! anything still waiting so delivery resumes. A late reply cannot act on a retired
//! event.
//!
//! This module is that bookkeeping and nothing else. It holds request identities and
//! deadlines, not events: the caller keeps each event and resumes it when the queue
//! reports the request retired, so the queue cannot leak, reorder or duplicate an
//! event it never owned.
//!
//! Three properties are load-bearing:
//!
//! - **Nothing waits.** Every method returns immediately. A caller that must decide now
//!   calls [`PolicyQueue::expire`] first, which retires everything past its deadline.
//! - **Retirement is in submission order.** A later request cannot be retired while an
//!   earlier one still waits, because resuming out of order would let a later pointer
//!   event overtake the earlier one it depends on. A short budget on a later request
//!   therefore cannot cut in front of an earlier one.
//! - **The fallback is normal delivery, and it is recorded.** An expired request
//!   resumes as [`Decision::Forward`] and the expiry is counted, which is what lets the
//!   session report that the script policy was bypassed instead of silently changing
//!   behaviour until the next reload.

use std::collections::VecDeque;

use gnoblin_types::{RequestId, Tick};

/// How many requests may wait at once, matching the C tree's bounded queue.
///
/// A full queue forwards immediately instead of growing: an unbounded queue would turn a
/// stuck script into unbounded memory, and dropping an event would lose input.
pub const MAX_PENDING: usize = 256;

/// What a script decided about one event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Decision {
    /// Let the event continue to its normal target.
    #[default]
    Forward,
    /// Withhold the event from its normal target.
    Consume,
}

/// What submitting an event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Submitted {
    /// The event waits for a decision until its deadline.
    Queued {
        /// The identity to correlate the reply with.
        id: RequestId,
        /// The tick by which a decision must arrive.
        deadline: Tick,
    },
    /// The queue was full, so the event was not queued. The caller delivers it as if no
    /// handler had matched.
    Bypassed,
}

/// What a reply did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The decision arrived in time and applies to the event.
    Applied {
        /// The request.
        id: RequestId,
        /// The decision.
        decision: Decision,
    },
    /// The request was already retired, by expiry or by a reload, so the decision
    /// cannot act. The recorded count is how a session notices a script that keeps
    /// replying too late.
    Late {
        /// The request.
        id: RequestId,
    },
    /// No such request was issued by this queue, which means the reply is broken or
    /// belongs to a previous session.
    Unknown {
        /// The claimed request.
        id: RequestId,
    },
}

/// A request retired without a decision, whose event the caller must resume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expired {
    /// The request that expired.
    pub id: RequestId,
    /// How many ticks late it was, for logging.
    pub lateness: u64,
}

/// Counters a session can report, and the model test checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyStats {
    /// Requests queued.
    pub queued: u64,
    /// Requests a script decided in time.
    pub applied: u64,
    /// Requests retired by their deadline.
    pub expired: u64,
    /// Replies that arrived after retirement.
    pub late: u64,
    /// Replies that named no issued request.
    pub unknown: u64,
    /// Events forwarded because the queue was full.
    pub bypassed: u64,
    /// Requests retired by a reload or a worker failure.
    pub retired: u64,
}

/// A request as the queue remembers it: its identity and the tick it must be decided by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Waiting {
    id: RequestId,
    deadline: Tick,
}

/// The queue of events waiting for a script decision.
///
/// The queue keeps request identities and deadlines only. Events stay with the caller.
#[derive(Clone, Debug)]
pub struct PolicyQueue {
    capacity: usize,
    waiting: VecDeque<Waiting>,
    next_id: u64,
    stats: PolicyStats,
}

/// The default queue uses [`MAX_PENDING`], the same bound the C tree enforces.
impl Default for PolicyQueue {
    fn default() -> Self {
        Self::new(MAX_PENDING)
    }
}

impl PolicyQueue {
    /// A queue that holds at most `capacity` requests. Zero is raised to one, because a
    /// queue that always bypasses would silently disable the script policy.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            waiting: VecDeque::new(),
            next_id: 0,
            stats: PolicyStats::default(),
        }
    }

    /// Queue an event whose decision is due within `budget` ticks.
    pub fn submit(&mut self, now: Tick, budget: u64) -> Submitted {
        if self.waiting.len() >= self.capacity {
            self.stats.bypassed += 1;
            return Submitted::Bypassed;
        }
        let id = RequestId::new(self.next_id);
        self.next_id += 1;
        let deadline = now.saturating_add(budget);
        self.waiting.push_back(Waiting { id, deadline });
        self.stats.queued += 1;
        Submitted::Queued { id, deadline }
    }

    /// Retire every request whose deadline has passed, in submission order.
    ///
    /// Retirement stops at the first request that still has time, so a later request
    /// with a shorter budget waits behind it.
    pub fn expire(&mut self, now: Tick) -> Vec<Expired> {
        let mut expired = Vec::new();
        while let Some(waiting) = self.waiting.front() {
            if !now.reached(waiting.deadline) {
                break;
            }
            let waiting = self.waiting.pop_front().expect("the front exists");
            self.stats.expired += 1;
            expired.push(Expired {
                id: waiting.id,
                lateness: now.since(waiting.deadline),
            });
        }
        expired
    }

    /// Apply a decision.
    pub fn reply(&mut self, id: RequestId, decision: Decision) -> Reply {
        let Some(index) = self.waiting.iter().position(|waiting| waiting.id == id) else {
            if id.get() < self.next_id {
                self.stats.late += 1;
                return Reply::Late { id };
            }
            self.stats.unknown += 1;
            return Reply::Unknown { id };
        };
        self.waiting.remove(index);
        self.stats.applied += 1;
        Reply::Applied { id, decision }
    }

    /// Retire everything, for a config reload or a failed script worker.
    ///
    /// The caller resumes each event in the returned order, which is the order the
    /// events arrived in.
    pub fn retire_all(&mut self) -> Vec<RequestId> {
        let retired: Vec<RequestId> = self.waiting.drain(..).map(|waiting| waiting.id).collect();
        self.stats.retired += retired.len() as u64;
        retired
    }

    /// How many requests are waiting.
    pub fn pending(&self) -> usize {
        self.waiting.len()
    }

    /// Whether nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }

    /// The capacity.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// The counters so far.
    pub const fn stats(&self) -> PolicyStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, MAX_PENDING, PolicyQueue, Reply, Submitted};
    use gnoblin_types::Tick;

    #[test]
    fn a_decision_in_time_applies() {
        let mut queue = PolicyQueue::new(4);
        let Submitted::Queued { id, deadline } = queue.submit(Tick::new(10), 3) else {
            panic!("the queue was empty");
        };
        assert_eq!(deadline, Tick::new(13));
        assert_eq!(queue.pending(), 1);
        assert_eq!(
            queue.reply(id, Decision::Consume),
            Reply::Applied {
                id,
                decision: Decision::Consume
            }
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn a_late_decision_cannot_act() {
        let mut queue = PolicyQueue::new(4);
        let Submitted::Queued { id, .. } = queue.submit(Tick::new(10), 3) else {
            panic!("queued");
        };
        assert!(queue.expire(Tick::new(12)).is_empty(), "not yet due");
        let expired = queue.expire(Tick::new(15));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);
        assert_eq!(expired[0].lateness, 2);
        assert_eq!(queue.reply(id, Decision::Consume), Reply::Late { id });
        assert_eq!(queue.stats().expired, 1);
        assert_eq!(queue.stats().late, 1);
        assert_eq!(queue.stats().applied, 0);
    }

    #[test]
    fn retirement_follows_submission_order() {
        let mut queue = PolicyQueue::new(4);
        let first = queue.submit(Tick::new(1), 50);
        let second = queue.submit(Tick::new(2), 5);
        let (Submitted::Queued { id: first, .. }, Submitted::Queued { id: second, .. }) = (first, second) else {
            panic!("queued");
        };
        let expired = queue.expire(Tick::new(10));
        assert!(
            expired.is_empty(),
            "the first request still has time, so nothing may be retired"
        );
        assert_eq!(queue.pending(), 2);
        let expired = queue.expire(Tick::new(60));
        assert_eq!(
            expired.iter().map(|expired| expired.id).collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn a_full_queue_bypasses_instead_of_growing() {
        let mut queue = PolicyQueue::new(2);
        assert!(matches!(queue.submit(Tick::new(1), 5), Submitted::Queued { .. }));
        assert!(matches!(queue.submit(Tick::new(1), 5), Submitted::Queued { .. }));
        assert_eq!(queue.submit(Tick::new(1), 5), Submitted::Bypassed);
        assert_eq!(queue.pending(), 2);
        assert_eq!(queue.stats().bypassed, 1);
        assert_eq!(
            PolicyQueue::new(0).capacity(),
            1,
            "a queue that always bypassed would disable policy"
        );
        assert_eq!(PolicyQueue::new(MAX_PENDING).capacity(), MAX_PENDING);
    }

    #[test]
    fn a_reload_retires_everything_in_order() {
        let mut queue = PolicyQueue::new(4);
        let Submitted::Queued { id: first, .. } = queue.submit(Tick::new(1), 50) else {
            panic!("queued");
        };
        let Submitted::Queued { id: second, .. } = queue.submit(Tick::new(2), 50) else {
            panic!("queued");
        };
        assert_eq!(queue.retire_all(), vec![first, second]);
        assert!(queue.is_empty());
        assert_eq!(queue.reply(second, Decision::Forward), Reply::Late { id: second });
        assert_eq!(queue.stats().retired, 2);
    }

    #[test]
    fn a_reply_for_an_unissued_request_is_unknown() {
        let mut queue = PolicyQueue::new(2);
        let Submitted::Queued { id, .. } = queue.submit(Tick::new(1), 5) else {
            panic!("queued");
        };
        let forged = super::RequestId::new(id.get() + 9);
        assert_eq!(queue.reply(forged, Decision::Forward), Reply::Unknown { id: forged });
        assert_eq!(queue.pending(), 1, "the real request is untouched");
    }
}
