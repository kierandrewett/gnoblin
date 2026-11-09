//! The privileged connections the guardian hands this compositor, and what each may do.
//!
//! Design/rust-compositor.md, "Process model": "Runtime and services connect over an
//! inherited file descriptor that carries extra capabilities: trusted input contexts,
//! input policy decisions, request handling and configuration commits", and "One IPC"
//! repeats it for the request layer. The guardian is the parent of every privileged
//! child, so it keeps one control socket open to this process for the whole session.
//! Each child's connection is sent down that socket as a handover that names the role
//! it was made for, and a child that restarts is handed a fresh connection the same way.
//! This module is this process's half: it reads those handovers and says what each role
//! is allowed to do.
//!
//! # Why the grant is the role's, and why it lives here
//!
//! A capability is a consequence of *which role the connection was made for*, and the
//! guardian's only statement about that is the role name in the handover. The table below
//! is therefore the compositor's, not the guardian's: the process that grants a power
//! decides who gets it, and a guardian that named a role this table does not know grants
//! nothing.
//!
//! # Deny by default
//!
//! A handover whose role grants nothing is dropped with a warning rather than served
//! unprivileged, because the guardian meant to hand over a power and the connection would
//! silently be an ordinary one. A connection that reaches the ordinary socket is granted
//! from its kernel peer uid, never from anything the client says (`socket_grants`). The
//! session owner's connections get the configuration reads and the reload request; no
//! other uid gets anything.

use std::io;
use std::os::unix::net::UnixStream;

use gnoblin_client::{Inherited, inherited_stream};
use gnoblin_proto::{Capabilities, Capability, Grant, privilege};

use crate::log;

/// The roles the table grants something to, in the order the server starts them.
///
/// Every role here has a non-empty grant and every grant has a role here. The test below
/// keeps the two in step, so a role added to [`grants`] cannot be forgotten at startup.
pub const ROLES: [&str; 4] = ["input", "portal", "runtime", "services"];

/// One privileged role this process serves, and the extension grants it advertises.
#[derive(Debug, Clone)]
pub struct Privileged {
    /// The role the connection is made for, as the table names it.
    pub role: String,
    /// The extension grants this role's socket advertises after every accept. Empty
    /// grants nothing; the core socket never carries grants.
    pub extension_grants: Vec<Grant>,
}

impl Privileged {
    /// One entry per role in [`ROLES`], with no extension grants.
    ///
    /// Each entry gets a stable client identity at startup, which survives the role's
    /// connections coming and going: the identity is what the rest of the compositor
    /// knows the role by, and a reconnect must not look like a new client to it.
    pub fn declared() -> Vec<Privileged> {
        ROLES
            .iter()
            .map(|role| Privileged {
                role: (*role).to_owned(),
                extension_grants: Vec::new(),
            })
            .collect()
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
        "runtime" => Capabilities::of(&[
            Capability::CommitConfig,
            Capability::ReadConfig,
            Capability::InputPolicy,
            Capability::WindowControl,
        ]),
        "services" => Capabilities::of(&[
            Capability::OfferRequest,
            Capability::RegisterLayer,
            Capability::ReadConfig,
        ]),
        "portal" => Capabilities::of(&[
            Capability::PortalUi,
            Capability::RetrieveSecret,
            Capability::CaptureScreen,
        ]),
        "input" => Capabilities::of(&[Capability::PortalInput]),
        _ => Capabilities::NONE,
    }
}

/// What a connection accepted from the ordinary socket is granted, from its peer's uid.
///
/// The session owner's own connections may read the configuration, ask for a reload and
/// change window state (close, minimise, maximise, fullscreen): the CLI does these, and a
/// reload still goes through the runtime, which is the one that commits. Commit itself
/// stays with the runtime role, so a same-uid shell cannot write
/// the document the runtime evaluates. Any other uid, or a peer whose credentials cannot
/// be read, is granted nothing.
pub fn owner_grants(peer_uid: Option<u32>, owner_uid: u32) -> Capabilities {
    match peer_uid {
        Some(uid) if uid == owner_uid => Capabilities::of(&[
            Capability::ReadConfig,
            Capability::ReloadConfig,
            Capability::WindowControl,
            Capability::TreeOwner,
        ]),
        _ => Capabilities::NONE,
    }
}

/// The grant for a connection accepted from the ordinary socket, read from the kernel.
///
/// `SO_PEERCRED` is the peer's uid as the kernel recorded it at `connect`, not anything the
/// client sent, so the grant cannot be claimed by a handshake. The compositor's own uid is
/// the session owner's: the socket lives in the owner's 0700 runtime directory.
pub fn socket_grants(stream: &UnixStream) -> Capabilities {
    let peer = rustix::net::sockopt::socket_peercred(stream)
        .ok()
        .map(|cred| cred.uid.as_raw());
    owner_grants(peer, rustix::process::getuid().as_raw())
}

/// Read handovers from `control` until the guardian closes it, passing each accepted
/// connection to `adopt` with the role it was made for.
///
/// This is the seam the server and the tests share: the server gives it the control
/// socket the guardian passed in, and the tests give it a socket pair they made. It runs
/// on its own thread, so a blocked read never holds up the serving threads.
///
/// A handover naming a role the table grants nothing to is dropped, which closes its
/// descriptor, and logged. A message that was read but refused is logged and skipped,
/// because the control socket is still usable. Only a read error ends the loop, since the
/// socket itself is then unusable and carrying on would mean serving nothing silently.
///
/// Returns when the guardian closes the control socket, which is how a session ends.
pub fn receive_handovers(control: &UnixStream, mut adopt: impl FnMut(String, UnixStream)) {
    loop {
        match privilege::receive_handover(control) {
            Ok(None) => return,
            Ok(Some(handover)) => {
                let role = handover.role;
                if grants(&role).is_empty() {
                    log::warn!(
                        area = "ipc",
                        role = %role,
                        "dropping a handover for a role this build grants nothing to"
                    );
                    continue;
                }
                adopt(role, UnixStream::from(handover.stream));
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                log::warn!(area = "ipc", error = %error, "skipping a handover the guardian sent malformed");
            }
            Err(error) => {
                log::warn!(
                    area = "ipc",
                    error = %error,
                    "the guardian's control socket failed; no more privileged connections will be accepted"
                );
                return;
            }
        }
    }
}

/// Adopts the guardian's control socket from this process's environment, if it passed one.
///
/// `Ok(None)` when the variable is absent. A compositor run by hand, or by a test, has no
/// guardian and therefore no privileged children to hand over, which is a session rather
/// than a fault. A variable that is present but does not name a descriptor is an error: a
/// guardian that meant to pass a control socket and passed garbage would otherwise leave
/// every privileged child it starts talking into a socket nobody reads.
pub fn guardian_from_environment() -> Result<Option<UnixStream>, Inherited> {
    inherited_stream(privilege::CONTROL_VARIABLE, |name| std::env::var(name).ok())
}

#[cfg(test)]
mod tests {
    use gnoblin_proto::Capability;

    use super::{ROLES, grants, owner_grants, socket_grants};

    #[test]
    fn the_capabilities_are_the_plan_s_two_privileged_roles() {
        let runtime = grants("runtime");
        assert!(
            runtime.contains(Capability::CommitConfig),
            "the runtime commits the configuration"
        );
        assert!(runtime.contains(Capability::InputPolicy), "and decides input policy");
        assert!(
            runtime.contains(Capability::ReadConfig),
            "and reads back what it committed, as it did before read-config was split out"
        );
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

        assert!(
            services.contains(Capability::ReadConfig),
            "the services role reads the accepted configuration to decide keyring.enabled"
        );

        let portal = grants("portal");
        assert!(portal.contains(Capability::PortalUi), "the relay asks for a dialog");
        assert!(
            portal.contains(Capability::RetrieveSecret),
            "and asks for an app's secret"
        );
        assert!(!portal.contains(Capability::OfferRequest), "but offers no other slot");
        assert!(
            portal.contains(Capability::CaptureScreen),
            "the relay asks for the pixels of a screenshot"
        );
        assert!(!portal.contains(Capability::PortalInput), "and routes no keys");

        let input = grants("input");
        assert!(input.contains(Capability::PortalInput), "the input relay routes keys");
        assert!(!input.contains(Capability::PortalUi), "and opens no dialog");

        // Deny by default: every other role, including one a newer guardian invented.
        for role in ["compositor", "invented", "guardian", "runtime2", ""] {
            assert_eq!(grants(role), gnoblin_proto::Capabilities::NONE, "`{role}`");
        }
    }

    #[test]
    fn the_declared_roles_are_exactly_the_roles_that_are_granted_something() {
        for role in ROLES {
            assert!(!grants(role).is_empty(), "`{role}` is declared but grants nothing");
        }
        for role in ["runtime", "services", "portal", "input"] {
            assert!(ROLES.contains(&role), "`{role}` is granted but never declared");
        }
    }

    #[test]
    fn the_session_owner_reads_and_reloads_the_configuration_and_cannot_commit_it() {
        let granted = owner_grants(Some(1000), 1000);
        assert_eq!(
            granted,
            gnoblin_proto::Capabilities::of(&[
                Capability::ReadConfig,
                Capability::ReloadConfig,
                Capability::WindowControl,
                Capability::TreeOwner,
            ])
        );
        // The owner's CLI asks the runtime to reload; the runtime is the one that commits.
        assert!(!granted.contains(Capability::CommitConfig));
        // The object tree is the owner's too, so a same-uid shell can read and change it.
        assert!(granted.contains(Capability::TreeOwner));
    }

    #[test]
    fn another_user_root_and_an_unknown_peer_are_granted_nothing() {
        assert_eq!(
            owner_grants(Some(1001), 1000),
            gnoblin_proto::Capabilities::NONE,
            "another uid"
        );
        assert_eq!(
            owner_grants(Some(0), 1000),
            gnoblin_proto::Capabilities::NONE,
            "root is not the owner"
        );
        assert_eq!(
            owner_grants(None, 1000),
            gnoblin_proto::Capabilities::NONE,
            "no peer credentials"
        );
    }

    #[test]
    fn the_tree_owner_capability_is_the_owner_grant_and_no_one_else_holds_it() {
        // The failure this prevents: tree members reachable from a role or a plain socket,
        // so a connection that was never the owner rewrites the object tree.
        for role in ROLES {
            assert!(
                !grants(role).contains(Capability::TreeOwner),
                "`{role}` holds tree-owner"
            );
        }
        assert!(
            !gnoblin_proto::Capabilities::NONE.contains(Capability::TreeOwner),
            "the default peer set"
        );
        assert!(owner_grants(Some(1000), 1000).contains(Capability::TreeOwner));
        assert!(
            !owner_grants(Some(1001), 1000).contains(Capability::TreeOwner),
            "another uid"
        );
        assert!(
            !owner_grants(None, 1000).contains(Capability::TreeOwner),
            "an unknown peer"
        );
    }

    #[test]
    fn a_socket_pair_made_by_this_process_gets_the_owner_grant() {
        // The peer of a socket pair is this process, so its uid is the owner's: the kernel
        // answer `socket_grants` reads, end to end rather than through the pure function.
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        assert_eq!(
            socket_grants(&ours),
            gnoblin_proto::Capabilities::of(&[
                Capability::ReadConfig,
                Capability::ReloadConfig,
                Capability::WindowControl,
                Capability::TreeOwner,
            ])
        );
    }
}
