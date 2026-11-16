//! The host's side of the core: what the compositor, the loader and the tests call, as Rust.
//!
//! A shell sees the C table. The code that hosts shells (the loader, and later the compositor edge that publishes events)
//! sees this module: load a module, activate or retire a generation, publish a signal into the shell's mailbox, read the
//! requests a shell made (reload, exit), and lend owned buffers back to a shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};

use crate::catalog;
use crate::registry;

pub use crate::loader::{Copying, LoadError, Module, load, retired_libraries};
pub use crate::memory::{lend_bytes, lend_text};
pub use crate::registry::{GenerationState, OwnedValue, SignalDelivery};

/// The function table this core hands to shells.
pub fn api() -> &'static GnoblinApi {
    &crate::table::API
}

/// A hash of the schema catalog this core was built with.
pub fn catalog_hash() -> u64 {
    catalog::CATALOG_HASH
}

/// Queues a signal for the shell, by catalog class and signal id. Any thread; wakes the shell's dispatch loop.
pub fn publish(delivery: SignalDelivery) {
    registry::publish(delivery);
}

/// Queues a signal for the shell by name (`("lock", "requested")`), for a caller that has no ids.
///
/// # Errors
///
/// The class or signal is not in the catalog.
pub fn publish_named(class: &str, signal: &str, node: u64, values: Vec<OwnedValue>) -> Result<(), String> {
    let class_id = catalog::class_id(class).ok_or_else(|| format!("no class `{class}` in the catalog"))?;
    let signal_id =
        catalog::signal_id(class_id, signal).ok_or_else(|| format!("class `{class}` has no signal `{signal}`"))?;
    publish(SignalDelivery {
        class: class_id,
        signal: signal_id,
        node,
        values,
    });
    Ok(())
}

/// How many published signals no `dispatch` has taken yet.
pub fn queued_events() -> usize {
    registry::queued_events()
}

static RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);
static EXIT_REQUEST: Mutex<Option<i32>> = Mutex::new(None);

/// True once, if a shell asked for a reload since the last call. The host reloads between dispatches.
pub fn take_reload_request() -> bool {
    RELOAD_REQUESTED.swap(false, Ordering::AcqRel)
}

/// The exit code a shell asked for, once.
pub fn take_exit_request() -> Option<i32> {
    EXIT_REQUEST.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// `request_reload`.
pub(crate) fn request_reload(ctx: GnoblinCtx) -> i32 {
    if let Err(code) = check_live(ctx, "request_reload") {
        return code;
    }
    RELOAD_REQUESTED.store(true, Ordering::Release);
    gnoblin_core_abi::E_OK
}

/// `request_exit`.
pub(crate) fn request_exit(ctx: GnoblinCtx, code: i32) -> i32 {
    if let Err(rc) = check_live(ctx, "request_exit") {
        return rc;
    }
    *EXIT_REQUEST.lock().unwrap_or_else(PoisonError::into_inner) = Some(code);
    gnoblin_core_abi::E_OK
}

/// A request is only honoured from a generation that has not retired; a retired one is counted as a stale call.
fn check_live(ctx: GnoblinCtx, what: &str) -> Result<(), i32> {
    registry::check_ctx(ctx, what, false).map(|_| ())
}
