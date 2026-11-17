//! The one call into the core's table that the SDK has no wrapper for.
//!
//! Everything else a shell does goes through `gnoblin_core::Gnoblin`. A runtime also has to say why a `start` failed, which the
//! loader reads from the core's last-error text, and that text is set through the table `start` is handed.
#![allow(unsafe_code)]

use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};

/// The table `start` was handed.
///
/// # Safety
///
/// `api` must be null or the pointer the loader passes to `start`: the core's static table, which is never unmapped.
pub(crate) unsafe fn from_table(api: *const GnoblinApi) -> Option<&'static GnoblinApi> {
    // SAFETY: the caller guarantees `api` is null or the core's static table, valid for `'static`.
    unsafe { api.as_ref() }
}

/// Sets the text the loader reports when `start` or `activate` of the generation `ctx` fails.
pub(crate) fn set_last_error(api: &GnoblinApi, ctx: GnoblinCtx, text: &str) {
    // SAFETY: `text` is a live string for the call.
    unsafe { (api.set_last_error)(ctx, text.as_ptr(), text.len()) };
}
