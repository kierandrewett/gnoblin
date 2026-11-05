//! `wp_security_context_v1`: application sandboxes.
//!
//! A sandboxed application does not get a separate socket with a policy in it: it asks
//! the compositor it is already connected to for a *listener*, and connects its own
//! child processes there. Each connection that arrives on that listener is a client of
//! its own — its own registry, its own objects — and it is marked
//! [`crate::access::ClientRole::Sandboxed`] before its first request is dispatched, which is
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

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::security_context::{
    SecurityContext, SecurityContextHandler, SecurityContextListenerSource, SecurityContextState,
};
use tracing::debug;

use crate::State;

impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        debug!(
            engine = ?context.sandbox_engine,
            app_id = ?context.app_id,
            instance = ?context.instance_id,
            "[access] a security context was created"
        );
        self.pending_contexts.push(source);
    }
}

/// The state's security-context global, named so the field has one owner.
///
/// `may_create` decides which connections may create a sandbox at all. The compositor
/// passes "not already sandboxed", because nesting contexts is the documented way for
/// a sandboxed client to climb out of its restrictions.
pub(crate) fn global(
    display_handle: &DisplayHandle,
    may_create: impl Fn(&smithay::reexports::wayland_server::Client) -> bool + Send + Sync + 'static,
) -> SecurityContextState {
    SecurityContextState::new::<State, _>(display_handle, may_create)
}
