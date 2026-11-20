//! `gnoblin-wayland-wp-security-context-v1`: the sandbox listeners wait on the state for the event loop.

use gnoblin_wayland_protocols::wp_security_context_v1 as security_context;
use smithay::wayland::security_context::{SecurityContext, SecurityContextHandler, SecurityContextListenerSource};

use crate::State;

impl security_context::Host for State {
    fn pending_contexts(&mut self) -> &mut Vec<SecurityContextListenerSource> {
        &mut self.pending_contexts
    }
}

impl SecurityContextHandler for State {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        security_context::context_created(self, source, &context);
    }
}
