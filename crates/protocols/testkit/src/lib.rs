//! The real-client harness the Wayland protocol crates' tests share.
//!
//! Every protocol is proved from the other side of the socket: a headless session on a socket of its own and a client that binds
//! the globals a test exercises ([`session`]), run against the desktop users get ([`stock`]). A protocol crate's tests depend on
//! this crate as a development dependency; it is the only crate of the protocol tier that names the compositor engine.

pub mod session;
pub mod stock;
