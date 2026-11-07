//! The connections the guardian handed this compositor, and what each may do.
//!
//! Design/rust-compositor.md, "Process model": "Runtime and services connect over an
//! inherited file descriptor that carries extra capabilities: trusted input contexts,
//! input policy decisions, request handling and configuration commits", and "One IPC"
//! repeats it for the request layer. The guardian is the parent of both processes, so
//! it creates one socketpair per privileged child, keeps one end for this process and
//! hands the other to the child; this module is this process's half — it finds the ends
//! in its environment and says what each one is allowed to do.
//!
//! # Why the grant is a role's, and why it lives here
//!
//! A capability is a consequence of *which child the descriptor was made for*, and the
//! guardian's only statement about that is the variable's name
//! ([`gnoblin_proto::privilege::owner_variable`]). The table below is therefore the
//! compositor's, not the guardian's: the process that grants a power decides who gets
//! it, and a guardian that named a role this table does not know grants nothing.
//!
//! # Deny by default
//!
//! A variable whose role grants nothing is dropped with a warning rather than served
//! unprivileged, because the guardian meant to hand over a power and the connection
//! would silently be an ordinary one. A connection that reaches the ordinary socket is
//! never privileged, however it was started: the grant is read from the handshake this
//! side writes, not from anything the client says.

use std::fmt;
use std::os::unix::net::UnixStream;

use gnoblin_client::{Inherited, inherited_stream};

use gnoblin_proto::{Capabilities, Capability, privilege};

use crate::log;

/// One privileged connection this process was handed.
#[derive(Debug)]
pub struct Privileged {
    /// The role the descriptor was made for, as the guardian named it.
    pub role: String,
    /// This process's end of the pair.
    pub stream: UnixStream,
    /// The extension grants this socket advertises after every accept. Empty grants
    /// nothing; the core socket never carries grants.
    pub extension_grants: Vec<gnoblin_proto::Grant>,
}

/// Why the privileged connections could not be read.
#[derive(Debug)]
pub struct PrivilegedError {
    /// The role whose descriptor is unusable.
    role: String,
    /// What was wrong with it.
    reason: Inherited,
}

impl fmt::Display for PrivilegedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the {} connection the guardian passed cannot be used: {}",
            self.role, self.reason
        )
    }
}

impl std::error::Error for PrivilegedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.reason)
    }
}

impl Privileged {
    /// Every privileged connection this process's environment names.
    ///
    /// A variable that names a descriptor this build cannot adopt is an error rather
    /// than a skipped connection: the guardian created a pair expecting a role to use
    /// it, and serving without it would leave that child talking into a socket nobody
    /// reads.
    ///
    /// # Errors
    ///
    /// [`PrivilegedError`] naming the role and what was wrong with its descriptor.
    pub fn from_environment() -> Result<Vec<Privileged>, PrivilegedError> {
        let mut connections = Vec::new();
        for (name, value) in std::env::vars() {
            let Some(role) = privilege::owner_role(&name) else {
                continue;
            };
            if grants(&role).is_empty() {
                log::warn!(variable = %name, "[ipc] ignoring a privileged descriptor this build grants nothing to");
                continue;
            }
            let stream =
                inherited_stream(&name, |looked_up| (looked_up == name).then(|| value.clone())).map_err(|reason| {
                    PrivilegedError {
                        role: role.clone(),
                        reason,
                    }
                })?;
            if let Some(stream) = stream {
                connections.push(Privileged {
                    role,
                    stream,
                    extension_grants: Vec::new(),
                });
            }
        }
        connections.sort_by(|left, right| left.role.cmp(&right.role));
        Ok(connections)
    }
}

/// What a role's connection may do.
///
/// The runtime commits the configuration and decides input policy (D1: the script
/// runtime is a separate process and the compositor runs no Lua); the services role
/// offers the requests its agents need decided. No other role is privileged, and a
/// name this table does not have grants nothing.
pub fn grants(role: &str) -> Capabilities {
    match role {
        "runtime" => Capabilities::of(&[Capability::CommitConfig, Capability::InputPolicy]),
        "services" => Capabilities::of(&[Capability::OfferRequest]),
        "portal" => Capabilities::of(&[Capability::PortalUi]),
        "input" => Capabilities::of(&[Capability::PortalInput]),
        _ => Capabilities::NONE,
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_proto::Capability;

    use super::grants;

    #[test]
    fn the_capabilities_are_the_plan_s_two_privileged_roles() {
        let runtime = grants("runtime");
        assert!(
            runtime.contains(Capability::CommitConfig),
            "the runtime commits the configuration"
        );
        assert!(runtime.contains(Capability::InputPolicy), "and decides input policy");
        assert!(
            !runtime.contains(Capability::OfferRequest),
            "the runtime offers no requests: prompts belong to a shell or the services role"
        );

        let services = grants("services");
        assert!(services.contains(Capability::OfferRequest));
        assert!(
            !services.contains(Capability::CommitConfig),
            "the services role commits no configuration"
        );

        let portal = grants("portal");
        assert!(portal.contains(Capability::PortalUi), "the relay asks for a dialog");
        assert!(!portal.contains(Capability::PortalInput), "and routes no keys");

        let input = grants("input");
        assert!(input.contains(Capability::PortalInput), "the input relay routes keys");
        assert!(!input.contains(Capability::PortalUi), "and opens no dialog");

        // Deny by default: every other role, including one a newer guardian invented.
        for role in ["compositor", "invented", "guardian", "runtime2", ""] {
            assert_eq!(grants(role), gnoblin_proto::Capabilities::NONE, "`{role}`");
        }
    }
}
