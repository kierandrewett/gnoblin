//! XML protocol `security_context_v1` (`<protocol name="security_context_v1">`); interfaces served: `wp_security_context_v1`.
//!
//! `wp_security_context_v1`: application sandboxes.
//!
//! A sandboxed application does not get a separate socket with a policy in it: it asks
//! the compositor it is already connected to for a *listener*, and connects its own
//! child processes there. Each connection that arrives on that listener is a client of
//! its own — its own registry, its own objects — and it is marked
//! [`gnoblin_wayland_api::access::ClientRole::Sandboxed`] before its first request is dispatched, which is
//! what makes "a sandboxed client sees no privileged global" true rather than hoped
//! for.
//!
//! ## Why the listener is queued
//!
//! The protocol hands the compositor a listening socket during a dispatch, and only
//! the event loop can watch one. The source is therefore queued on the state and taken
//! by the loop on the same turn, which keeps the accept path out of the protocol
//! handler without a second event loop.
//!
//! ## Failure modes first
//!
//! 1. **A sandbox is created and its connections are never accepted.** The listener is
//!    drained every turn, so a client that connects to its own sandbox socket is served
//!    like any other: `a_sandboxed_client_sees_no_privileged_global` in `tests/access.rs`.
//! 2. **A sandboxed connection inherits the role of its creator.** Roles are recorded per
//!    connection, never per process, so a privileged shell that creates a sandbox does
//!    not make the sandboxed application privileged: the same test, from the privileged
//!    socket's creator.

use gnoblin_wayland_api::access::ClientRole;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use smithay::reexports::wayland_protocols::wp::security_context::v1::server::{
    wp_security_context_manager_v1::WpSecurityContextManagerV1, wp_security_context_v1::WpSecurityContextV1,
};
use smithay::reexports::wayland_server::{Client, Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::security_context::{
    SecurityContext, SecurityContextGlobalData, SecurityContextListenerSource, SecurityContextState,
    SecurityContextUserData,
};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wp-security-context-v1",
    interfaces: &[ServedInterface::new("wp_security_context_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor: where the listening sockets wait for the event loop.
pub trait Host {
    /// The listeners the protocol created, not yet taken by the event loop.
    fn pending_contexts(&mut self) -> &mut Vec<SecurityContextListenerSource>;
}

/// Creates the `wp_security_context_manager_v1` global.
///
/// A sandbox may not create another sandbox: the protocol says nesting is how a sandboxed client escapes its own
/// restrictions, so the manager is hidden from the connections that already have one. Every other connection may create
/// one, and what the sandbox buys is a *denied* role for the connection it creates.
pub fn register<D>(registrar: &Registrar<'_>) -> SecurityContextState
where
    D: GlobalDispatch<WpSecurityContextManagerV1, SecurityContextGlobalData>
        + Dispatch<WpSecurityContextManagerV1, GlobalData>
        + Dispatch<WpSecurityContextV1, SecurityContextUserData>
        + 'static,
{
    let access = std::sync::Arc::clone(registrar.access());
    SecurityContextState::new::<D, _>(registrar.display(), move |client: &Client| {
        access.role(&client.id()) != ClientRole::Sandboxed
    })
}

/// A client created a security context: its listening socket waits for the event loop.
pub fn context_created<H: Host>(host: &mut H, source: SecurityContextListenerSource, context: &SecurityContext) {
    log::debug!(area = "access", engine = ?context.sandbox_engine,
        app_id = ?context.app_id,
        instance = ?context.instance_id,
        "a security context was created"
    );
    host.pending_contexts().push(source);
}
