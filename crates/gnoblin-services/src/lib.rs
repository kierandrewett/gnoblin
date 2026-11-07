//! D-Bus agents: polkit, prompter, PipeWire privacy, inhibit and idle. ScreenSaver is
//! `gnoblin-service-screensaver` and GeoClue is `gnoblin-service-location`.
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
//! - The polkit agent is `gnoblin-service-polkit`, which [`role`] serves: each
//!   `BeginAuthentication` becomes a `gnoblin_service_core::Proposition` offered through the
//!   IPC, and the provider's `gnoblin_service_core::Decision` is the reply polkitd gets.

#![forbid(unsafe_code)]

/// The Secret Service agent, served when `keyring.provider` names Gnoblin.
pub mod keyring;
/// Client types the compat host links its compat adapters with. See the module docs.
pub mod link;
/// Passphrase and keyring prompt handling, gated by `prompts.enabled`.
pub mod prompts;
/// The services role: adopts the privileged connection, claims slots and serves agents.
pub mod role;

/// The role's D-Bus connections ([`Buses`]), the role itself, and the error it reports when it cannot run.
pub use role::{Buses, Role, RoleError};

gnoblin_log::logger!(log = "services");

/// The Secret Service agent over Gnoblin's own store, at the paths the environment names. `None`
/// when `keyring.provider` names another program, which then keeps the name.
#[must_use]
pub fn keyring_agent() -> Option<Box<dyn link::LinkedAgent>> {
    if !keyring::enabled_from_environment() {
        return None;
    }
    let Some(store) = keyring::store_path_from_environment() else {
        log::warn!("no keyring store path: neither XDG_DATA_HOME nor HOME is absolute");
        return None;
    };
    Some(Box::new(keyring::KeyringAgent::new(
        store,
        keyring::socket_path_from_environment(),
    )))
}

/// The GeoClue agent, with the `location` table of the configuration as its policy. The
/// services role links it as a linked agent; the session binary reaches it only through here.
pub fn location_agent() -> Box<dyn link::LinkedAgent> {
    Box::new(gnoblin_service_location::LocationAgent::new(
        gnoblin_service_location::policy_from_environment(),
    ))
}
