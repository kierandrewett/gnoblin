//! D-Bus agents: polkit, GeoClue, prompter, PipeWire privacy, ScreenSaver, inhibit and idle.
//!
//! This is the services role of `design/rust-compositor.md` ("Providers and slots")
//! as an **IPC client**. It depends on `gnoblin-client` and the request vocabulary in
//! `gnoblin-model`, never on an engine crate: the role runs in its own process
//! beside the compositor, exactly as the plan's dependency rules require.
//!
//! The machinery the agents share (the slot registry, the request state machine and the
//! privileged connection they offer through) lives in `gnoblin-service-core`. What lives
//! here is the role and the agents it serves:
//!
//! - [`role`], the role itself: it adopts the privileged connection the guardian passed,
//!   claims the slots the plan gives it a default for, serves the polkit agent and the gcr
//!   prompter when their buses and names are available, and declines an offer it cannot draw
//!   instead of letting it wait for a deadline.
//! - [`polkit`], the `org.freedesktop.PolicyKit1.AuthenticationAgent` implementation.
//!   Each `BeginAuthentication` becomes a `gnoblin_service_core::Proposition` offered through
//!   the IPC, and the provider's typed `gnoblin_service_core::Decision` is the reply polkitd
//!   gets. The state machine it drives is `gnoblin_service_core::Agent`, which holds no bus
//!   name and no bus handle, so its failure modes are tested without a bus.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it. The shared state machine's failure modes are
//! in `gnoblin-service-core`; this list is the one for the role's agents, in `tests/`.
//!
//! 1. **The polkit agent's reply carries the provider's identity as authority.**
//!    polkitd gets success or a refusal, and the identity stays a log line:
//!    `the_polkit_reply_carries_no_identity`.
//! 2. **An authentication hangs when the slot has no provider.** The D-Bus call
//!    returns at once with an error naming the slot:
//!    `a_begin_authentication_without_a_provider_names_the_slot`.
//! 3. **A cancelled authentication is still reported as success.** The waiting call
//!    returns `Cancelled`: `a_cancel_ends_the_waiting_authentication`.
//! 4. **Two agents register in one session.** The second registration is refused:
//!    `an_agent_registers_once`.

#![forbid(unsafe_code)]

pub mod link;
pub mod location;
pub mod polkit;
pub mod prompts;
pub mod role;
pub mod screensaver;
pub mod subject;

pub use polkit::Polkit;
pub use role::{Buses, Role, RoleError};
