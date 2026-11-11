//! The privileged connections the guardian hands this compositor, and who may use them.
//!
//! Design/rust-compositor.md, "Process model": "Runtime and services connect over an
//! inherited file descriptor". "One IPC" repeats it for the request layer. The guardian is
//! the parent of every privileged child, so it keeps one control socket open to this process
//! for the whole session. Each child's connection is sent down that socket as a handover
//! that names the role it was made for, and a child that restarts is handed a fresh
//! connection the same way. This module is this process's half: it reads those handovers
//! and says which connections are the session owner's.
//!
//! # One predicate: the owner connection
//!
//! A connection is either the *owner* or it is not. The owner is every connection the
//! guardian handed over (the runtime, the services, the portal and input relays: all of them
//! are same-user helpers the guardian started), and every connection that reaches the
//! ordinary socket from the session owner's uid, read from the kernel (`SO_PEERCRED`). A
//! handover naming a role this build does not know is dropped with a warning rather than
//! served, because the guardian meant to hand over a power and would otherwise get an
//! ordinary connection silently. Any other connection, including another uid, root, or a
//! peer whose credentials cannot be read, is not the owner and is refused the owner's
//! members with `denied`.
//!
//! The predicate is the only gate for owner members: there are no per-role capability
//! sets. The Wayland-level client roles (`ClientRole::View`, `ClientRole::FrameRenderer`)
//! are a separate mechanism and are unaffected.

use std::io;
use std::os::unix::net::UnixStream;

use gnoblin_client::{Inherited, inherited_stream};
use gnoblin_proto::privilege;

use crate::log;

/// The roles a guardian may hand over, in the order the server starts them.
///
/// A handover naming a role outside this list is dropped. Every role here is an owner
/// connection; the list only says which names the guardian may use.
pub const ROLES: [&str; 4] = ["input", "portal", "runtime", "services"];

/// One privileged role this process serves.
#[derive(Debug, Clone)]
pub struct Privileged {
    /// The role the connection is made for, as the table names it.
    pub role: String,
}

impl Privileged {
    /// One entry per role in [`ROLES`].
    ///
    /// Each entry gets a stable client identity at startup, which survives the role's
    /// connections coming and going: the identity is what the rest of the compositor
    /// knows the role by, and a reconnect must not look like a new client to it.
    pub fn declared() -> Vec<Privileged> {
        ROLES
            .iter()
            .map(|role| Privileged {
                role: (*role).to_owned(),
            })
            .collect()
    }
}

/// Whether a connection whose peer uid is `peer_uid` is the session owner's.
///
/// `None` (no peer credentials) is not the owner. The owner is the uid the compositor runs
/// as: the socket lives in that uid's 0700 runtime directory.
pub fn owner_of(peer_uid: Option<u32>, owner_uid: u32) -> bool {
    peer_uid == Some(owner_uid)
}

/// Whether a connection accepted from the ordinary socket is the session owner's, read from
/// the kernel.
///
/// `SO_PEERCRED` is the peer's uid as the kernel recorded it at `connect`, not anything the
/// client sent, so the answer cannot be claimed by a handshake.
pub fn socket_is_owner(stream: &UnixStream) -> bool {
    let peer = rustix::net::sockopt::socket_peercred(stream)
        .ok()
        .map(|cred| cred.uid.as_raw());
    owner_of(peer, rustix::process::getuid().as_raw())
}

/// Read handovers from `control` until the guardian closes it, passing each accepted
/// connection to `adopt` with the role it was made for.
///
/// This is the seam the server and the tests share: the server gives it the control
/// socket the guardian passed in, and the tests give it a socket pair they made. It runs
/// on its own thread, so a blocked read never holds up the serving threads.
///
/// A handover naming a role outside [`ROLES`] and other than [`privilege::VIEW_ROLE`] is dropped,
/// which closes its descriptor, and logged. The view role is not an owner connection: its stream is
/// a Wayland connection, which `adopt` gives to the Wayland server with the capture-only `view` role. A message that was read but refused is logged and skipped, because the control
/// socket is still usable. Only a read error ends the loop, since the socket itself is then
/// unusable and carrying on would mean serving nothing silently.
///
/// Returns when the guardian closes the control socket, which is how a session ends.
pub fn receive_handovers(control: &UnixStream, mut adopt: impl FnMut(String, UnixStream)) {
    loop {
        match privilege::receive_handover(control) {
            Ok(None) => return,
            Ok(Some(handover)) => {
                let role = handover.role;
                if !ROLES.contains(&role.as_str()) && role != privilege::VIEW_ROLE {
                    log::warn!(
                        area = "ipc",
                        role = %role,
                        "dropping a handover for a role this build does not know"
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
    use super::{ROLES, owner_of, receive_handovers, socket_is_owner};
    use gnoblin_proto::privilege;

    #[test]
    fn the_owner_is_the_uid_the_compositor_runs_as() {
        assert!(owner_of(Some(1000), 1000), "the owner's own uid");
        assert!(!owner_of(Some(1001), 1000), "another uid");
        assert!(!owner_of(Some(0), 1000), "root is not the owner");
        assert!(!owner_of(None, 1000), "a peer whose credentials cannot be read");
    }

    #[test]
    fn a_socket_pair_made_by_this_process_is_the_owner() {
        // The peer of a socket pair is this process, so its uid is the owner's: the kernel
        // answer `socket_is_owner` reads, end to end rather than through the pure function.
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        assert!(socket_is_owner(&ours));
    }

    #[test]
    fn the_roles_the_guardian_may_hand_over_are_the_declared_ones() {
        for role in ["runtime", "services", "portal", "input"] {
            assert!(ROLES.contains(&role), "`{role}` is a handover role");
        }
        for role in ["compositor", "invented", "guardian", "runtime2", ""] {
            assert!(!ROLES.contains(&role), "`{role}` is not a handover role");
        }
        assert!(
            !ROLES.contains(&privilege::VIEW_ROLE),
            "the view role is a Wayland connection, not an owner IPC connection"
        );
    }

    #[test]
    fn the_view_role_is_adopted_and_an_unknown_role_is_dropped() {
        let (guardian, compositor) = rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("a control socket");
        let (stream, _peer) = std::os::unix::net::UnixStream::pair().expect("a stream");
        for role in ["invented", privilege::VIEW_ROLE, "runtime"] {
            privilege::send_handover(&guardian, role, &stream).expect("a handover");
        }
        drop(guardian);
        let mut adopted = Vec::new();
        receive_handovers(&std::os::unix::net::UnixStream::from(compositor), |role, _| {
            adopted.push(role);
        });
        assert_eq!(
            adopted,
            ["view", "runtime"],
            "the unknown role never reaches the adopter"
        );
    }
}
