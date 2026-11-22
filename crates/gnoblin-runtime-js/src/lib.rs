//! `libgnoblin-runtime-js`: the JavaScript and TypeScript runtime of the shell host.
//!
//! The library exports one symbol, `gnoruntime_entry_v1`, which returns a static [`GnoblinRuntimeEntry`]
//! (the runtime entry of `gnoblin-core-abi`). The core's loader opens it, once, when a `.js`, `.mjs`, `.ts` or `.mts`
//! shell is selected, and calls `start` for each generation of the script. The engine is `gnoblin-js` (QuickJS-ng, the oxc type
//! stripper, the module loader with its confinement, the generation with its budgets, the bytecode cache); the shell over the
//! compositor's tree is its `JsRole`. Neither the engine nor the type stripper is linked into the `gnoblin` binary any more: a
//! compositor that never runs a script never maps them.
//!
//! - [`instance`] is what `start`, `turn` and `shutdown` do to the one shell a thread runs: boot it over the core's link, reload it
//!   as a new generation, run its frames, end it.
//! - [`link`] is the compositor's link as the core relays it (`tree_link_*` of the table), as a `TreeLink` the engine can drive.
//! - [`core`] is every call into the core, so the `unsafe` of this library is in two small modules.
//!
//! Every entry point is contained: a panic in the engine is caught here, reported to the core with the generation, and returned as
//! `E_PANIC`, because a panic that unwinds out of an `extern "C"` function aborts the compositor.
#![allow(unsafe_code)]

mod core_native;
pub mod embed;
mod instance;
mod link;
mod object_native;
mod std_native;
mod table;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Once;

use gnoblin_core_abi::{
    ABI_MAJOR, ABI_MINOR, E_ABI, E_ERR, E_OK, E_PANIC, GnoblinApi, GnoblinCtx, GnoblinRuntimeEntry, GnoblinStr,
};

/// What the entry says it runs: `.js`, `.mjs`, `.ts` and `.mts`.
const EXTENSIONS: &str = "js mjs ts mts";

/// The runtime's name: the middle of its library's file name.
const NAME: &str = "js";

const fn text(text: &'static str) -> GnoblinStr {
    GnoblinStr {
        ptr: text.as_ptr(),
        len: text.len(),
    }
}

static ENTRY: GnoblinRuntimeEntry = GnoblinRuntimeEntry {
    size: size_of::<GnoblinRuntimeEntry>() as u32,
    abi_major: ABI_MAJOR,
    abi_minor_built: ABI_MINOR,
    flags: 0,
    name: text(NAME),
    extensions: text(EXTENSIONS),
    version: text(instance::VERSION),
    start,
    shutdown,
    // A generation is staged and swapped in inside `start` (the engine evaluates beside the one that serves and commits only a
    // clean one), so there is nothing to activate and nothing to abandon.
    activate: None,
    abandon: None,
    turn: Some(turn),
};

/// The runtime's one export.
#[unsafe(no_mangle)]
pub extern "C" fn gnoruntime_entry_v1() -> *const GnoblinRuntimeEntry {
    &raw const ENTRY
}

/// The text of a caught panic.
fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned())
}

/// Forwards this library's log to the core's, once: the log is a per-library singleton, and without a subscriber here every line
/// the engine logs is dropped.
fn install_log(core: table::CoreTable) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Some(spec) = std::env::var("GNOBLIN_LOG")
            .ok()
            .and_then(|text| gnoblin_log::filter::Spec::parse(&text).ok())
        {
            gnoblin_log::filter::set_spec(spec);
        }
        gnoblin_log::forward::install(move |level, logged| {
            let level = match level {
                gnoblin_log::Level::ERROR => gnoblin_core_abi::LOG_ERROR,
                gnoblin_log::Level::WARN => gnoblin_core_abi::LOG_WARN,
                gnoblin_log::Level::INFO => gnoblin_core_abi::LOG_INFO,
                gnoblin_log::Level::DEBUG => gnoblin_core_abi::LOG_DEBUG,
                gnoblin_log::Level::TRACE => gnoblin_core_abi::LOG_TRACE,
            };
            let fields = logged.fields_line();
            let line = if fields.is_empty() {
                format!("{}: {}", logged.component, logged.message)
            } else {
                format!("{}: {} {fields}", logged.component, logged.message)
            };
            core.log(table::log_context(), level, &line);
        });
    });
}

/// Runs `body` so that a panic in it is reported to the core and becomes `E_PANIC` with the panic's text as the last error.
fn contained(core: Option<table::CoreTable>, ctx: GnoblinCtx, what: &str, body: impl FnOnce() -> i32) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(code) => code,
        Err(payload) => {
            let message = format!(
                "the JavaScript runtime panicked in {what}: {}",
                payload_text(payload.as_ref())
            );
            if let Some(core) = core {
                core.report_panic(ctx, &message);
                core.set_last_error(ctx, &message);
            }
            E_PANIC
        }
    }
}

/// Stages a generation of the script at `script`: the first boots the shell, a later one reloads it.
unsafe extern "C" fn start(api: *const GnoblinApi, ctx: GnoblinCtx, script: GnoblinStr) -> i32 {
    // SAFETY: the loader passes the core's static table.
    let Some(core) = (unsafe { table::CoreTable::from_table(api) }) else {
        return E_ABI;
    };
    if script.ptr.is_null() {
        core.set_last_error(ctx, "the loader gave the JavaScript runtime no script");
        return E_ERR;
    }
    // SAFETY: the loader passes the script's path as `len` bytes that live through the call.
    let path = PathBuf::from(
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(script.ptr, script.len) }).into_owned(),
    );
    contained(Some(core), ctx, "start", || {
        install_log(core);
        match instance::start(core, ctx, &path) {
            Ok(()) => E_OK,
            Err(message) => {
                core.set_last_error(ctx, &message);
                E_ERR
            }
        }
    })
}

/// Ends a generation.
unsafe extern "C" fn shutdown(ctx: GnoblinCtx) {
    let _ = catch_unwind(AssertUnwindSafe(|| instance::shutdown(ctx)));
}

/// One turn of the shell's loop.
unsafe extern "C" fn turn(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    contained(table::remembered(), ctx, "turn", || {
        match instance::turn(ctx, timeout_ms) {
            Ok(ran) => ran,
            Err(message) => {
                // The text is the core's last error on this thread, which the loop reads when the turn ends the shell. A runtime has
                // the table only through `start`, so the turn reports through the instance's own handle.
                instance::report_ended(ctx, &message);
                E_ERR
            }
        }
    })
}
