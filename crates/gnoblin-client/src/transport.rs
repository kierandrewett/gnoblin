//! The transport seam: a socket that carries a handshake and then envelopes.
//!
//! [`gnoblin_proto::Transport`] owns the envelope path and the descriptor path. The
//! handshake is not an envelope but a fixed-layout first frame, so it needs this
//! small extension: a connection that can send and receive it before the envelope
//! stream starts.
//!
//! A real socket implements both traits in one type. The in-process loopback that
//! the tests use ([`crate::loopback`]) does the same, so the client above this seam
//! is exercised exactly as it will be on a socket.

use gnoblin_proto::Handshake;

use crate::error::HandshakeError;

/// A connection that begins with the fixed-layout handshake.
pub trait Transport: gnoblin_proto::Transport {
    /// Send the handshake frame. A client sends [`Handshake::hello`] first.
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError>;

    /// Receive the peer's handshake frame.
    ///
    /// A frame that is not a handshake is an error rather than something to skip: a
    /// peer that starts with an envelope has misunderstood the protocol, and a
    /// version mismatch after it would be unreadable.
    fn receive_handshake(&mut self) -> Result<Handshake, HandshakeError>;
}
