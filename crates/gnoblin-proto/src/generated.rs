//! The wire types, generated from the object schema by `build.rs`.
//!
//! Nothing here is written by hand: `build.rs` renders one `.proto` document per
//! schema namespace into `OUT_DIR`, compiles them with `proto/envelope.proto` (the
//! one hand-written file), and generates the Rust modules from the descriptor set.
//! The modules are named after the Protocol Buffers package: `gnoblin` is the
//! compositor's own namespace, and `ipc` is the envelope. An extension's types are not
//! here: its payloads are opaque to the stock crates and are generated outside this tree.
//!
//! The generated code is allowed to be less tidy than the hand-written code: it is
//! not reviewed as Rust, and a future protobuf release may emit shapes clippy would
//! otherwise object to.

/// The envelope, its four kinds and their bodies.
#[allow(clippy::all, dead_code)]
pub mod ipc {
    include!(concat!(env!("OUT_DIR"), "/gnoblin.ipc.rs"));
}

/// Objects in the `gnoblin` namespace.
#[allow(clippy::all, dead_code)]
pub mod gnoblin {
    include!(concat!(env!("OUT_DIR"), "/gnoblin.rs"));
}

/// The members the schema marks secret, rendered by `build.rs` from the object
/// schema into `OUT_DIR`.
///
/// Included rather than checked in, so the schema's `secret` flag is the only
/// place sensitivity is decided and no build can disagree with a committed table.
pub(crate) mod sensitive {
    include!(concat!(env!("OUT_DIR"), "/sensitive.rs"));
}

/// The tree namespace: the owner socket's wire types, one module per object.
#[allow(clippy::all, dead_code)]
pub mod tree {
    include!(concat!(env!("OUT_DIR"), "/tree.rs"));
}
