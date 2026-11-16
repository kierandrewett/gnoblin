//! `libgnoblin-core`: the implementation of the shell ABI.
//!
//! This crate is built as a `cdylib` whose SONAME is `libgnoblin-core.so.1` (set in `build.rs`; Cargo names the file
//! `libgnoblin_core_impl.so`, and dist installs it under the SONAME). It exports four symbols (`exports.rs`) and one
//! versioned function table (`table.rs`); everything a shell does goes through the table.
//!
//! What it holds now, and what is still to come:
//!
//! - `registry`: the C handles over `gnoblin-shell-host`'s generations, registrations, event queue and dispatch loop, which
//!   keep the invariants of `design/modular-framework.md` section 5.4 and 5.5 (`E_RETIRED` after retirement, `drop_user`
//!   exactly once, staged registrations inactive, `E_THREAD` off the shell thread).
//! - panic containment for every exported function and every call into shell code, and the loader (unique-copy `dlopen`,
//!   the entry check, `init`, and the bounded retire list): both are `gnoblin-shell-host`'s, used through `host`.
//! - `catalog`: the schema catalog, generated at build time, behind the `catalog_*` functions.
//! - `memory`: owned buffers lent to a shell and taken back.
//! - `host`: the Rust interface the compositor edge and the tests use.
//!
//! The compositor is not linked here yet: no tree, no Wayland, no `gnoblin_main`. Signals reach the shell through
//! [`host::publish`], which is also what the compositor edge will call. Nothing outside this crate's own tests and its
//! `rlib_control` example may depend on it as a Rust library (`impl-sealed`): a shell that does carries a second copy of
//! the core, which is the failure `cargo xtask dist-check` exists to catch.

gnoblin_log::logger!(pub(crate) log = "core");

// Panic containment and the last-error text live in the shell host, which the loader needs as well; the core's own
// modules reach them as `crate::guard`.
use gnoblin_shell_host::guard;

mod catalog;
mod exports;
pub mod host;
mod memory;
mod registry;
mod table;

#[cfg(feature = "test-driver")]
mod driver;

pub use exports::{gnoblin_abi_version, gnoblin_catalog_hash, gnoblin_get_api, gnoblin_last_error};

#[cfg(feature = "test-driver")]
pub use driver::gnoblin_test_main;

#[cfg(test)]
mod tests;
