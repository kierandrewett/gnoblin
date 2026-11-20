//! The plain-data types the Wayland protocol crates and the compositor both name.
//!
//! A protocol crate may depend on `gnoblin-wayland-api` and on this crate, and on nothing else of the workspace
//! (`design/protocol-crates.md`). Some protocols speak in the compositor's own terms: a head the output manager
//! reports, a request a window is asked for. Those values are defined once, here, so a protocol crate and the
//! compositor that serves it agree on them without either naming the other.
//!
//! The crate holds data and trivial helpers. Anything that decides something about the compositor stays in
//! `gnoblin-wayland`.

pub mod activation;
pub mod decoration;
pub mod output;
