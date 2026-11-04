//! The wire types, compiled from the checked-in `.proto` files by `build.rs`.
//!
//! Nothing here is written by hand: `cargo xtask gen-proto` writes the namespace
//! files from the object schema, `proto/envelope.proto` is the one hand-written
//! file, and both are compiled together. The modules are named after the Protocol
//! Buffers package, so `gnoblin` is the compositor's own namespace, `bsm_greeter`
//! is the greeter extension, and `ipc` is the envelope.
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

/// Objects in the `bsm.greeter` extension namespace.
#[allow(clippy::all, dead_code)]
pub mod bsm_greeter {
    include!(concat!(env!("OUT_DIR"), "/bsm.greeter.rs"));
}
