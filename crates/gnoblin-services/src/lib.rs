//! D-Bus agents: polkit, GeoClue, prompter, PipeWire privacy, ScreenSaver, inhibit and idle.
//!
//! This is the services role of `design/rust-compositor.md` ("Providers and slots")
//! as an **IPC client**. It depends on `gnoblin-client` and the request vocabulary in
//! `gnoblin-model`, never on an engine crate: the role runs in its own process
//! beside the compositor, exactly as the plan's dependency rules require.
//!
//! Three things live here so far, and the rest of the role arrives with its own slots:
//!
//! - [`role`], the role itself: it adopts the privileged connection the guardian passed,
//!   claims the slots the plan gives it a default for, serves the polkit agent and the gcr
//!   prompter when their buses and names are available, and declines an offer it cannot draw
//!   instead of letting it wait for a deadline.
//! - [`slot`], the slot registry. Every optional capability is a slot with one
//!   provider ([`Slot`]), the plan's default is the services role's for three of them
//!   and nobody's for `prompt.secret`, and the state is published as the `session`
//!   object's `slot_*` properties so a shell, `gnoblinctl` and the recovery overlay
//!   read the same answer.
//! - [`polkit`], the `org.freedesktop.PolicyKit1.AuthenticationAgent` implementation.
//!   Each `BeginAuthentication` becomes a [`Proposition`] offered through the IPC,
//!   and the provider's typed [`Decision`] is the reply polkitd gets. The state
//!   machine is [`agent`], which holds no bus name and no bus handle, so its failure
//!   modes are tested without a bus.
//!
//! # Why the offer is a trait
//!
//! Offering a request to the connections that may claim it is a capability of the
//! compositor's connection, not of a plain client: a provider must be *told* what it
//! is being asked before it can claim, and the schema's `request` object carries
//! only the identity and the deadline. [`Offerer`] is that seam. The composition root
//! implements it over the privileged connection, this crate's tests implement it over
//! the client's in-process link, and the fail-closed rule lives on this side of it:
//! a slot with no provider is refused before anything is offered.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it. The agents' own lists are in their
//! module docs; this list is the crate-level one, in `tests/`.
//!
//! 1. **A request is offered to a slot nobody provides.** Refused, naming the slot,
//!    with no offer and no waiting: `no_provider_fails_closed_and_names_the_slot`.
//! 2. **Two connections claim one slot.** The second is refused and told who holds
//!    it: `a_second_claimant_is_refused`.
//! 3. **A provider goes away while its request is pending.** The request ends as
//!    cancelled instead of hanging until the deadline:
//!    `a_departed_provider_cancels_its_pending_requests`.
//! 4. **An answer arrives after the request ended.** Refused with `Finished`, never
//!    reopening it: `an_answer_after_a_cancel_is_refused`.
//! 5. **A publication names a property the schema does not declare.** The published
//!    fields are compared with the checked-in `session` schema:
//!    `the_published_slots_are_the_session_properties`.
//! 6. **The published slot state never reaches a client.** It is published into the
//!    graph of an in-process compositor and read back through a subscription:
//!    `a_client_learns_who_provides_each_slot`.
//! 7. **The polkit agent's reply carries the provider's identity as authority.**
//!    polkitd gets success or a refusal, and the identity stays a log line:
//!    `the_polkit_reply_carries_no_identity`.
//! 8. **An authentication hangs when the slot has no provider.** The D-Bus call
//!    returns at once with an error naming the slot:
//!    `a_begin_authentication_without_a_provider_names_the_slot`.
//! 9. **A cancelled authentication is still reported as success.** The waiting call
//!    returns `Cancelled`: `a_cancel_ends_the_waiting_authentication`.
//! 10. **Two agents register in one session.** The second registration is refused:
//!     `an_agent_registers_once`.

#![forbid(unsafe_code)]

pub mod agent;
pub mod location;
pub mod polkit;
pub mod role;
pub mod slot;
pub mod subject;

pub use agent::{Agent, Decision, Finished, Identity, Offerer, Pending, PolkitRequest, Proposition, Refusal};
pub use gnoblin_model::{RequestId, RequestOutcome, Tick};
pub use polkit::Polkit;
pub use role::{Buses, Host, LinkedAgent, OfferFailure, Role, RoleError};
pub use slot::{ClaimError, Provider, Slot, Slots, UnknownSlot};
