//! Where descriptors travel beside a frame.
//!
//! Design/rust-compositor.md, D16: frames are Protocol Buffers on
//! `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`, and file descriptors ride alongside a
//! frame through `SCM_RIGHTS`. This trait is the seam for that transport, not the
//! transport itself.
//!
//! Why the seam and not the socket yet: receiving ancillary data needs
//! `recvmsg` with a control buffer, and the workspace forbids `unsafe` outside a
//! crate that declares its own lint table and confines it to a named FFI shim.
//! The crate that owns the socket will do that and implement this trait; until
//! then everything above the transport (framing, envelope, handshake, version
//! rule) is written and tested against byte streams, so nothing above it changes
//! when the descriptor path lands.
//!
//! A descriptor here is an identity, not a capability the receiver may use: the
//! compositor still checks that the connection may ask for what the descriptor
//! carries.

/// The connection seam that carries frames and their descriptors. Unix only, because
/// descriptor passing is a Unix mechanism.
#[cfg(unix)]
pub use unix::Transport;

#[cfg(unix)]
mod unix {
    use std::os::fd::{BorrowedFd, OwnedFd};

    use crate::Envelope;
    use crate::error::CodecError;

    /// A connection that carries frames and their descriptors.
    ///
    /// Implementors own the socket. The contract is one frame per call in the
    /// order written: a transport never splits or coalesces frames, and a receiver
    /// either gets the whole frame with its descriptors or an error.
    pub trait Transport {
        /// Send one frame and the descriptors that belong to it.
        ///
        /// The descriptors are borrowed and stay the caller's to close; a
        /// transport duplicates them into the socket's control message and must
        /// not close them.
        fn send(&mut self, envelope: &Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError>;

        /// Receive one frame, appending any descriptors that arrived with it to
        /// `fds`.
        ///
        /// Returns `Ok(None)` when the peer closed the connection cleanly between
        /// frames. Descriptors appended here are owned by the caller.
        fn receive(&mut self, fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError>;
    }
}
