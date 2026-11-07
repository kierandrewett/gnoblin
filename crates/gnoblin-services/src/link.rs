//! The client types `gnoblin-compat-host` links its compat adapters with.
//!
//! The `gnoblin` binary is the composition root, and `xtask/src/deps.rs` (`binary_roots`)
//! forbids it from naming engine crates directly. The adapters the host serves through this
//! role still need the compositor client, the wire version and the request `Fields`, so
//! this module hands out exactly those items through the role crate the binary already
//! depends on. Nothing here is new behaviour: each item is a re-export of its owner.

/// Compositor client, socket and transport, for the adapters the binary serves.
pub use gnoblin_client::{Client, UnixTransport, socket_path};
/// Request fields, ordered by name so a snapshot, a frame and an assertion agree.
pub use gnoblin_model::Fields;
/// The wire protocol version the adapters speak.
pub use gnoblin_proto::Version;
/// Linked-agent plumbing: the host, the agent trait, offer failures, request ids, outcomes and ticks.
pub use gnoblin_service_core::{Host, LinkedAgent, OfferFailure, RequestId, RequestOutcome, Tick};
