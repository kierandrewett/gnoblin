//! The ABI face of the shell host: contexts, registrations, the event queue and the dispatch loop, as C handles.
//!
//! The machinery is `gnoblin-shell-host`'s ([`GenerationHost`]): generations, the registrations each owns, queued events and the
//! dispatch loop, with their invariants (`design/modular-framework.md` sections 5.4, 5.5). This module is what is specific to
//! the C ABI: a handle the shell holds is an encoded generation or registration id, a callback is an `extern "C"` function
//! with a `user` pointer and an optional `drop_user`, an error is a code and a last-error text, and an event is a
//! `GnoblinEvent` built from a published [`SignalDelivery`].
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use gnoblin_core_abi::{
    self as abi, GnoblinCtx, GnoblinCtxInfo, GnoblinDropFn, GnoblinEvent, GnoblinEventFn, GnoblinPostFn, GnoblinReg,
    GnoblinShutdownFn, GnoblinStats, GnoblinStr, GnoblinValue,
};
pub use gnoblin_shell_host::generation::GenerationState;
use gnoblin_shell_host::generation::{CallbackOutcome, GenerationHost, Listener, Release, RemoveError, Work};
use gnoblin_shell_host::handle::{KIND_CTX, KIND_REG, decode, encode};

use crate::guard::{PANICS, set_code_error, set_error};
use crate::{catalog, log};

/// A value the compositor side publishes with an event. Owned, so it outlives the queue.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    Int(i64),
    /// An unsigned integer.
    Uint(u64),
    /// A double.
    Double(f64),
    /// Text.
    Text(String),
    /// A node id.
    Node(u64),
}

/// A signal to deliver to the shell, by class and signal id.
#[derive(Clone, Debug, PartialEq)]
pub struct SignalDelivery {
    /// The catalog class id.
    pub class: u32,
    /// The signal id within the class.
    pub signal: u32,
    /// The node it came from, or zero.
    pub node: u64,
    /// The payload fields.
    pub values: Vec<OwnedValue>,
}

/// The one host of the process: every generation, registration and queued event.
pub(crate) static HOST: GenerationHost<SignalDelivery> = GenerationHost::new();

// ---- contexts ---------------------------------------------------------------------------------------------------------

/// Makes a context for a module that is about to run `init`, staged, owned by the calling thread (which becomes the shell
/// thread). The generation number is the context's sequence number. The loader does this for a real module; the tests do it
/// for a context of their own.
#[cfg(test)]
pub(crate) fn new_context(module_path: &str, language: u32, reload_count: u32, isolate: bool) -> GnoblinCtx {
    use gnoblin_shell_host::generation::GenerationInfo;

    let id = HOST.new_generation(GenerationInfo {
        module_path: module_path.to_owned(),
        state_dir: String::new(),
        language,
        reload_count,
        isolate,
    });
    encode(KIND_CTX, id)
}

/// The generation number of a context handle.
#[cfg(test)]
pub(crate) fn generation_of(ctx: GnoblinCtx) -> Option<u64> {
    decode(KIND_CTX, ctx)
}

/// The state of a context, `None` for a handle the core did not issue.
pub(crate) fn state_of(ctx: GnoblinCtx) -> Option<GenerationState> {
    HOST.state_of(decode(KIND_CTX, ctx)?)
}

/// Staged to active: from now its registrations receive events.
#[cfg(test)]
pub(crate) fn activate(ctx: GnoblinCtx) -> Result<(), String> {
    let id = decode(KIND_CTX, ctx).ok_or("not a context handle")?;
    HOST.activate(id)
}

/// Runs every shutdown hook the context registered, in registration order. A hook runs once.
#[cfg(test)]
pub(crate) fn run_shutdown_hooks(ctx: GnoblinCtx) {
    if let Some(id) = decode(KIND_CTX, ctx) {
        HOST.run_shutdown_hooks(id);
    }
}

/// Retires a context: removes every registration it owns (running each `drop_user`), drops its queued posts, and marks it
/// so every later call from it is refused. Returns how many registrations were removed.
#[cfg(test)]
pub(crate) fn finish_retire(ctx: GnoblinCtx) -> usize {
    decode(KIND_CTX, ctx).map_or(0, |id| HOST.finish_retire(id))
}

/// Checks that `handle` names a context the caller may use for `what`, returning its index. Records the last error and
/// counts a stale call when it does not.
pub(crate) fn check_ctx(handle: GnoblinCtx, what: &str, shell_thread: bool) -> Result<u64, i32> {
    let Some(index) = decode(KIND_CTX, handle) else {
        set_code_error(abi::E_STALE, format!("{what}: {handle:#x} is not a context handle"));
        return Err(abi::E_STALE);
    };
    HOST.check(index, shell_thread).map_err(|error| error.report(what))?;
    Ok(index)
}

// ---- table functions, over the host ---------------------------------------------------------------------------------

/// `ctx_info`.
pub(crate) fn ctx_info(ctx: GnoblinCtx) -> Result<GnoblinCtxInfo, i32> {
    let index = check_ctx(ctx, "ctx_info", true)?;
    let info = HOST.info(index).ok_or(abi::E_STALE)?;
    Ok(GnoblinCtxInfo {
        size: size_of::<GnoblinCtxInfo>() as u32,
        language: info.language,
        generation: index,
        reload_count: info.reload_count,
        isolate: u32::from(info.isolate),
        // The host keeps the record, which is never removed and never mutated, so the pointers stay valid after `info`
        // (our handle on it) is dropped.
        module_path: GnoblinStr::borrowed(&info.module_path),
        state_dir: GnoblinStr::borrowed(&info.state_dir),
    })
}

/// What a registering function asks for.
pub(crate) enum Registering {
    Signal {
        node: u64,
        class: u32,
        signal: u32,
        cb: GnoblinEventFn,
    },
    Shutdown {
        cb: GnoblinShutdownFn,
    },
}

/// The release of a registration: the shell's `drop_user`, called once with the `user` pointer it came with.
fn release_of(user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> Option<Release> {
    let user = user as usize;
    drop_user.map(|drop_user| {
        let release: Release = Box::new(move || {
            // SAFETY: `drop_user` and `user` came from the same registration call; the shell guarantees the function
            // accepts that pointer, and this is the one call the registration is owed.
            unsafe { drop_user(user as *mut c_void) };
        });
        release
    })
}

/// Runs a shell's signal callback for one event, building the ABI's event struct from the published delivery.
fn call_signal(cb: GnoblinEventFn, user: usize, event: &SignalDelivery) -> CallbackOutcome {
    // The payload as the ABI struct: values point into `event`, which outlives the call.
    let values: Vec<GnoblinValue> = event.values.iter().map(raw_value).collect();
    let raw = GnoblinEvent {
        size: size_of::<GnoblinEvent>() as u32,
        class: event.class,
        signal: event.signal,
        reserved: 0,
        node: event.node,
        values: values.as_ptr(),
        n_values: values.len(),
    };
    // SAFETY: `cb` and `user` came from one `on_signal` call. `raw` and the `values` it points at are live locals, valid
    // for the whole call, as the ABI promises the callback.
    let rc = unsafe { cb(user as *mut c_void, &raw const raw) };
    if rc == abi::E_PANIC {
        CallbackOutcome::Panicked
    } else {
        CallbackOutcome::Handled
    }
}

/// `on_signal` and `on_shutdown`: adopts `user`, returning a registration handle, or zero (with `user` not adopted).
pub(crate) fn register(
    ctx: GnoblinCtx,
    what: &str,
    registration: Registering,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    let Ok(owner) = check_ctx(ctx, what, true) else {
        return 0;
    };
    let release = release_of(user, drop_user);
    let user = user as usize;
    let outcome = match registration {
        Registering::Signal {
            node,
            class,
            signal,
            cb,
        } => {
            if let Err(message) = catalog::check_signal(class, signal) {
                set_code_error(abi::E_NO_SUCH_MEMBER, format!("{what}: {message}"));
                return 0;
            }
            HOST.register_listener(
                owner,
                Listener {
                    matches: Box::new(move |event: &SignalDelivery| {
                        class == event.class && signal == event.signal && (node == 0 || node == event.node)
                    }),
                    call: Arc::new(move |event: &SignalDelivery| call_signal(cb, user, event)),
                },
                release,
            )
        }
        Registering::Shutdown { cb } => {
            let hook: Work = Box::new(move || {
                // SAFETY: `cb` and `user` came from one `on_shutdown` call, and the host removes the registration before
                // it runs the hook, so this is the only call it gets.
                unsafe { cb(user as *mut c_void) };
            });
            HOST.register_shutdown(owner, hook, release)
        }
    };
    match outcome {
        Ok(id) => encode(KIND_REG, id),
        Err((error, _unadopted)) => {
            error.report(what);
            0
        }
    }
}

/// `remove`.
pub(crate) fn remove(handle: GnoblinReg) -> i32 {
    let Some(id) = decode(KIND_REG, handle) else {
        set_code_error(
            abi::E_STALE,
            format!("remove: {handle:#x} is not a registration handle"),
        );
        return abi::E_STALE;
    };
    match HOST.remove(id) {
        Ok(()) => abi::E_OK,
        Err(RemoveError::Gone) => {
            set_code_error(abi::E_STALE, "remove: the registration was already removed");
            abi::E_STALE
        }
        Err(RemoveError::WrongThread) => {
            set_code_error(
                abi::E_THREAD,
                "remove must be called on the shell thread of the registration's generation",
            );
            abi::E_THREAD
        }
    }
}

/// `post`: queues `f(user)` for the shell thread of `ctx`.
pub(crate) fn post(ctx: GnoblinCtx, f: GnoblinPostFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> i32 {
    let owner = match check_ctx(ctx, "post", false) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    let user = user as usize;
    let work: Work = Box::new(move || {
        // SAFETY: `f` and `user` came from one `post` call; the host takes the post off the queue before it runs it, so
        // this is the only call it gets, and it consumes `user` (the shell's trampoline reclaims its box).
        unsafe { f(user as *mut c_void) };
    });
    let discard: Release = Box::new(move || {
        if let Some(drop_user) = drop_user {
            // SAFETY: the post never ran (its generation retired first), so `drop_user` is owed one call with `user`.
            unsafe { drop_user(user as *mut c_void) };
        }
    });
    match HOST.post(owner, work, discard) {
        Ok(()) => abi::E_OK,
        Err((error, _work, _discard)) => error.report("post"),
    }
}

/// `stats`.
pub(crate) fn stats(ctx: GnoblinCtx) -> Result<GnoblinStats, i32> {
    check_ctx(ctx, "stats", false)?;
    let stats = HOST.stats();
    Ok(GnoblinStats {
        size: size_of::<GnoblinStats>() as u32,
        reserved: 0,
        registrations: stats.registrations,
        callbacks: stats.callbacks,
        panics: PANICS.load(Ordering::Relaxed),
        retired_generations: stats.retired_generations,
        stale_calls: stats.stale_calls,
    })
}

/// `report_panic`: logs a panic a shell caught, tagged with its generation. A report from a retired generation is still
/// logged (marked as such): a panic is diagnostic output, not an action on the compositor.
pub(crate) fn report_panic(ctx: GnoblinCtx, message: &str) -> i32 {
    let Some(index) = decode(KIND_CTX, ctx) else {
        log::error!("PANIC in a shell with an unknown context: {message}");
        return abi::E_STALE;
    };
    if HOST.state_of(index) == Some(GenerationState::Retired) {
        log::error!(
            generation = index,
            "PANIC in retired shell generation {index}: {message}"
        );
    } else {
        log::error!(generation = index, "PANIC in shell generation {index}: {message}");
    }
    abi::E_OK
}

/// `log`: one line from a shell, tagged with its generation.
pub(crate) fn log_line(ctx: GnoblinCtx, level: u32, message: &str) -> i32 {
    let index = match check_ctx(ctx, "log", false) {
        Ok(index) => index,
        Err(code) => return code,
    };
    match level {
        abi::LOG_ERROR => log::error!(generation = index, "{message}"),
        abi::LOG_WARN => log::warn!(generation = index, "{message}"),
        abi::LOG_DEBUG => log::debug!(generation = index, "{message}"),
        abi::LOG_TRACE => log::trace!(generation = index, "{message}"),
        _ => log::info!(generation = index, "{message}"),
    }
    abi::E_OK
}

/// `set_last_error`: sets this thread's error text on behalf of a shell whose `init` is returning a failure.
pub(crate) fn set_last_error(ctx: GnoblinCtx, message: &str) -> i32 {
    if decode(KIND_CTX, ctx).is_none() {
        return abi::E_STALE;
    }
    set_error(message);
    abi::E_OK
}

// ---- the event queue and the dispatch loop --------------------------------------------------------------------------

/// Queues a signal for the shell, waking its dispatch loop. The compositor side calls this from any thread.
pub(crate) fn publish(delivery: SignalDelivery) {
    HOST.publish(delivery);
}

/// How many events are waiting.
pub(crate) fn queued_events() -> usize {
    HOST.queued_events()
}

/// `dispatch`: runs the posts of `ctx`, then delivers every queued event, waiting up to `timeout_ms` for the first.
/// Returns the number of callbacks run, or a negative error code.
pub(crate) fn dispatch(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    let owner = match check_ctx(ctx, "dispatch", true) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    match HOST.dispatch(owner, timeout_ms) {
        Ok(called) => i32::try_from(called).unwrap_or(i32::MAX),
        Err(error) => error.report("dispatch"),
    }
}

/// An owned value as the ABI struct, borrowing its text from the owner.
fn raw_value(value: &OwnedValue) -> GnoblinValue {
    let mut raw = GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind: 0,
        flags: 0,
        reserved: 0,
        bits: 0,
        ptr: core::ptr::null(),
        len: 0,
    };
    match value {
        OwnedValue::Bool(value) => {
            raw.kind = abi::VALUE_BOOL;
            raw.bits = u64::from(*value);
        }
        OwnedValue::Int(value) => {
            raw.kind = abi::VALUE_INT;
            // Two's complement, as the ABI defines the bit pattern.
            raw.bits = *value as u64;
        }
        OwnedValue::Uint(value) => {
            raw.kind = abi::VALUE_UINT;
            raw.bits = *value;
        }
        OwnedValue::Double(value) => {
            raw.kind = abi::VALUE_DOUBLE;
            raw.bits = value.to_bits();
        }
        OwnedValue::Node(value) => {
            raw.kind = abi::VALUE_NODE;
            raw.bits = *value;
        }
        OwnedValue::Text(text) => {
            raw.kind = abi::VALUE_TEXT;
            raw.ptr = text.as_ptr().cast();
            raw.len = text.len();
        }
    }
    raw
}

/// A snapshot of the host's size, for the tests.
#[cfg(test)]
pub(crate) fn live_registrations() -> usize {
    HOST.live_registrations()
}
