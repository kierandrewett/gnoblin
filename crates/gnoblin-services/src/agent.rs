//! The `auth.polkit` state machine: one polkit request, one offer, one answer.
//!
//! A `BeginAuthentication` call from polkitd becomes a [`Proposition`] offered to
//! the compositor, a shell claims it and answers with the [`Decision`] the user
//! made, and the D-Bus call returns that decision. Nothing in between holds the
//! password: polkit's own authentication runs in polkit's helper, and this crate
//! never sees a secret for this slot.
//!
//! Three rules shape the state machine, and each is a failure mode from the plan:
//!
//! - **Fail closed with no provider.** [`Agent::begin`] consults the slot registry
//!   before it offers anything, and a slot with no provider is refused with an error
//!   that names the slot. The alternative is an offer nobody claims, which polkitd
//!   reports only after its own timeout as "Not authorized".
//! - **Finish once.** A request is in exactly one of pending or finished, and a
//!   finished request keeps a bounded tombstone, so an answer that arrives after a
//!   cancel is refused rather than reopening the request.
//! - **A provider that goes away ends its requests.** A claim that changes the
//!   provider of a slot, and a request past its deadline, both finish as
//!   [`Decision::Cancelled`], because the connection that was drawing the prompt is no
//!   longer the one that would answer it.
//!
//! The broker in `gnoblin-model` is deliberately not reused here. It answers "who
//! owns this request" for the compositor, which sees claims and answers as wire
//! frames; this agent answers a different question — which polkit invocation and
//! which cookie a request belongs to — and polkitd's cookie is not something the
//! broker has ever heard of. What the two share is the finish-once rule, which is a
//! `BTreeMap` and a tombstone rather than a second copy of the broker.
//!
//! # Failure modes first
//!
//! 1. **An answer arrives after the request was cancelled.** The tombstone turns it
//!    into [`Refusal::Finished`]: `an_answer_after_a_cancel_is_refused`.
//! 2. **A request is offered to a slot nobody provides.** No offer is made and the
//!    refusal names the slot: `no_provider_fails_closed_and_names_the_slot`.
//! 3. **The provider goes away while a request is pending.** The request finishes as
//!    cancelled, so the waiting D-Bus call returns instead of hanging:
//!    `a_departed_provider_cancels_its_pending_requests`.
//! 4. **A request outlives its deadline.** [`Agent::expire`] finishes it as
//!    cancelled: `a_request_past_its_deadline_is_cancelled`.
//! 5. **polkitd cancels by cookie.** The pending request ends and the waiting call
//!    returns cancelled: `a_cancel_by_cookie_ends_the_request`.
//! 6. **Two agents register in one session.** The second registration is refused
//!    instead of the two fighting over the session: `an_agent_registers_once`.
//! 7. **An answer is not the encoding the slot promised.** Non-UTF-8 bytes and an
//!    empty answer both fail closed as a refusal of the user's consent, never as a
//!    successful authentication: `an_unreadable_answer_is_a_refusal`.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use gnoblin_model::{RequestId, RequestOutcome, Tick, Value};
use gnoblin_types::OwnerId;

use crate::slot::{ClaimError, Provider, Slot, Slots};

/// How many finished requests the agent remembers.
///
/// A tombstone is what makes a late answer [`Refusal::Finished`] rather than
/// [`Refusal::UnknownRequest`], and the buffer is bounded so a session that asks for
/// thousands of authentications does not grow without limit. Forgetting a request
/// never reopens it: an answer to one that fell out of the buffer is refused as
/// unknown, which is still a refusal.
pub const REMEMBERED_FINISHED: usize = 64;

/// One identity polkit offered the agent, such as `unix-user` with the user's uid.
///
/// The details are the model's values, not a second vocabulary: a `unix-user`
/// identity's uid is a `uint` there, and a publication does not need a mapping table.
#[derive(Clone, PartialEq, Debug)]
pub struct Identity {
    /// The identity kind, which is the key of polkit's `(sa{sv})` identity.
    pub kind: String,
    /// The identity's details.
    pub details: BTreeMap<String, Value>,
}

/// The `BeginAuthentication` arguments, as polkitd sent them.
#[derive(Clone, PartialEq, Debug)]
pub struct PolkitRequest {
    /// polkitd's cookie, which is what `CancelAuthentication` names.
    pub cookie: String,
    /// The action being authorized, such as `org.freedesktop.policykit.exec`.
    pub action_id: String,
    /// The sentence the provider shows, in polkitd's words.
    pub message: String,
    /// The icon name polkitd suggested, if any.
    pub icon_name: String,
    /// Free-form details polkitd attached to the action.
    pub details: BTreeMap<String, String>,
    /// The identities the user may authenticate as.
    pub identities: Vec<Identity>,
}

/// What a provider is asked to decide.
///
/// The slot is part of it because a provider's claim and the request are joined by the
/// slot rather than by a connection: a shell can hold one slot while another holds a
/// different one.
#[derive(Clone, PartialEq, Debug)]
pub struct Proposition {
    /// The slot that owns this request.
    pub slot: Slot,
    /// The action, message, icon, details and identities of the request.
    pub request: PolkitRequest,
}

impl Proposition {
    /// The proposition for one polkit request.
    pub fn new(request: PolkitRequest) -> Proposition {
        Proposition {
            slot: Slot::AuthPolkit,
            request,
        }
    }
}

/// What the user decided.
///
/// The identity is what the user authenticated as, and it is only ever informational:
/// the plan's rule is that a provider presents a choice and returns a typed answer and
/// never holds authority. A component that acts on an authorization validates it
/// against the request it made, not against this name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decision {
    /// The user authenticated as this identity.
    Allowed {
        /// The identity the provider says the user authenticated as.
        identity: String,
    },
    /// The user declined, or the provider could not authenticate them.
    Denied,
    /// Nothing was decided: the request was cancelled, timed out or lost its provider.
    Cancelled,
}

/// One request that ended, and what the D-Bus layer must deliver.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finished {
    /// The request that ended.
    pub request: RequestId,
    /// How it ended.
    pub decision: Decision,
}

/// One request that is waiting for its answer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pending {
    /// The request's identity on the IPC.
    pub request: RequestId,
    /// The slot that owns the request.
    pub slot: Slot,
    /// polkitd's cookie, which `CancelAuthentication` names.
    pub cookie: String,
    /// The action being authorized, for a log line that never carries an answer.
    pub action_id: String,
    /// When the request fails if nobody answers it.
    pub deadline: Tick,
}

/// The request the agent just offered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Offered {
    /// The identity the answer and the cancellation name.
    pub request: RequestId,
}

/// The privileged connection through which a slot request reaches the IPC.
///
/// The services role is a client (design/rust-compositor.md, "Providers and slots"),
/// so offering a request to the connections that can claim it is the composition
/// root's job: it holds the connection that is allowed to offer, and this crate holds
/// the state machine that decides what is offered and what an answer means. The trait
/// is that seam, and it is also the fail-closed seam: a connection that knows
/// `auth.polkit` has no provider refuses here, before anything is offered.
///
/// It is not `Send` on purpose: the tests drive the agent with an in-process host over
/// the client's loopback, which is single-threaded. [`crate::polkit`] is where the
/// bound is added, because a served interface's future is spawned.
pub trait Offerer {
    /// Offer one request, announcing it to the connections allowed to claim it.
    fn offer(&mut self, proposition: &Proposition, deadline: Tick) -> Result<RequestId, Refusal>;
}

/// A boxed offerer is an offerer, so the D-Bus layer can hold whichever connection the
/// composition root gave it without naming its type.
impl Offerer for Box<dyn Offerer + Send> {
    fn offer(&mut self, proposition: &Proposition, deadline: Tick) -> Result<RequestId, Refusal> {
        (**self).offer(proposition, deadline)
    }
}

/// Why a request could not be started, answered or finished.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// No provider holds the slot, so nothing could show the prompt. The message names
    /// the slot, because that is the one thing a caller or a log needs to fix it.
    NoProvider {
        /// The slot with no provider.
        slot: Slot,
        /// What the compositor said, which is the detail a log keeps.
        reason: String,
    },
    /// A claim or a release was refused by the registry.
    Claim(ClaimError),
    /// The request is not one this agent offered, and it is not in the tombstone
    /// buffer either.
    UnknownRequest {
        /// The request the caller named.
        request: RequestId,
    },
    /// The request already ended. An answer here never reopens it.
    Finished {
        /// The request that is already finished.
        request: RequestId,
    },
    /// An authentication agent is already registered in this session.
    AlreadyRegistered,
    /// The D-Bus registration was refused by polkitd.
    Registration {
        /// What polkitd or the bus said.
        reason: String,
    },
    /// The offer could not be delivered: the privileged connection refused it or ended.
    ///
    /// This is not a policy answer, so it is separate from [`Refusal::NoProvider`]: the
    /// slot has a provider, and the compositor never heard about the request.
    Connection {
        /// What the connection said.
        reason: String,
    },
}

impl Refusal {
    /// The slot this refusal is about, when it is about one.
    pub fn slot(&self) -> Option<Slot> {
        match self {
            Refusal::NoProvider { slot, .. } => Some(*slot),
            Refusal::Claim(ClaimError::Held { slot, .. })
            | Refusal::Claim(ClaimError::NotHolder { slot })
            | Refusal::Claim(ClaimError::TakenOver { slot }) => Some(*slot),
            _ => None,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoProvider { slot, reason } => write!(
                formatter,
                "no provider claims the `{slot}` slot, so the request was not offered: {reason}"
            ),
            Refusal::Claim(error) => write!(formatter, "{error}"),
            Refusal::UnknownRequest { request } => {
                write!(formatter, "request {} was never offered by this agent", request.get())
            }
            Refusal::Finished { request } => write!(formatter, "request {} already ended", request.get()),
            Refusal::AlreadyRegistered => formatter.write_str("an authentication agent is already registered"),
            Refusal::Registration { reason } => write!(formatter, "polkitd refused the registration: {reason}"),
            Refusal::Connection { reason } => {
                write!(formatter, "the privileged connection refused the offer: {reason}")
            }
        }
    }
}

impl std::error::Error for Refusal {}

impl From<ClaimError> for Refusal {
    fn from(error: ClaimError) -> Refusal {
        Refusal::Claim(error)
    }
}

/// The live half of one request, kept out of [`Pending`] so a transition can read it and
/// then replace it without holding a borrow.
#[derive(Clone, Copy, Debug)]
struct Deadline {
    deadline: Tick,
}

/// The polkit agent's state: the slots, the requests waiting for an answer and the
/// tombstones of the ones that ended.
#[derive(Debug)]
pub struct Agent<O: Offerer> {
    slots: Slots,
    offerer: O,
    pending: BTreeMap<RequestId, (Pending, Deadline)>,
    finished: BTreeMap<RequestId, Decision>,
    forgotten: VecDeque<RequestId>,
    registered: bool,
}

impl<O: Offerer> Agent<O> {
    /// An agent with the plan's default providers and nothing registered.
    pub fn new(offerer: O) -> Agent<O> {
        Agent {
            slots: Slots::new(),
            offerer,
            pending: BTreeMap::new(),
            finished: BTreeMap::new(),
            forgotten: VecDeque::new(),
            registered: false,
        }
    }

    /// The registry, for a publication or a report.
    pub fn slots(&self) -> &Slots {
        &self.slots
    }

    /// The connection-side offerer, for a caller that also drives that side.
    pub fn offerer(&self) -> &O {
        &self.offerer
    }

    /// The connection-side offerer, mutable, for a caller that also drives that side.
    pub fn offerer_mut(&mut self) -> &mut O {
        &mut self.offerer
    }

    /// Whether an authentication agent is registered in this session.
    pub fn is_registered(&self) -> bool {
        self.registered
    }

    /// Record that this agent registered, or refuse a second registration.
    ///
    /// polkit allows one agent per session, so two registrations from one process is a
    /// bug in the caller rather than a state to tolerate.
    pub fn register(&mut self) -> Result<(), Refusal> {
        if self.registered {
            return Err(Refusal::AlreadyRegistered);
        }
        self.registered = true;
        Ok(())
    }

    /// Record that the agent unregistered, cancelling everything it was waiting on.
    pub fn unregister(&mut self) -> Vec<Finished> {
        self.registered = false;
        self.cancel_slot(Slot::AuthPolkit)
    }

    /// Claim a slot for a provider, cancelling the requests the previous provider was
    /// holding.
    ///
    /// A claim is how a shell takes `auth.polkit` over, and a provider change is why the
    /// pending requests end: the connection that was drawing the prompt is not the one
    /// that will answer it.
    pub fn claim(&mut self, slot: Slot, provider: Provider, owner: Option<OwnerId>) -> Result<Vec<Finished>, Refusal> {
        let changed = self.slots.claim(slot, provider, owner)?;
        Ok(if changed { self.cancel_slot(slot) } else { Vec::new() })
    }

    /// Release a shell's claim, cancelling what it was holding.
    pub fn release(&mut self, slot: Slot, owner: OwnerId) -> Result<Vec<Finished>, Refusal> {
        let changed = self.slots.release(slot, owner)?;
        Ok(if changed { self.cancel_slot(slot) } else { Vec::new() })
    }

    /// Offer one polkit request to the slot's provider.
    pub fn begin(&mut self, request: PolkitRequest, deadline: Tick) -> Result<Offered, Refusal> {
        let slot = Slot::AuthPolkit;
        if !self.slots.provider(slot).is_present() {
            return Err(Refusal::NoProvider {
                slot,
                reason: "this agent refuses before offering when the slot has no provider".to_owned(),
            });
        }
        let proposition = Proposition::new(request.clone());
        let offered = self.offerer.offer(&proposition, deadline)?;
        let pending = Pending {
            request: offered,
            slot,
            cookie: request.cookie,
            action_id: request.action_id,
            deadline,
        };
        self.pending.insert(offered, (pending, Deadline { deadline }));
        Ok(Offered { request: offered })
    }

    /// Report how a request this agent offered ended.
    ///
    /// The compositor owns the broker, so the outcome arrives from the connection rather
    /// than from this agent, and it is the model's own vocabulary: a provider that
    /// answered, a request that ended without one, or a deadline that passed.
    pub fn answer(&mut self, request: RequestId, outcome: &RequestOutcome) -> Result<Decision, Refusal> {
        if !self.pending.contains_key(&request) {
            return Err(self.absent(request));
        }
        let decision = Decision::from_outcome(outcome);
        self.commit(request, decision.clone());
        Ok(decision)
    }

    /// Cancel a request this agent offered, as polkitd does when the caller exits.
    pub fn cancel(&mut self, request: RequestId) -> Result<Decision, Refusal> {
        if !self.pending.contains_key(&request) {
            return Err(self.absent(request));
        }
        let decision = Decision::Cancelled;
        self.commit(request, decision.clone());
        Ok(decision)
    }

    /// Cancel whatever `CancelAuthentication` named by cookie.
    ///
    /// polkitd names the cookie, not the request, and a cookie that matches nothing is
    /// not an error: polkitd may cancel a request that already finished.
    pub fn cancel_cookie(&mut self, cookie: &str) -> Option<Finished> {
        let request = self
            .pending
            .values()
            .find(|(pending, _)| pending.cookie == cookie)
            .map(|(pending, _)| pending.request)?;
        self.commit(request, Decision::Cancelled);
        Some(Finished {
            request,
            decision: Decision::Cancelled,
        })
    }

    /// Cancel every pending request of one slot, returning what ended.
    pub fn cancel_slot(&mut self, slot: Slot) -> Vec<Finished> {
        let requests: Vec<RequestId> = self
            .pending
            .values()
            .filter(|(pending, _)| pending.slot == slot)
            .map(|(pending, _)| pending.request)
            .collect();
        self.finish_all(requests)
    }

    /// Cancel every request at or past its deadline, returning what ended.
    ///
    /// The caller supplies the current tick, so nothing here reads a clock and a test
    /// replays the same outcome from the same ticks.
    pub fn expire(&mut self, now: Tick) -> Vec<Finished> {
        let requests: Vec<RequestId> = self
            .pending
            .iter()
            .filter(|(_, (_, live))| live.deadline <= now)
            .map(|(request, _)| *request)
            .collect();
        self.finish_all(requests)
    }

    /// Every request that is waiting for an answer.
    pub fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.pending.values().map(|(pending, _)| pending)
    }

    /// The state of one finished request, for a report or a test.
    pub fn decision(&self, request: RequestId) -> Option<&Decision> {
        self.finished.get(&request)
    }

    /// Why a request is not pending: finished, or never offered.
    fn absent(&self, request: RequestId) -> Refusal {
        if self.finished.contains_key(&request) {
            Refusal::Finished { request }
        } else {
            Refusal::UnknownRequest { request }
        }
    }

    /// Finish one request and record its tombstone.
    fn commit(&mut self, request: RequestId, decision: Decision) {
        if self.pending.remove(&request).is_none() {
            return;
        }
        self.finished.insert(request, decision);
        self.forgotten.push_back(request);
        while self.forgotten.len() > REMEMBERED_FINISHED {
            if let Some(oldest) = self.forgotten.pop_front() {
                self.finished.remove(&oldest);
            }
        }
    }

    /// Finish every named request as cancelled.
    fn finish_all(&mut self, requests: Vec<RequestId>) -> Vec<Finished> {
        let live: Vec<RequestId> = requests
            .into_iter()
            .filter(|request| self.pending.contains_key(request))
            .collect();
        for request in &live {
            self.commit(*request, Decision::Cancelled);
        }
        live.into_iter()
            .map(|request| Finished {
                request,
                decision: Decision::Cancelled,
            })
            .collect()
    }
}

impl Decision {
    /// Read the typed decision out of a finished request.
    ///
    /// The `auth.polkit` slot's encoding is: an empty answer is [`Decision::Denied`],
    /// any other answer names the identity the user authenticated as, and a request that
    /// ended without one or timed out is [`Decision::Cancelled`]. Bytes that are not
    /// UTF-8 are refused as `Denied`, because a provider that sent something unreadable
    /// has not said the user consented.
    pub fn from_outcome(outcome: &RequestOutcome) -> Decision {
        match outcome {
            RequestOutcome::Ended | RequestOutcome::TimedOut => Decision::Cancelled,
            RequestOutcome::Answered(bytes) if bytes.is_empty() => Decision::Denied,
            RequestOutcome::Answered(bytes) => match std::str::from_utf8(bytes) {
                Ok(identity) => Decision::Allowed {
                    identity: identity.to_owned(),
                },
                Err(_) => Decision::Denied,
            },
        }
    }
}
