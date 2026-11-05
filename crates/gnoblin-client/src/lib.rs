//! Rust client library for the IPC, used by gnoblinctl, the runtime, services, the
//! portal, tests and shells.
//!
//! This is the client half of `design/rust-compositor.md` ("One IPC" and "Object
//! model and IPC"): one vocabulary for shells, `gnoblinctl`, the portal, the
//! services role and the script runtime, spoken over [`gnoblin_proto`]'s envelopes.
//!
//! A client does four things, and this crate does nothing else:
//!
//! - **Connect.** The first exchange is the fixed-layout handshake, so a version
//!   disagreement is reported with both versions named before any envelope is
//!   decoded ([`Client::connect`]).
//! - **Subscribe.** [`Client::subscribe`] returns the snapshot at revision R; every
//!   later observation arrives through [`Client::next_event`] exactly once, in
//!   revision order.
//! - **Call and await.** A method call returns an [`Operation`] and the completion
//!   arrives as an observation, once ([`Client::await_operation`]).
//! - **Claim.** A [`Request`] is offered, exactly one owner claims it, and it
//!   finishes once with a completion that carries no answer value
//!   ([`Client::claim`], [`Client::answer`]).
//!
//! The client's public vocabulary is `gnoblin-model`'s: identities, values,
//! observations, snapshots and operations. Nothing here renames them, which is what
//! keeps the graph, the wire and the client from drifting into three dialects.
//!
//! The crate holds no socket. A connection arrives as a [`Transport`], which is
//! [`gnoblin_proto::Transport`] plus the handshake; [`loopback`] is the in-process
//! implementation the tests use, and the socket crate will be another. The wire
//! layout of the members this layer defines is documented in `src/wire.rs`.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/failure_modes.rs`.
//!
//! 1. **A late subscriber misses or repeats a diff.** The snapshot arrives before
//!    the first forwarded diff, and the graph stamps it, so a subscriber that
//!    starts from it is exactly current: `a_late_subscriber_gets_the_snapshot_and_no_replay`.
//! 2. **An operation completes twice.** The client sees one completion and the
//!    compositor refuses the second: `an_operation_is_delivered_exactly_once`.
//! 3. **A request is answered after it finished.** The second answer is refused
//!    with `request-finished`: `an_answer_after_the_finish_is_refused`.
//! 4. **Two claimants take one request.** The second claim is refused with
//!    `request-taken`, and the loser cannot answer:
//!    `two_clients_cannot_claim_one_request`.
//! 5. **Diffs arrive out of revision order.** Revisions strictly increase across a
//!    subscription: `diffs_arrive_in_revision_order`.
//! 6. **A subscription outlives its object.** Exactly one `closed` arrives, nothing
//!    follows it for that object, and its identity is never reused:
//!    `a_subscription_outliving_its_object_ends_at_closed`.
//! 7. **A version mismatch is discovered late.** The handshake refuses it and names
//!    both versions: `a_version_mismatch_is_refused_at_connect`.
//! 8. **A refused member is silently ignored.** An undeclared method or a dead
//!    object comes back as an error frame with the schema's code:
//!    `an_undeclared_method_is_refused_with_the_schema_code` and
//!    `a_dead_object_is_refused_with_its_own_code`.
//! 9. **An unanswered request hangs.** A tick fails it and the client learns the
//!    timeout: `an_unanswered_request_times_out_and_the_client_learns_it`.
//! 10. **A payload this build cannot read is half-applied.** A decode refuses the
//!     whole frame: the unit tests in `src/wire.rs`.
//! 11. **A prompt's answer travels unflagged, so a log keeps the password.** The
//!     request layer is schema-addressed, so `request.answer` is flagged from the
//!     schema's `secret` field and redaction drops the bytes:
//!     `a_request_answer_is_flagged_sensitive_and_redacts`, and
//!     `the_request_members_are_the_schema_members` keeps the names equal.

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod error;
#[cfg(unix)]
mod loopback;
#[cfg(unix)]
mod transport;
#[cfg(unix)]
pub mod unix;
#[cfg(unix)]
pub mod wire;

#[cfg(unix)]
pub use client::{Client, Event};
#[cfg(unix)]
pub use error::{ClientError, ConnectError, HandshakeError, WireError};
#[cfg(unix)]
pub use loopback::{Loopback, Server};
#[cfg(unix)]
pub use transport::Transport;
#[cfg(unix)]
pub use unix::{DEFAULT_SOCKET, RUNTIME_VARIABLE, SOCKET_VARIABLE, UnixTransport, socket_path};
#[cfg(unix)]
pub use wire::{NAMESPACE, code, member};
