//! Where the process model's inherited descriptors are named, and how they are handed over.
//!
//! Design/rust-compositor.md, "Process model": the runtime and the services role
//! connect to the compositor "over an inherited file descriptor that carries extra
//! capabilities". The guardian is what creates the descriptor (it is the parent of
//! both processes), and a descriptor can only cross an `exec` as a number in the
//! environment, so the name of that number is part of the IPC's contract rather than
//! of one process.
//!
//! Two variables, both built from the constants here so the spelling exists once:
//!
//! - [`FD_VARIABLE`] carries the descriptor a privileged child is given. The child
//!   reads it, adopts the connection, and the handshake tells it what the compositor
//!   granted ([`crate::Capability`]).
//! - [`CONTROL_VARIABLE`] carries the guardian's control socket into the compositor.
//!   It is how a fresh pair reaches the compositor, one message per spawn: see
//!   [`handover_payload`] and [`handover_role`].
//!
//! Each spawn of a privileged child gets its own socket pair, so a child that dies
//! takes its end with it and the compositor sees the end of stream. The descriptor
//! the compositor keeps is never the one the child held: a pair that outlived its
//! child would carry the dead child's unread bytes into the next session.
//!
//! A descriptor in the environment is only accessible when `FD_CLOEXEC` is clear, so
//! the guardian clears it for exactly the descriptor a child is given and for exactly
//! the one spawn it is given in. Nothing else in the environment is privileged: a
//! client that connects to the socket by path holds
//! [`Capabilities::NONE`](crate::Capabilities::NONE) however it was started.

use std::io::{self, IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, OwnedFd};

use rustix::cmsg_space;
use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer, SendAncillaryMessage,
    SendFlags, recvmsg, sendmsg,
};

/// The variable that carries this process's privileged IPC descriptor.
pub const FD_VARIABLE: &str = "GNOBLIN_IPC_FD";

/// The variable that carries the guardian's control socket into the compositor.
///
/// The compositor reads the descriptor from here once, at start. Every later privileged
/// connection arrives as a message on it, so the compositor never needs a variable per
/// role: the role is part of the message.
pub const CONTROL_VARIABLE: &str = "GNOBLIN_IPC_CONTROL_FD";

/// The payload of one handover message: the role the stream is for.
///
/// The stream itself travels as `SCM_RIGHTS` ancillary data; the payload says which
/// role it belongs to. A role name is the only thing the compositor needs to choose
/// the grants, and it is what the compositor checks against the roles it expects.
pub fn handover_payload(role: &str) -> &[u8] {
    role.as_bytes()
}

/// The role a handover payload names, or `None` for a payload that names none.
///
/// `None` for an empty payload and for bytes that are not UTF-8, because a stream whose
/// role cannot be read must not be given any role's grants. The caller closes the
/// stream rather than guessing, since a guessed role would hand it another child's
/// capabilities.
pub fn handover_role(payload: &[u8]) -> Option<&str> {
    std::str::from_utf8(payload).ok().filter(|role| !role.is_empty())
}

/// The most a handover payload may carry. A role name is a short word; the bound makes the
/// receiving buffer fixed-size, and a longer payload is refused rather than cut, because a
/// cut role name could name a different role than the guardian meant.
pub const MAX_HANDOVER_PAYLOAD: usize = 64;

/// One privileged stream the guardian handed to the compositor.
#[derive(Debug)]
pub struct Handover {
    /// The role the stream was made for, as the guardian wrote it.
    pub role: String,
    /// The compositor's end of the spawn's pair. Owned, so a refused handover closes it
    /// on drop rather than leaking a descriptor the compositor never serves.
    pub stream: OwnedFd,
}

/// Send `stream` to the compositor over `control`, tagged with `role`.
///
/// The control socket is `SOCK_SEQPACKET` so one `sendmsg` is exactly one handover: the
/// descriptor cannot be coalesced with a neighbouring message, and a reader never sees
/// half a role name. `SCM_RIGHTS` duplicates the descriptor into the receiver, so the
/// sender may close its own copy as soon as this returns; the guardian does, which is
/// what makes the compositor the only holder of the end it serves.
///
/// # Errors
///
/// `InvalidInput` for a role that is empty or too long to carry; otherwise the error
/// `sendmsg` reported.
pub fn send_handover(control: impl AsFd, role: &str, stream: impl AsFd) -> io::Result<()> {
    let payload = handover_payload(role);
    if payload.is_empty() || payload.len() > MAX_HANDOVER_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a handover role must be non-empty and short enough to carry",
        ));
    }
    let descriptors = [stream.as_fd()];
    let mut space = [MaybeUninit::uninit(); cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    if !ancillary.push(SendAncillaryMessage::ScmRights(&descriptors)) {
        return Err(io::Error::other(
            "the handover descriptor did not fit its control buffer",
        ));
    }
    let sent = sendmsg(control, &[IoSlice::new(payload)], &mut ancillary, SendFlags::NOSIGNAL)?;
    if sent == payload.len() {
        Ok(())
    } else {
        // A SEQPACKET send is all or nothing, so a short count means the kernel broke the
        // contract; report it rather than let the caller believe the handover went.
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "the handover was only partly sent",
        ))
    }
}

/// Receive the next handover from `control`.
///
/// `Ok(None)` is an orderly end: the guardian closed its end, which is how the compositor
/// learns the session is over. `InvalidData` names a message that was read but refused
/// (truncated, no descriptor, or no role). Its descriptor is closed on the way out and the
/// socket is still usable, so the caller logs and carries on. Any other error means the
/// socket itself is unusable and the caller should stop reading it.
///
/// The received descriptor is close-on-exec, so it does not leak into a child the
/// compositor later starts.
///
/// # Errors
///
/// See above.
pub fn receive_handover(control: impl AsFd) -> io::Result<Option<Handover>> {
    let mut payload = [0u8; MAX_HANDOVER_PAYLOAD];
    let mut space = [MaybeUninit::uninit(); cmsg_space!(ScmRights(1))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let received = recvmsg(
        control,
        &mut [IoSliceMut::new(&mut payload)],
        &mut ancillary,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    // Take the first descriptor and let the rest drop: an extra descriptor is one the
    // guardian did not mean to send, and holding it would keep a stream open that nothing
    // serves. Dropping the `OwnedFd` closes it.
    let mut stream: Option<OwnedFd> = None;
    for message in ancillary.drain() {
        if let RecvAncillaryMessage::ScmRights(descriptors) = message {
            for descriptor in descriptors {
                if stream.is_none() {
                    stream = Some(descriptor);
                }
            }
        }
    }
    if received.bytes == 0 && stream.is_none() {
        return Ok(None);
    }
    if received.flags.intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a handover was longer than the compositor reads",
        ));
    }
    let Some(role) = handover_role(&payload[..received.bytes]) else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a handover names no role"));
    };
    let Some(stream) = stream else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a handover carries no descriptor",
        ));
    };
    Ok(Some(Handover {
        role: role.to_owned(),
        stream,
    }))
}

/// The descriptor number a variable's value names.
///
/// `None` for a value that is not a plain non-negative integer, which is what a
/// caller reports rather than guessing a descriptor: adopting the wrong number would
/// take the connection away from the process that owns it.
pub fn descriptor(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok().filter(|number| *number >= 0)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    use super::{
        CONTROL_VARIABLE, FD_VARIABLE, descriptor, handover_payload, handover_role, receive_handover, send_handover,
    };

    #[test]
    fn a_role_round_trips_through_its_handover_payload() {
        assert_eq!(handover_role(handover_payload("runtime")), Some("runtime"));
        assert_eq!(handover_role(handover_payload("services")), Some("services"));
    }

    #[test]
    fn a_payload_that_names_no_role_is_refused() {
        // An empty payload and a payload that is not text name no role: the stream is
        // closed rather than given the grants of some role it might have been meant for.
        assert_eq!(handover_role(b""), None);
        assert_eq!(handover_role(&[0xff, 0xfe]), None);
    }

    #[test]
    fn the_two_variables_are_distinct() {
        // A child reads FD_VARIABLE; the compositor reads CONTROL_VARIABLE. Confusing the two
        // would hand the compositor a child's end, which is the bug the per-spawn pair removes.
        assert_ne!(FD_VARIABLE, CONTROL_VARIABLE);
    }

    #[test]
    fn only_a_plain_descriptor_number_is_accepted() {
        assert_eq!(descriptor("7"), Some(7));
        assert_eq!(descriptor(" 12 "), Some(12));
        assert_eq!(descriptor("-3"), None);
        assert_eq!(descriptor(""), None);
        assert_eq!(descriptor("/tmp/socket"), None);
    }

    /// A `SOCK_SEQPACKET` pair standing in for the guardian-to-compositor control socket.
    fn control_pair() -> (std::os::fd::OwnedFd, std::os::fd::OwnedFd) {
        rustix::net::socketpair(
            rustix::net::AddressFamily::UNIX,
            rustix::net::SocketType::SEQPACKET,
            rustix::net::SocketFlags::CLOEXEC,
            None,
        )
        .expect("a control socket pair")
    }

    #[test]
    fn a_handover_carries_its_role_and_a_working_stream() {
        // The stream the guardian sends is one end of a fresh pair; the compositor must get
        // a descriptor that reaches the other end, or the handover is useless.
        let (guardian, compositor) = control_pair();
        let (mut runtime, served) = UnixStream::pair().expect("a stream pair");
        send_handover(&guardian, "runtime", &served).expect("the handover is sent");
        drop(served);

        let handover = receive_handover(&compositor)
            .expect("the handover is received")
            .expect("a handover, not an orderly end");
        assert_eq!(handover.role, "runtime");
        let mut received = UnixStream::from(handover.stream);
        runtime.write_all(b"ping").expect("the runtime writes");
        let mut buffer = [0u8; 4];
        received.read_exact(&mut buffer).expect("the compositor reads");
        assert_eq!(&buffer, b"ping");
    }

    #[test]
    fn a_closed_control_socket_is_an_orderly_end() {
        // The guardian closing its end is how the compositor learns the session is over, so
        // it must read as `None` rather than as a failure to log on every shutdown.
        let (guardian, compositor) = control_pair();
        drop(guardian);
        assert!(receive_handover(&compositor).expect("a clean read").is_none());
    }

    #[test]
    fn a_role_too_long_or_empty_is_not_sent() {
        // Refusing here keeps a malformed role from ever reaching the compositor, where it
        // would have to be guessed at.
        let (guardian, _compositor) = control_pair();
        let (_runtime, served) = UnixStream::pair().expect("a stream pair");
        let long = "x".repeat(super::MAX_HANDOVER_PAYLOAD + 1);
        assert_eq!(
            send_handover(&guardian, &long, &served).expect_err("too long").kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            send_handover(&guardian, "", &served).expect_err("empty").kind(),
            std::io::ErrorKind::InvalidInput
        );
    }
}
