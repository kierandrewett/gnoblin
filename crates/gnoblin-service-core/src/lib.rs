//! The services role's shared machinery: the slot registry, the request state machine and
//! the connection every agent offers through.
//!
//! This crate is an **IPC client** of the compositor (`design/rust-compositor.md`, "Providers
//! and slots"). It depends on `gnoblin-client` and the request vocabulary in `gnoblin-model`,
//! never on an engine crate, so it runs in the services process beside the compositor.
//!
//! - [`slot`], the slot registry. Every optional capability is a slot with one provider
//!   ([`CapabilitySlot`]). The state is published as the `session` object's `slot_*`
//!   properties so a shell, `gnoblinctl` and the recovery overlay read the same answer.
//! - [`agent`], the request state machine shared by every agent that offers a request.
//!   It holds no bus name and no bus handle, so its failure modes are tested without a bus.
//! - [`host`], the privileged connection the agents offer through, and the [`LinkedAgent`]
//!   seam the services role starts them behind.
//!
//! # Why the offer is a trait
//!
//! Offering a request to the connections that may claim it is a capability of the
//! compositor's connection, not of a plain client: a provider must be *told* what it is
//! being asked before it can claim. [`Offerer`] is that seam. The services role implements
//! it over the privileged [`Host`], this crate's tests implement it over the client's
//! in-process link, and the fail-closed rule lives on this side of it: a slot with no
//! provider is refused before anything is offered.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it.
//!
//! 1. **A request is offered to a slot nobody provides.** Refused, naming the slot, with no
//!    offer and no waiting: `no_provider_fails_closed_and_names_the_slot`.
//! 2. **Two connections claim one slot.** The second is refused and told who holds it:
//!    `a_second_claimant_is_refused`.
//! 3. **A provider goes away while its request is pending.** The request ends as cancelled
//!    instead of hanging until the deadline: `a_departed_provider_cancels_its_pending_requests`.
//! 4. **An answer arrives after the request ended.** Refused with `Finished`, never
//!    reopening it: `an_answer_after_a_cancel_is_refused`.
//! 5. **A publication names a property the schema does not declare.** The published fields
//!    are compared with the checked-in `session` schema:
//!    `the_published_slots_are_the_session_properties`.
//! 6. **The published slot state never reaches a client.** It is published into the graph of
//!    an in-process compositor and read back through a subscription:
//!    `a_client_learns_who_provides_each_slot`.

#![forbid(unsafe_code)]

/// The request state machine shared by every agent that offers a request.
pub mod agent;
/// The privileged connection agents offer through, and the linked-agent seam.
pub mod host;
/// The slot registry: each optional capability has one provider.
pub mod slot;

/// Agent types, re-exported from [`agent`]: an agent, its decision and its refusal.
pub use agent::{Agent, Decision, Finished, Identity, Offerer, Pending, PolkitRequest, Proposition, Refusal};
/// Request identity and outcome vocabulary, re-exported from `gnoblin-model`.
pub use gnoblin_model::{RequestId, RequestOutcome, Tick};
/// The host connection and its failure type, re-exported from [`host`].
pub use host::{Host, LinkedAgent, OfferFailure};
/// Slot types, re-exported from [`slot`].
pub use slot::{CapabilitySlot, ClaimError, Provider, Slots, UnknownSlot};
