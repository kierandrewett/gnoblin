//! Live object graph, revisions, property diffs and subscriptions.
//!
//! This crate is the object model of `design/rust-compositor.md` ("Core
//! abstractions", "One IPC" and "Object model and IPC"): a live graph of objects
//! with identities and lifetimes, typed property values, monotonic revisions, and
//! one delivery rule.
//!
//! The rule everything else exists to keep:
//!
//! **Property diffs and signals are the only way state leaves the graph.** A
//! subscriber never receives a whole-graph snapshot after it subscribes. It gets
//! a [`Snapshot`] once, at the graph's revision R, and then every [`Observation`]
//! published after R exactly once and in revision order. Nothing is coalesced,
//! nothing is reordered and no second snapshot is ever handed out, so a
//! subscriber that starts from the snapshot and applies what it is given always
//! holds exactly the graph's state. `tests/model.rs` checks that equality over
//! random sequences.
//!
//! The shared primitives are `gnoblin-types`' and are re-exported here: an
//! identity, a revision, a tick and an error code are the same values the engine,
//! the client and the engine crates name, so this crate declares none of its own.
//!
//! Nothing here does I/O, reads a clock, spawns a task or allocates a channel.
//! Every call takes its input as an argument, including a request's deadline
//! ([`Tick`]) and the current tick that expires it, so a test replays any
//! sequence from a seed and gets the same observations.
//!
//! Three pieces make up the crate:
//!
//! - [`Graph`], the objects, their properties, the declarative signal calls and
//!   the pending [`Operation`]s.
//! - [`Subscription`]s, the snapshot-then-diffs rule.
//! - [`RequestBroker`], the offered → claimed → finished state machine whose
//!   timeout is fail closed, and the [`CapabilitySlot`] registry that says which provider a
//!   request is offered to.
//!
//! The vocabulary is the schema's, not this crate's: object types, properties,
//! methods, signals and error codes come from [`gnoblin_schema::Catalog`], and a
//! graph refuses a property, argument or value the schema does not declare. A
//! name that is not in the schema is a bug in the caller, not something to
//! tolerate.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it.
//!
//! 1. **A late subscriber misses a diff, or receives one twice.** A subscription
//!    is created with a snapshot stamped at the graph revision and a queue that
//!    only ever receives observations published after it, so nothing earlier can
//!    be replayed and nothing later can be skipped:
//!    `failure_modes.rs::a_late_subscriber_misses_nothing_and_repeats_nothing`.
//! 2. **An operation completes twice.** Completion removes the pending entry, so
//!    a second completion is refused and never produces a second observation:
//!    `failure_modes.rs::an_operation_completes_once`.
//! 3. **A request is answered after it finished.** A finished request keeps a
//!    tombstone, so a late answer is refused instead of reopening it:
//!    `failure_modes.rs::an_answer_after_the_finish_is_refused`.
//! 4. **Two owners claim one request.** The first claim wins; a second owner is
//!    refused and can never answer on the claimant's behalf:
//!    `failure_modes.rs::two_claimants_only_the_first_wins`.
//! 5. **Diffs are delivered out of revision order.** Every publish bumps one
//!    counter and pushes into each matching queue in that same step, so a
//!    subscription's stream is strictly increasing:
//!    `failure_modes.rs::diffs_arrive_in_revision_order`.
//! 6. **A subscriber outlives its object.** A closed object's identity is never
//!    reused, its [`Observation::Closed`] arrives exactly once, and everything
//!    after it addresses no object:
//!    `failure_modes.rs::a_subscriber_outlives_its_object`.
//! 7. **A value of the wrong type enters the graph.** Every write is checked
//!    against the schema before anything is published, and a refused write
//!    leaves the revision untouched:
//!    `failure_modes.rs::a_wrongly_typed_value_never_enters_the_graph`.
//! 8. **An undeclared member is accepted.** Creating, reading or signalling a
//!    property, method or signal the object's schema does not declare fails:
//!    `failure_modes.rs::an_undeclared_member_is_refused`.
//! 9. **A request never times out.** [`RequestBroker::tick`] finishes every
//!    overdue request as timed out, so a request cannot outlive its deadline:
//!    `failure_modes.rs::a_request_times_out_fail_closed`.
//! 10. **A departed owner leaves a request hanging.** Releasing an owner finishes
//!     its claimed requests: `failure_modes.rs::a_departed_owner_finishes_its_requests`.

mod error;
mod graph;
mod object;
mod operation;
mod power;
mod request;
mod slot;
mod types;
mod value;

pub use error::{ModelError, RequestError};
pub use graph::{Filter, Graph, OBJECT_GONE, ObjectView, Observation, Snapshot, Subscription};
pub use object::Object;
pub use operation::{Operation, OperationOutcome};
pub use power::{PowerBackend, PowerError, PowerErrorKind, PowerVerb};
pub use request::{DeclineReason, RequestBroker, RequestCompletion, RequestEvent, RequestOutcome, RequestState};
pub use slot::{CapabilitySlot, ClaimError, Provider, Slots, UnknownSlot};
pub use types::TypeKey;
pub use value::{Fields, Value};

// The shared primitives, re-exported so a caller of the graph names one crate.
pub use gnoblin_types::{ErrorCode, ObjectId, OperationId, OwnerId, RequestId, Revision, SubscriptionId, Tick};
