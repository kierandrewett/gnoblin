//! The protocol modules that have not yet moved into crates of their own.
//!
//! Every Wayland protocol this crate serves becomes a crate under `crates/protocols/`, named
//! `gnoblin-wayland-<protocol>` (`design/protocol-crates.md`). A module answers one protocol's requests
//! and translates them into window-manager events; none of them decides placement, focus or stacking.
//! What is left here is moved one family per commit, and this module is deleted with the last of them.
//! The crates and what they advertise are the rows of `xtask/protocols.toml`.

/// Pixel path shared by the three capture protocols: frames, sources and sessions.
pub mod capture_shared;
/// `ext_image_capture_source_v1`: the output and toplevel sources a capture names.
pub mod ext_image_capture_source_v1;
/// `ext_image_copy_capture_v1`: image-copy sessions that stream frames into client buffers.
pub mod ext_image_copy_capture_v1;
/// `gnoblin_view_source_v1`: the owner-only capture source of a View node.
pub mod gnoblin_view_source_v1;
/// The session and frame objects of `ext_image_copy_capture_v1`, with the device stated as the wire carries it.
pub mod image_copy_session;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
/// `zwlr_screencopy_manager_v1`: one-shot screencopy of an output or region.
pub mod zwlr_screencopy_v1;
