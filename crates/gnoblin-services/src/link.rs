//! The client types the session binary links its compat adapters with.
//!
//! The `gnoblin` binary is the composition root, and `xtask/src/deps.rs` (`binary_roots`)
//! forbids it from naming engine crates directly. The adapters it serves through this
//! role still need the compositor client, the wire version and the request `Fields`, so
//! this module hands out exactly those items through the role crate the binary already
//! depends on. Nothing here is new behaviour: each item is a re-export of its owner.

pub use gnoblin_client::{Client, UnixTransport, socket_path};
pub use gnoblin_model::Fields;
pub use gnoblin_proto::Version;
