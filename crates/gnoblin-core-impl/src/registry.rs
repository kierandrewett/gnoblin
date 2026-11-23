//! The ABI face of the shell host: contexts, registrations, the event queue and the dispatch loop, as C handles.
//!
//! The machinery is `gnoblin-shell-host`'s ([`GenerationHost`]): generations, the registrations each owns, queued events and the
//! dispatch loop, with their invariants. This module is what is specific to
//! the C ABI: a handle the shell holds is an encoded generation or registration id, a callback is an `extern "C"` function
//! with a `user` pointer and an optional `drop_user`, an error is a code and a last-error text, and an event is a
//! `GnoblinEvent` built from a published [`SignalDelivery`].
//!
//! `dispatch` is also where the shell thread's own work happens, once per turn: the tree link is read and its change batches
//! applied (so a callback reads the tree as the whole batch left it), the events that brings are published to the mailbox with
//! the timers that fell due and the programs that ended, the host delivers them, and what the callbacks wrote goes out as one
//! transaction.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gnoblin_core_abi::{
    self as abi, GnoblinCtx, GnoblinCtxInfo, GnoblinDropFn, GnoblinEvent, GnoblinEventFn, GnoblinPostFn, GnoblinReg,
    GnoblinShutdownFn, GnoblinStats, GnoblinStr,
};
pub use gnoblin_shell_host::generation::GenerationState;
use gnoblin_shell_host::generation::{CallbackOutcome, GenerationHost, Listener, Release, RemoveError, Work};
use gnoblin_shell_host::handle::{KIND_CTX, KIND_REG, decode, encode};

pub use crate::events::{ShellEvent, SignalDelivery};
use crate::guard::{PANICS, set_code_error, set_error};
use crate::session::{self, Lifecycle};
pub use crate::values::OwnedValue;
use crate::values::RawScope;
use crate::{budget, catalog, log, wake};

/// The host's lifecycle hooks: they keep the session's programs and claims in step with the generations.
static LIFECYCLE: Lifecycle = Lifecycle;

/// The one host of the process: every generation, registration and queued event.
pub(crate) static HOST: GenerationHost<ShellEvent> = GenerationHost::with_observer(&LIFECYCLE);

/// How long one wait for the compositor's link may last, when nothing else (a post, a timer) is due sooner. The link has no
/// descriptor the host sleeps on together with its mailbox, so a quiet turn is a short sleep; a post or a published signal wakes
/// the mailbox at once.
const LINK_POLL: Duration = Duration::from_millis(4);

/// `window.request`: the name a signal registration goes by in the budget's log lines.
fn signal_label(class: u32, signal: u32) -> String {
    let class_name = catalog::class_label(class).unwrap_or_else(|| format!("class {class}"));
    let signal_name = catalog::signal_name(class, signal).unwrap_or_else(|| format!("signal {signal}"));
    format!("{class_name}.{signal_name}")
}

// ---- contexts ---------------------------------------------------------------------------------------------------------

/// Makes a context for a module that is about to run `init`, staged, owned by the calling thread (which becomes the shell
/// thread). The generation number is the context's sequence number. The loader does this for a real module; the tests do it
/// for a context of their own, and the std host for the library it loads into the compositor's process.
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

/// The language a context's module is written in (`abi::LANG_*`), `None` for a handle the core did not issue.
pub(crate) fn language_of(ctx: GnoblinCtx) -> Option<u32> {
    HOST.info(decode(KIND_CTX, ctx)?).map(|info| info.language)
}

/// The path of the module a generation index was loaded from, for a registration that asks whether two generations are the
/// same shell.
pub(crate) fn module_path_of(index: u64) -> Option<String> {
    HOST.info(index).map(|info| info.module_path.clone())
}

/// Staged to active: from now its registrations receive events.
pub(crate) fn activate(ctx: GnoblinCtx) -> Result<(), String> {
    let id = decode(KIND_CTX, ctx).ok_or("not a context handle")?;
    HOST.activate(id)
}

/// Runs every shutdown hook the context registered, in registration order. A hook runs once.
pub(crate) fn run_shutdown_hooks(ctx: GnoblinCtx) {
    if let Some(id) = decode(KIND_CTX, ctx) {
        HOST.run_shutdown_hooks(id);
    }
}

/// Retires a context: removes every registration it owns (running each `drop_user`), drops its queued posts, and marks it
/// so every later call from it is refused. Returns how many registrations were removed.
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

/// Whether generation `id` is the active one: the one whose claims and timers are live now.
pub(crate) fn is_active(id: u64) -> bool {
    HOST.state_of(id) == Some(GenerationState::Active)
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
pub(crate) fn release_of(user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> Option<Release> {
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
    // The payload as the ABI struct: values point into `event` and into `scope`, which outlive the call.
    let mut scope = RawScope::default();
    let values: Vec<_> = event.values.iter().map(|value| scope.raw(value)).collect();
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
    outcome_of(rc)
}

/// What a callback's return code says about how it ended: the SDK's trampoline returns `E_PANIC` after it caught a panic.
pub(crate) fn outcome_of(rc: i32) -> CallbackOutcome {
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
            let label: Arc<str> = Arc::from(format!("on_signal {}", signal_label(class, signal)));
            HOST.register_listener(
                owner,
                Listener {
                    matches: Box::new(move |event: &ShellEvent| {
                        matches!(event, ShellEvent::Signal(event)
                            if class == event.class && signal == event.signal && (node == 0 || node == event.node))
                    }),
                    call: Arc::new(move |event: &ShellEvent| match event {
                        ShellEvent::Signal(delivery) => {
                            let _watch = budget::watch(owner, &label);
                            call_signal(cb, user, delivery)
                        }
                        _ => CallbackOutcome::Handled,
                    }),
                },
                release,
            )
        }
        Registering::Shutdown { cb } => {
            let label: Arc<str> = Arc::from("on_shutdown");
            let hook: Work = Box::new(move || {
                let _watch = budget::watch(owner, &label);
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

/// A registration that is not for a signal: the events it takes, what it does with one, and what to clean up when it ends.
pub(crate) struct Subscription {
    /// Whether an event is for this registration. Called with the host's lock held: keep it to a comparison.
    pub matches: Box<dyn Fn(&ShellEvent) -> bool + Send + Sync>,
    /// Calls the shell's callback for an event this registration matched.
    pub call: Arc<dyn Fn(&ShellEvent) -> CallbackOutcome + Send + Sync>,
    /// Runs when the registration ends, before the shell's `drop_user`: the core's own state that went with it (a timer, a claim).
    pub cleanup: Option<Release>,
}

/// Registers `subscription` for the generation of `ctx`, adopting `user` and `drop_user`. Returns the registration's id, or the
/// ABI code with the last error recorded (`user` is then not adopted and `cleanup` was not run: the caller undoes what it did).
pub(crate) fn subscribe(
    ctx: GnoblinCtx,
    what: &str,
    subscription: Subscription,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> Result<u64, i32> {
    let owner = check_ctx(ctx, what, true)?;
    let shell_release = release_of(user, drop_user);
    let Subscription { matches, call, cleanup } = subscription;
    let release: Option<Release> = match (cleanup, shell_release) {
        (None, release) => release,
        (Some(cleanup), None) => Some(cleanup),
        (Some(cleanup), Some(release)) => Some(Box::new(move || {
            cleanup();
            release();
        })),
    };
    // Every callback of the shell is timed against the budget, whatever event it is for.
    let label: Arc<str> = Arc::from(what);
    let inner = call;
    let call: Arc<dyn Fn(&ShellEvent) -> CallbackOutcome + Send + Sync> = Arc::new(move |event: &ShellEvent| {
        let _watch = budget::watch(owner, &label);
        inner(event)
    });
    match HOST.register_listener(owner, Listener { matches, call }, release) {
        Ok(id) => Ok(id),
        Err((error, _unadopted)) => Err(error.report(what)),
    }
}

/// The owner of a context, for a caller that registers against it.
pub(crate) fn owner_of(ctx: GnoblinCtx, what: &str) -> Result<u64, i32> {
    check_ctx(ctx, what, true)
}

/// Removes a registration by its id (a one-shot callback removing itself, or a cancelled pending call). The shell's `drop_user`
/// runs now, or after the callback that is running returns.
pub(crate) fn remove_id(id: u64) {
    let _ = HOST.remove(id);
}

/// Removes the registration with id `id` for `remove` and `cancel`: the ABI code for a registration that is gone or called from
/// another thread.
pub(crate) fn remove_pending(id: u64) -> Result<(), i32> {
    match HOST.remove(id) {
        Ok(()) => Ok(()),
        Err(RemoveError::Gone) => {
            set_code_error(abi::E_STALE, "the registration was already removed");
            Err(abi::E_STALE)
        }
        Err(RemoveError::WrongThread) => {
            set_code_error(
                abi::E_THREAD,
                "remove must be called on the shell thread of the registration's generation",
            );
            Err(abi::E_THREAD)
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
    match remove_pending(id) {
        Ok(()) => abi::E_OK,
        Err(code) => code,
    }
}

/// `post`: queues `f(user)` for the shell thread of `ctx`.
pub(crate) fn post(ctx: GnoblinCtx, f: GnoblinPostFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> i32 {
    let owner = match check_ctx(ctx, "post", false) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    let user = user as usize;
    let label: Arc<str> = Arc::from("post");
    let work: Work = Box::new(move || {
        let _watch = budget::watch(owner, &label);
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
        Ok(()) => {
            wake::signal();
            abi::E_OK
        }
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
    // A panic in `init` is reported here before the load fails, and the loader shows this text as the reason: the shell's own words,
    // with where it happened.
    set_error(message);
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
    HOST.publish(ShellEvent::Signal(delivery));
    wake::signal();
}

/// Queues any event for the shell, waking its dispatch loop: the verbs' answers, which a thread of the owner connection brings. Any
/// thread.
pub(crate) fn publish_event(event: ShellEvent) {
    HOST.publish(event);
    wake::signal();
}

/// How many events are waiting.
pub(crate) fn queued_events() -> usize {
    HOST.queued_events()
}

/// The shell thread's own turn of work: reads the tree link, applies what it sent, and publishes what that brings with the
/// timers that fell due and the programs that ended.
///
/// # Errors
///
/// The tree link failed or a change batch was missed. The tree is dropped (the shell must attach again).
fn pump_into_host(ctx: GnoblinCtx) -> Result<(), String> {
    let events = session::with(session::Session::pump)?;
    crate::model::pumped(ctx, &events);
    for event in events {
        HOST.publish(event);
    }
    let fired = session::with(|session| session.take_due(is_active));
    for timer in fired {
        HOST.publish(ShellEvent::Timer(timer));
    }
    Ok(())
}

/// `dispatch`: runs the posts of `ctx`, then delivers every queued event, waiting up to `timeout_ms` for the first. Returns the
/// number of callbacks run, or a negative error code.
///
/// A turn is: pump the link, deliver, then send what the callbacks wrote as one transaction. It returns after the first turn that
/// ran a callback, or when the time is up.
pub(crate) fn dispatch(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    let owner = match check_ctx(ctx, "dispatch", true) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    let mut called = 0usize;
    let mut failure: Option<String> = None;
    loop {
        // Before the mailbox is taken, so a post that lands while this turn runs signals again and is not lost.
        wake::drain();
        if let Err(text) = pump_into_host(ctx) {
            log::error!(generation = owner, "the tree link failed: {text}");
            failure = Some(text);
        }
        match HOST.dispatch(owner, 0) {
            Ok(n) => called += n,
            Err(error) => return error.report("dispatch"),
        }
        if let Err(text) = session::with(session::Session::flush) {
            log::error!(generation = owner, "the tree link failed: {text}");
            failure = Some(text);
        }
        let now = Instant::now();
        if called > 0 || failure.is_some() || now >= deadline {
            break;
        }
        // Nothing ran. Sleep on the mailbox (a post or a published signal wakes it) until the first of the deadline, the next timer
        // and the next look at the link.
        let mut wait = deadline - now;
        if session::with(|session| session.attached()) {
            wait = wait.min(LINK_POLL);
        }
        if let Some(timer) = session::with(|session| session.next_due_in(is_active)) {
            wait = wait.min(timer);
        }
        // Round up to a whole millisecond: a wait of zero would spin.
        let wait_ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX).saturating_add(1);
        match HOST.dispatch(owner, wait_ms) {
            Ok(n) => called += n,
            Err(error) => return error.report("dispatch"),
        }
        if called > 0 {
            if let Err(text) = session::with(session::Session::flush) {
                log::error!(generation = owner, "the tree link failed: {text}");
                failure = Some(text);
            }
            break;
        }
    }
    sync_wake();
    if let Some(text) = failure {
        set_code_error(abi::E_ERR, format!("dispatch: the tree link failed: {text}"));
        if called == 0 {
            return abi::E_ERR;
        }
    }
    i32::try_from(called).unwrap_or(i32::MAX)
}

/// Brings the wake descriptor in step with the shell thread's session: the next timer of an active generation and the tree link's
/// descriptor. Shell thread. Free until a shell asks for the descriptor.
pub(crate) fn sync_wake() {
    session::with(|session| wake::sync(session.next_due_in(is_active), session.link_fd()));
}

/// `wake_fd`: a duplicate of the wake descriptor, for the shell to poll.
pub(crate) fn wake_fd(ctx: GnoblinCtx) -> i32 {
    let owner = match check_ctx(ctx, "wake_fd", false) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    let set = match wake::get() {
        Ok(set) => set,
        Err(text) => {
            set_code_error(abi::E_ERR, format!("wake_fd: {text}"));
            return abi::E_ERR;
        }
    };
    // From the shell thread the set is brought in step before the shell first polls it; from another thread it is as the shell
    // thread last left it, and the shell thread's own calls keep it so.
    if HOST.check(owner, true).is_ok() {
        sync_wake();
        // Something may have been queued before the set existed.
        if HOST.queued_events() > 0 {
            wake::signal();
        }
    }
    match set.duplicate() {
        Ok(fd) => std::os::fd::IntoRawFd::into_raw_fd(fd),
        Err(error) => {
            set_code_error(abi::E_ERR, format!("wake_fd: cannot duplicate the descriptor: {error}"));
            abi::E_ERR
        }
    }
}

/// A snapshot of the host's size, for the tests.
#[cfg(test)]
pub(crate) fn live_registrations() -> usize {
    HOST.live_registrations()
}
