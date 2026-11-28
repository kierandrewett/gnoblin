//! `libgnoblin-runtime-python`: the Python runtime of the shell host.
//!
//! The library exports one symbol, `gnoruntime_entry_v1`, which returns a static [`GnoblinRuntimeEntry`]
//! (the runtime entry of `gnoblin-core-abi`). The core's loader opens it, once, when a `.py` shell is selected, and
//! calls `start` for each generation of the script. The interpreter is the system CPython, embedded through PyO3; the shell is an
//! ordinary script that imports the package `gnoblin`, which this library serves from itself (nothing of it is read from disk) and
//! which calls the built-in module `_gnoblin_core` of this library. That module is a thin layer over the core SDK (`gnoblin-core`):
//! every call goes to the installed core with the handle the loader made for the generation, and no compositor logic is repeated here.
//!
//! - [`interp`] brings the interpreter up, once, and puts the finder and the log routing in place.
//! - [`finder`] serves the package: the modules generated from the catalog and embedded, and the ones this crate builds.
//! - [`descriptors`] (`gnoblin._base`), [`singleton`] (`gnoblin.api`), [`subscription`] and [`values`] (`gnoblin.events`), [`errors`]
//!   (`gnoblin.errors`) are the hand-written half of the package, in Rust; [`methods`] binds a native
//!   function as a method.
//! - [`logging`] routes Python's own reporting (an uncaught exception, a warning, a `logging` record) into the core's log.
//! - [`generation`] is the shell this thread runs: the generations the loader has made, which one the module's functions act on, and
//!   the handles a script holds (subscriptions, programs).
//! - [`load`] stages, commits and ends a generation through [`loader`], which imports the script and the modules it imports into a fresh
//!   namespace and puts the previous generation's back when the import fails.
//! - [`watch`] sees an edit of a file a generation imported and asks the host for a reload.
//! - [`module`] is `_gnoblin_core`: [`tree`], [`events`], [`input`] and [`process`] hold its functions, [`convert`] the values that
//!   cross, [`callback`] how a script's callback runs, and [`errors`] the exceptions.
//!
//! A generation is staged by `start` and served by `activate`, as for a native shell: the script's registrations belong to the
//! generation that evaluated it, receive no event until the loader commits it, and are removed when it retires. That is the whole
//! of "no listener twice": the core removes the old generation's registrations, and nothing here keeps a second list.
//!
//! Every entry point is contained: a panic is caught here, reported to the core, and returned as `E_PANIC`, because a panic that
//! unwinds out of an `extern "C"` function aborts the compositor.
#![allow(unsafe_code)]
// The version script (`exports.map`) makes PyO3's `PyInit__gnoblin_core` local on purpose; lld notes that it overrides rustc's own list.
#![allow(linker_messages)]

mod callback;
mod convert;
mod descriptors;
mod errors;
mod events;
mod finder;
mod generation;
mod input;
mod interp;
mod load;
mod loader;
mod logging;
mod methods;
mod module;
mod object_native;
mod process;
mod singleton;
mod std_native;
mod subscription;
mod table;
mod tree;
mod values;
mod verbs;
mod watch;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::time::Duration;

use gnoblin_core_abi::{
    ABI_MAJOR, ABI_MINOR, E_ABI, E_ERR, E_OK, E_PANIC, GnoblinApi, GnoblinCtx, GnoblinRuntimeEntry, GnoblinStr,
};

/// What the entry says it runs: `.py`.
const EXTENSIONS: &str = "py";

/// The runtime's name: the middle of its library's file name.
const NAME: &str = "python";

/// The runtime's version string, as the entry reports it.
const VERSION: &str = concat!("gnoblin-runtime-python ", env!("CARGO_PKG_VERSION"));

/// The longest one turn waits inside the core's dispatch: what has no descriptor to wait on (the watcher) is looked at this often.
const TURN_SLICE: Duration = Duration::from_millis(50);

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
    version: text(VERSION),
    start,
    shutdown,
    activate: Some(activate),
    abandon: Some(abandon),
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

/// Runs `body` so that a panic in it is logged, left as the last error and returned as `E_PANIC`.
fn contained(ctx: GnoblinCtx, what: &str, body: impl FnOnce() -> i32) -> i32 {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(code) => code,
        Err(payload) => {
            let message = format!(
                "the Python runtime panicked in {what}: {}",
                payload_text(payload.as_ref())
            );
            generation::log_error(&message);
            generation::set_last_error(ctx, &message);
            E_PANIC
        }
    }
}

/// Stages a generation of the script at `script`: the first boots the interpreter and the shell, a later one is a reload.
unsafe extern "C" fn start(api: *const GnoblinApi, ctx: GnoblinCtx, script: GnoblinStr) -> i32 {
    // SAFETY: the loader passes the core's static table.
    let Some(api) = (unsafe { table::from_table(api) }) else {
        return E_ABI;
    };
    if script.ptr.is_null() {
        table::set_last_error(api, ctx, "the loader gave the Python runtime no script");
        return E_ERR;
    }
    // SAFETY: the loader passes the script's path as `len` bytes that live through the call.
    let path = PathBuf::from(
        String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(script.ptr, script.len) }).into_owned(),
    );
    contained(ctx, "start", || match load::stage(api, ctx, &path) {
        Ok(()) => E_OK,
        Err(message) => {
            table::set_last_error(api, ctx, &message);
            E_ERR
        }
    })
}

/// Makes the staged generation the serving one.
unsafe extern "C" fn activate(ctx: GnoblinCtx) -> i32 {
    contained(ctx, "activate", || match load::activate(ctx) {
        Ok(()) => E_OK,
        Err(message) => {
            generation::set_last_error(ctx, &message);
            E_ERR
        }
    })
}

/// Throws a staged generation away unserved.
unsafe extern "C" fn abandon(ctx: GnoblinCtx) {
    let _ = catch_unwind(AssertUnwindSafe(|| load::abandon(ctx)));
}

/// Ends a generation.
unsafe extern "C" fn shutdown(ctx: GnoblinCtx) {
    let _ = catch_unwind(AssertUnwindSafe(|| load::end(ctx)));
}

/// One turn of the shell's loop: runs what the core has queued for the serving generation, waiting up to `timeout_ms` for the first,
/// with the interpreter's lock released so a thread of the script may run meanwhile; then looks at the files the generation
/// imported.
unsafe extern "C" fn turn(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    contained(ctx, "turn", || {
        let wait = Duration::from_millis(u64::from(timeout_ms)).min(TURN_SLICE);
        let ran = generation::dispatch(ctx, wait);
        watch::poll(ctx);
        match ran {
            Ok(count) => i32::try_from(count).unwrap_or(i32::MAX),
            Err(message) => {
                generation::set_last_error(ctx, &message);
                generation::log_error(&message);
                E_ERR
            }
        }
    })
}
