//! The IPC wire layer: length-prefixed Protocol Buffers frames, the RPC envelope,
//! the fixed-layout handshake, and the version rule that decides whether two
//! sides may speak at all.
//!
//! Why this shape (design/rust-compositor.md, "One IPC", "IPC extensions",
//! "Versioning" and D16): one socket serves shells, `gnoblinctl`, the portal, the
//! script runtime, the services role and extension providers, so the wire is
//! binary and generated from `gnoblin-schema`. Every frame is an [`Envelope`] that
//! names its namespace and member, which lets the compositor route an extension's
//! frames without decoding the extension's payload, and drop the payload of a
//! [`FLAG_SENSITIVE`] frame before it reaches a log.
//!
//! Frame layout: a four-byte big-endian body length, then the encoded envelope.
//! The first frame on a connection is not an envelope at all but a fixed-layout
//! [`Handshake`], so a version mismatch is always reportable. Descriptors travel
//! beside a frame through the [`Transport`] seam.
//!
//! The `.proto` files generated from the object schema live in `proto/` and are
//! checked by `cargo xtask gen-proto --check`. The envelope in [`envelope`] is
//! hand-written because it is not a schema member: it is the layer that carries
//! schema members.

mod envelope;
mod error;
mod frame;
mod handshake;
mod transport;
mod version;

pub use envelope::{Envelope, Error, Event, FLAG_SENSITIVE, Kind, Reply, Request, UNKNOWN_MEMBER};
pub use error::CodecError;
pub use frame::{LENGTH_PREFIX_LEN, MAX_FRAME_LEN, read_frame, write_frame};
pub use handshake::{HANDSHAKE_LEN, Handshake, HandshakeKind, MAGIC};
#[cfg(unix)]
pub use transport::Transport;
pub use version::{Mismatch, accepts, check};

/// The version type the wire, the schema and every generated surface share.
///
/// Re-exported so a client crate needs one import for the version it was built
/// against, rather than depending on `gnoblin-schema` for it.
pub use gnoblin_schema::Version;
