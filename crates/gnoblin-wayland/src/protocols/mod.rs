//! What has not yet moved into a crate of its own: the capture engine and the view-source protocol.
//!
//! Every Wayland protocol this crate serves becomes a crate under `crates/protocols/`, named
//! `gnoblin-wayland-<protocol>` (`crates/protocols/api/DESIGN.md`). The three standard capture protocols
//! (`ext_image_capture_source_v1`, `ext_image_copy_capture_v1` and `zwlr_screencopy_v1`) have moved. Their
//! crates hold the wire objects and ask the compositor for each answer through a host trait, and
//! `glue/` forwards those asks to the capture engine here.
//!
//! This module is deleted with the last of them. What remains waits for the owner's decision on how a View is
//! captured (the view-source protocol or a standard route): the capture engine names the View tables
//! and `gnoblin_view_source_v1` creates sources in them. The crates and what they advertise are the rows of
//! `xtask/protocols.toml`.

/// The pixel path the capture protocols share: frames, sources, sessions and the lock embargo.
pub mod capture_shared;
/// `gnoblin_view_source_v1`: the owner-only capture source of a View node.
pub mod gnoblin_view_source_v1;
/// The generated halves of the Gnoblin-owned protocols; see the module for why the
/// generated code carries the one `unsafe_code` allow in this crate.
mod wire;
