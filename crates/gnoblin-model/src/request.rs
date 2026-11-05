//! The request broker: offered, claimed by one owner, finished once.
//!
//! A `Request` is how the compositor asks a provider to decide something: polkit
//! authentication, a secret prompt, a portal consent dialog, a location request
//! (design/rust-compositor.md, "Main data paths" and "Providers and slots"). The
//! broker is the only place that decides who owns one and when it ends.
//!
//! The state machine is small on purpose:
//!
//! ```text
//! offer → Offered → claim(owner) → Claimed → answer | tick(deadline) → Finished
//! ```
//!
//! Three properties matter more than the transitions:
//!
//! - **One owner.** The first claim wins. A second owner is refused with the
//!   holder's identity, and an answer from an owner that does not hold the request
//!   is refused, so a client cannot answer on another's behalf.
//! - **One finished outcome.** Finishing removes the live entry and records a
//!   tombstone, so a late answer is refused rather than reopening the request, and
//!   [`RequestBroker::events`] yields exactly one `Finished` per request.
//! - **Fail closed on timeout.** A request is offered with a deadline and the
//!   caller supplies the current [`Tick`]; [`RequestBroker::tick`] finishes every
//!   request at or past its deadline as [`RequestOutcome::TimedOut`]. Nothing here
//!   reads a clock, so a compositor, a test and a replay all reach the same
//!   outcome from the same ticks.

use std::collections::{BTreeMap, VecDeque};

use gnoblin_types::{OwnerId, RequestId, Tick};

use crate::error::RequestError;

/// How many finished requests the broker remembers.
///
/// A tombstone is what turns a late answer into [`RequestError::Finished`] instead
/// of [`RequestError::Unknown`], and the buffer is bounded so a session that
/// offers thousands of prompts does not grow without limit. An answer to a request
/// that has fallen out of the buffer is refused as unknown, which is still a
/// refusal: forgetting never reopens a request.
pub const REMEMBERED_FINISHED: usize = 256;

/// What the owning provider answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The provider chose something. The bytes are the provider's encoded answer
    /// and never leave the host that offered the request.
    Answered(Vec<u8>),
    /// The provider declined or went away without choosing.
    Ended,
}

impl From<Answer> for RequestOutcome {
    fn from(answer: Answer) -> RequestOutcome {
        match answer {
            Answer::Answered(bytes) => RequestOutcome::Answered(bytes),
            Answer::Ended => RequestOutcome::Ended,
        }
    }
}

/// How a request ended. Exactly one of these is produced per request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestOutcome {
    /// A provider answered, and here is what it said. Only the host that offered
    /// the request sees the value: a prompt answer never reaches a subscriber.
    Answered(Vec<u8>),
    /// The request was ended without an answer.
    Ended,
    /// The deadline passed before an answer arrived.
    TimedOut,
}

/// The finished outcome as a peer may learn it: which kind, never the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestCompletion {
    /// A provider answered. The value is not part of this.
    Answered,
    /// The request was ended without an answer.
    Ended,
    /// The deadline passed.
    TimedOut,
}

impl From<&RequestOutcome> for RequestCompletion {
    fn from(outcome: &RequestOutcome) -> RequestCompletion {
        match outcome {
            RequestOutcome::Answered(_) => RequestCompletion::Answered,
            RequestOutcome::Ended => RequestCompletion::Ended,
            RequestOutcome::TimedOut => RequestCompletion::TimedOut,
        }
    }
}

/// Something that happened to a request, in the order it happened.
///
/// The broker hands these to its caller rather than to the object graph: a
/// request's answer is not object state, and a prompt answer must never reach
/// another client's subscription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestEvent {
    /// A new request is available to claim.
    Offered {
        /// The new request.
        request: RequestId,
        /// When [`RequestBroker::tick`] will fail it if nobody claims and answers.
        deadline: Tick,
    },
    /// The request ended. This is emitted exactly once per request.
    Finished {
        /// The request that ended.
        request: RequestId,
        /// How it ended, including the answer only the offering host reads.
        outcome: RequestOutcome,
    },
}

/// The live state of one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestState {
    /// Offered, with nobody owning it yet.
    Offered {
        /// When the request fails if it is never claimed.
        deadline: Tick,
    },
    /// Owned by one connection.
    Claimed {
        /// The owner, which is the only party that may answer.
        owner: OwnerId,
        /// When the request fails if it is never answered.
        deadline: Tick,
    },
    /// Finished. It stays finished; a late answer is refused.
    Finished(RequestOutcome),
}

/// The live entry for one request, kept separate from the finished tombstone so a
/// transition can read it and then replace it without holding a borrow.
#[derive(Clone, Copy, Debug)]
enum Live {
    Offered { deadline: Tick },
    Claimed { owner: OwnerId, deadline: Tick },
}

impl Live {
    fn deadline(self) -> Tick {
        match self {
            Live::Offered { deadline } | Live::Claimed { deadline, .. } => deadline,
        }
    }
}

/// The request broker.
#[derive(Debug)]
pub struct RequestBroker {
    next: u64,
    live: BTreeMap<RequestId, Live>,
    finished: BTreeMap<RequestId, RequestOutcome>,
    forgotten: VecDeque<RequestId>,
    events: VecDeque<RequestEvent>,
}

impl Default for RequestBroker {
    fn default() -> RequestBroker {
        RequestBroker::new()
    }
}

impl RequestBroker {
    /// An empty broker.
    pub fn new() -> RequestBroker {
        RequestBroker {
            next: 1,
            live: BTreeMap::new(),
            finished: BTreeMap::new(),
            forgotten: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    /// Offer a request that fails at `deadline`.
    pub fn offer(&mut self, deadline: Tick) -> RequestId {
        let request = RequestId::new(self.next);
        self.next += 1;
        self.live.insert(request, Live::Offered { deadline });
        self.events.push_back(RequestEvent::Offered { request, deadline });
        request
    }

    /// Claim a request for one owner.
    ///
    /// The first claim wins. A repeated claim by the same owner succeeds, because a
    /// client that retries a claim it already won should not be told it lost; a
    /// claim by anyone else is refused with the holder's identity.
    pub fn claim(&mut self, request: RequestId, owner: OwnerId) -> Result<(), RequestError> {
        match self.live_state(request)? {
            Live::Claimed { owner: holder, .. } if holder != owner => {
                Err(RequestError::Taken { request, owner: holder })
            }
            Live::Claimed { .. } => Ok(()),
            Live::Offered { deadline } => {
                self.live.insert(request, Live::Claimed { owner, deadline });
                Ok(())
            }
        }
    }

    /// Answer a request the owner holds.
    ///
    /// An answer from anyone else, or to a request that already finished, is
    /// refused; a refusal never changes the request's state.
    pub fn answer(&mut self, request: RequestId, owner: OwnerId, answer: Answer) -> Result<(), RequestError> {
        match self.live_state(request)? {
            Live::Offered { .. } => Err(RequestError::NotOwner { request, owner }),
            Live::Claimed { owner: holder, .. } if holder != owner => Err(RequestError::NotOwner { request, owner }),
            Live::Claimed { .. } => {
                self.finish(request, answer.into());
                Ok(())
            }
        }
    }

    /// Fail every request at or past its deadline, and report how many ended.
    ///
    /// This is the fail-closed half of the contract: a request that nobody answers
    /// ends as [`RequestOutcome::TimedOut`] rather than staying offered forever.
    pub fn tick(&mut self, now: Tick) -> usize {
        let overdue: Vec<RequestId> = self
            .live
            .iter()
            .filter(|(_, state)| state.deadline() <= now)
            .map(|(request, _)| *request)
            .collect();
        for request in &overdue {
            self.finish(*request, RequestOutcome::TimedOut);
        }
        overdue.len()
    }

    /// Finish every request an owner still holds, and report how many ended.
    ///
    /// A provider connection that goes away leaves nothing waiting: its requests
    /// end as [`RequestOutcome::Ended`], which the offering host reports as the
    /// provider having gone.
    pub fn release(&mut self, owner: OwnerId) -> usize {
        let held: Vec<RequestId> = self
            .live
            .iter()
            .filter(|(_, state)| matches!(state, Live::Claimed { owner: holder, .. } if *holder == owner))
            .map(|(request, _)| *request)
            .collect();
        for request in &held {
            self.finish(*request, RequestOutcome::Ended);
        }
        held.len()
    }

    /// The state of one request: live, finished or gone from the broker's memory.
    pub fn state(&self, request: RequestId) -> Option<RequestState> {
        if let Some(live) = self.live.get(&request) {
            return Some(match *live {
                Live::Offered { deadline } => RequestState::Offered { deadline },
                Live::Claimed { owner, deadline } => RequestState::Claimed { owner, deadline },
            });
        }
        self.finished.get(&request).cloned().map(RequestState::Finished)
    }

    /// Take every event since the last call, in order.
    pub fn events(&mut self) -> Vec<RequestEvent> {
        self.events.drain(..).collect()
    }

    /// The live entry, or the refusal that says why there is none.
    fn live_state(&self, request: RequestId) -> Result<Live, RequestError> {
        match self.live.get(&request) {
            Some(live) => Ok(*live),
            None if self.finished.contains_key(&request) => Err(RequestError::Finished { request }),
            None => Err(RequestError::Unknown { request }),
        }
    }

    /// Move a live request to finished and emit its one `Finished` event.
    ///
    /// A request that is already finished is left alone, so this can never emit a
    /// second outcome for the same request.
    fn finish(&mut self, request: RequestId, outcome: RequestOutcome) {
        if self.live.remove(&request).is_none() {
            return;
        }
        self.events.push_back(RequestEvent::Finished {
            request,
            outcome: outcome.clone(),
        });
        self.finished.insert(request, outcome);
        self.forgotten.push_back(request);
        while self.forgotten.len() > REMEMBERED_FINISHED {
            if let Some(oldest) = self.forgotten.pop_front() {
                self.finished.remove(&oldest);
            }
        }
    }
}
