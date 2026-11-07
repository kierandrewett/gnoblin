//! The compat host: the one crate that knows which compat adapters exist and how each process
//! wires them.
//!
//! The `gnoblin` binary is dispatch and nothing else (`crates/gnoblin/src/main.rs`). The wiring
//! it used to carry lives here, so the binary names this crate and no adapter. Only this crate
//! and the tests link the adapter crates; `xtask/src/deps.rs` enforces that.
//!
//! The crate has two modules, one per process that serves adapters:
//!
//! - [`session`] is the compositor role's side. It builds the compat `Extension` values from the
//!   adapters' support matrices, registers them on the session configuration, builds the XSETTINGS
//!   owner, and exports the Qt and dconf settings into the guardian's environment, which every child
//!   inherits.
//!   The `Extension` registry (`compat`, the Redirect and Interop adapters) is compositor-side and
//!   stays in this crate until the `Extension` seam is decided; it is split out later.
//! - [`services`] is the services role's side. It holds the D-Bus agents that serve the compat
//!   interfaces (dconf layer, DisplayConfig, IdleMonitor, introspection), and the
//!   list of linked agents the role starts.
//!
//! # Why the services glue is here and not in `gnoblin-services`
//!
//! `gnoblin-services` is the services role's library, but it must name no adapter: a service crate
//! that did would leak an adapter into every process that links it. The services role is still a
//! client of the compositor, and the glue that serves an adapter on its connection is wiring, which
//! is this crate's job. So the role's library stays adapter-free and the adapter wiring has one home.
//!
//! # Why the session wiring is here and not in `gnoblin-compositor`
//!
//! The compositor is the engine and must not link a compat adapter: the adapters reach it through
//! the `Extension` and `XSettings` seams it exposes, and the compositor never names an adapter. This
//! crate is the only place that turns adapters into those seam values.
//!
//! # Re-exports
//!
//! [`Mode`] is re-exported so the binary can name the default mode without depending on
//! `gnoblin-compat-core` directly.

pub mod services;
pub mod session;

pub use gnoblin_compat_core::Mode;

gnoblin_log::logger!(log = "gnoblin");
