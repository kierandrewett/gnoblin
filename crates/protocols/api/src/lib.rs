//! What a protocol crate may ask of the compositor, and what it must tell it back.
//!
//! Every Wayland protocol that `gnoblin-wayland` serves is a crate of its own, named
//! `gnoblin-wayland-<protocol>`. A protocol crate depends on this crate and on nothing in
//! `gnoblin-wayland`: it states the compositor operations it needs as a trait, and
//! `gnoblin-wayland` implements that trait on its `State`. The direction of the edges is what
//! lets one protocol change, build and test without the compositor behind it
//! (`design/protocol-crates.md`).
//!
//! The crate holds only what two or more protocol crates share:
//!
//! - [`protocol`]: the table every crate fills in ([`ProtocolInfo`]), and the handle a crate
//!   creates its globals through ([`Registrar`]);
//! - [`host`]: the compositor operations more than one protocol needs.
//!
//! A need that only one protocol has lives in that protocol's own host trait. An operation
//! enters this crate when a second crate needs it, so the crate never becomes a copy of `State`.

pub mod host;
pub mod protocol;

pub use protocol::{Privilege, ProtocolInfo, Registrar, ServedInterface};
