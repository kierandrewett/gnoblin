//! The provider ABI, both ends.
//!
//! A provider is a piece of desktop policy the compositor asks for decisions: where windows go, what a portal dialog shows,
//! what the configuration means. Std implements them as ordinary Rust, and the core calls them across the C ABI, because two
//! libraries share no Rust layout. This crate is the one place that knows both directions, so a change to a vtable is a
//! change to one file and the round trip is tested without a library boundary:
//!
//! - `export_*` turns a Rust provider into a vtable and the `user` pointer it is called with. Std calls these, then hands the
//!   pair to `provider_register`. Every function of the vtable is wrapped in `catch_unwind` and answers `E_PANIC` for a panic.
//! - `Foreign*` wraps a registered vtable as the Rust trait the compositor already drives ([`gnoblin_wm_api::WindowManager`],
//!   [`gnoblin_portal_ui::provider::UiProvider`]). The core calls these. A provider that panics, answers an error or sends
//!   an effect that does not decode is dropped for the rest of the session: one log line, then the null provider.
//!
//! Nothing here reads a clock, a file or the environment, and nothing logs: the owner passes a `report` function that says what
//! happened in its own log, because a log is a per-library singleton.
#![allow(unsafe_code)]

mod config;
mod ui;
mod wm;

pub use config::{ForeignConfig, Observe, export_config};
pub use ui::{ForeignUi, export_ui};
pub use wm::{ForeignWm, export_wm};

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{E_PANIC, GnoblinDropFn};

/// Says what happened in the owner's log: one line, in the owner's words.
pub type Report = Box<dyn Fn(&str) + Send + Sync>;

/// A provider ready to be registered: the vtable, the pointer each of its functions is called with and the function that
/// frees the pointer. Registering hands all three to the core, which calls `drop_user` once.
pub struct Exported<V: 'static> {
    /// The vtable, a static.
    pub vtable: &'static V,
    /// The provider's state.
    pub user: *mut c_void,
    /// Frees `user`.
    pub drop_user: GnoblinDropFn,
}

/// Runs `body` and answers its result, or `E_PANIC` when it panicked. Every function of an exported vtable is a `guarded` call,
/// because a panic unwinding out of an `extern "C"` function aborts the process.
pub(crate) fn guarded(body: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(E_PANIC)
}
