//! Contexts, registrations, the event queue and the dispatch loop.
//!
//! One [`Registrar`] per process holds every generation context and every registration a shell made, plus the queues the
//! compositor side publishes into. The invariants this module keeps (`design/modular-framework.md` sections 5.4, 5.5):
//!
//! - Each registration is owned by the generation context that made it, and removed with it. `drop_user` runs exactly
//!   once per adopted registration, on the shell thread, while the generation's code is still mapped.
//! - A retired context's calls return `E_RETIRED` and do nothing. The context record is kept so the answer stays
//!   `E_RETIRED` rather than becoming "no such handle".
//! - A registration of a context that is not active (staged for a reload) receives nothing.
//! - Shell code never runs with the lock held. A callback may remove registrations, including its own; removing the
//!   running one defers its `drop_user` until the callback returns.
//! - An event is delivered to a registration only if the registration still exists, is not marked dead, and its context is
//!   active at the moment of the call: removed-before-call is checked, not assumed.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::collections::{BTreeMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use gnoblin_core_abi::{
    self as abi, GnoblinCtx, GnoblinCtxInfo, GnoblinDropFn, GnoblinEvent, GnoblinEventFn, GnoblinPostFn, GnoblinReg,
    GnoblinShutdownFn, GnoblinStats, GnoblinStr, GnoblinValue,
};

use crate::guard::{PANICS, payload_text, set_code_error, set_error};
use crate::handle::{KIND_CTX, KIND_REG, decode, encode};
use crate::{catalog, log};

/// The most events the mailbox holds before the oldest is dropped. A shell that never dispatches must not grow the
/// compositor's memory without bound.
const MAILBOX_CAP: usize = 65_536;

/// Callbacks run, across every generation.
static CALLBACKS: AtomicU64 = AtomicU64::new(0);
/// Generations retired.
static RETIRED: AtomicU64 = AtomicU64::new(0);
/// Calls refused because the calling generation had retired.
static STALE_CALLS: AtomicU64 = AtomicU64::new(0);

/// Where a generation is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationState {
    /// Loaded and initialising, or waiting for its reload to commit: its registrations receive nothing yet.
    Staged,
    /// Running: its registrations receive events.
    Active,
    /// Gone: every call it makes is refused.
    Retired,
}

/// One load of one shell module.
struct Ctx {
    state: GenerationState,
    /// The thread that made the context; every shell-thread function checks the caller against it.
    thread: ThreadId,
    module_path: String,
    state_dir: String,
    language: u32,
    reload_count: u32,
    isolate: bool,
}

enum Listener {
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

/// One callback the shell registered.
struct Reg {
    owner: u64,
    target: Listener,
    user: usize,
    drop_user: Option<GnoblinDropFn>,
    /// Removed while its callback was running: it is deleted, and its `drop_user` runs, when the callback returns.
    dead: bool,
}

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

/// Work posted to the shell thread.
struct Post {
    owner: u64,
    f: GnoblinPostFn,
    user: usize,
    drop_user: Option<GnoblinDropFn>,
}

struct Registrar {
    next_ctx: u64,
    next_reg: u64,
    ctxs: BTreeMap<u64, Ctx>,
    regs: BTreeMap<u64, Reg>,
    events: VecDeque<SignalDelivery>,
    posts: VecDeque<Post>,
    /// The registration whose callback is running now.
    in_flight: Option<u64>,
}

static HOST: Mutex<Registrar> = Mutex::new(Registrar {
    next_ctx: 1,
    next_reg: 1,
    ctxs: BTreeMap::new(),
    regs: BTreeMap::new(),
    events: VecDeque::new(),
    posts: VecDeque::new(),
    in_flight: None,
});
static WAKE: Condvar = Condvar::new();

/// The host, recovering the lock if a panic poisoned it: the state is consistent between statements.
fn host() -> MutexGuard<'static, Registrar> {
    HOST.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Calls a function the shell gave us, containing a panic that unwinds out of it.
fn contained<T>(on_panic: T, what: &str, call: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(value) => value,
        Err(payload) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            log::error!(
                what = what,
                "{what} unwound across the ABI: {}",
                payload_text(payload.as_ref())
            );
            on_panic
        }
    }
}

/// Runs a `drop_user` function, if there is one.
fn run_drop(drop_user: Option<GnoblinDropFn>, user: usize) {
    if let Some(drop_user) = drop_user {
        // SAFETY: `drop_user` and `user` came from the same registration call; the shell guarantees the function accepts
        // that pointer, and this is the one call the registration is owed.
        contained((), "drop_user", || unsafe { drop_user(user as *mut c_void) });
    }
}

// ---- contexts ---------------------------------------------------------------------------------------------------------

/// Makes a context for a module that is about to run `init`, staged, owned by the calling thread (which becomes the
/// shell thread). The generation number is the context's sequence number.
pub(crate) fn new_context(module_path: &str, language: u32, reload_count: u32, isolate: bool) -> GnoblinCtx {
    let mut host = host();
    let index = host.next_ctx;
    host.next_ctx += 1;
    host.ctxs.insert(
        index,
        Ctx {
            state: GenerationState::Staged,
            thread: thread::current().id(),
            module_path: module_path.to_owned(),
            state_dir: String::new(),
            language,
            reload_count,
            isolate,
        },
    );
    encode(KIND_CTX, index)
}

/// The generation number of a context handle.
pub(crate) fn generation_of(ctx: GnoblinCtx) -> Option<u64> {
    decode(KIND_CTX, ctx)
}

/// The state of a context, `None` for a handle the core did not issue.
pub(crate) fn state_of(ctx: GnoblinCtx) -> Option<GenerationState> {
    let index = decode(KIND_CTX, ctx)?;
    host().ctxs.get(&index).map(|ctx| ctx.state)
}

/// Staged to active: from now its registrations receive events.
pub(crate) fn activate(ctx: GnoblinCtx) -> Result<(), String> {
    let index = decode(KIND_CTX, ctx).ok_or("not a context handle")?;
    let mut host = host();
    match host.ctxs.get_mut(&index) {
        Some(record) if record.state == GenerationState::Staged => {
            record.state = GenerationState::Active;
            Ok(())
        }
        Some(record) => Err(format!("generation {index} is {:?}, not staged", record.state)),
        None => Err("no such context".to_owned()),
    }
}

/// Runs every shutdown hook the context registered, in registration order. A hook runs once.
pub(crate) fn run_shutdown_hooks(ctx: GnoblinCtx) {
    let Some(index) = decode(KIND_CTX, ctx) else { return };
    loop {
        let next = {
            let mut host = host();
            let id = host
                .regs
                .iter()
                .find(|(_, reg)| reg.owner == index && !reg.dead && matches!(reg.target, Listener::Shutdown { .. }))
                .map(|(id, _)| *id);
            id.and_then(|id| host.regs.remove(&id))
        };
        let Some(reg) = next else { break };
        if let Listener::Shutdown { cb } = reg.target {
            // SAFETY: `cb` and `user` came from one `on_shutdown` call, and the registration was removed from the table
            // just above, so this is the only call it gets.
            contained((), "shutdown hook", || unsafe { cb(reg.user as *mut c_void) });
            CALLBACKS.fetch_add(1, Ordering::Relaxed);
        }
        run_drop(reg.drop_user, reg.user);
    }
}

/// Retires a context: removes every registration it owns (running each `drop_user`), drops its queued posts, and marks it
/// so every later call from it is refused. Returns how many registrations were removed.
///
/// A registration whose callback is running (retire called from inside it) is marked dead and removed when the callback
/// returns.
pub(crate) fn finish_retire(ctx: GnoblinCtx) -> usize {
    let Some(index) = decode(KIND_CTX, ctx) else { return 0 };
    let (regs, posts) = {
        let mut host = host();
        let Some(record) = host.ctxs.get_mut(&index) else {
            return 0;
        };
        if record.state == GenerationState::Retired {
            return 0;
        }
        record.state = GenerationState::Retired;
        RETIRED.fetch_add(1, Ordering::Relaxed);
        let in_flight = host.in_flight;
        let owned: Vec<u64> = host
            .regs
            .iter()
            .filter(|(_, reg)| reg.owner == index)
            .map(|(id, _)| *id)
            .collect();
        let mut removed = Vec::new();
        for id in owned {
            if Some(id) == in_flight {
                if let Some(reg) = host.regs.get_mut(&id) {
                    reg.dead = true;
                }
            } else if let Some(reg) = host.regs.remove(&id) {
                removed.push(reg);
            }
        }
        let (mine, rest): (VecDeque<Post>, VecDeque<Post>) = std::mem::take(&mut host.posts)
            .into_iter()
            .partition(|post| post.owner == index);
        host.posts = rest;
        (removed, mine)
    };
    let count = regs.len();
    for reg in regs {
        run_drop(reg.drop_user, reg.user);
    }
    for post in posts {
        run_drop(post.drop_user, post.user);
    }
    count
}

/// Checks that `handle` names a context the caller may use for `what`, returning its index. Records the last error and
/// counts a stale call when it does not.
pub(crate) fn check_ctx(handle: GnoblinCtx, what: &str, shell_thread: bool) -> Result<u64, i32> {
    let Some(index) = decode(KIND_CTX, handle) else {
        set_code_error(abi::E_STALE, format!("{what}: {handle:#x} is not a context handle"));
        return Err(abi::E_STALE);
    };
    let host = host();
    let Some(record) = host.ctxs.get(&index) else {
        drop(host);
        set_code_error(abi::E_STALE, format!("{what}: no generation {index}"));
        return Err(abi::E_STALE);
    };
    if record.state == GenerationState::Retired {
        drop(host);
        STALE_CALLS.fetch_add(1, Ordering::Relaxed);
        set_code_error(
            abi::E_RETIRED,
            format!("generation {index} has been retired; {what} did nothing"),
        );
        return Err(abi::E_RETIRED);
    }
    if shell_thread && record.thread != thread::current().id() {
        drop(host);
        set_code_error(
            abi::E_THREAD,
            format!("{what} must be called on the shell thread of generation {index}"),
        );
        return Err(abi::E_THREAD);
    }
    Ok(index)
}

// ---- table functions, over the host ---------------------------------------------------------------------------------

/// `ctx_info`.
pub(crate) fn ctx_info(ctx: GnoblinCtx) -> Result<GnoblinCtxInfo, i32> {
    let index = check_ctx(ctx, "ctx_info", true)?;
    let host = host();
    let record = host.ctxs.get(&index).ok_or(abi::E_STALE)?;
    Ok(GnoblinCtxInfo {
        size: size_of::<GnoblinCtxInfo>() as u32,
        language: record.language,
        generation: index,
        reload_count: record.reload_count,
        isolate: u32::from(record.isolate),
        // The strings live in the context record, which is never removed and never mutated, so the pointers stay valid.
        module_path: GnoblinStr::borrowed(&record.module_path),
        state_dir: GnoblinStr::borrowed(&record.state_dir),
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
    let target = match registration {
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
            Listener::Signal {
                node,
                class,
                signal,
                cb,
            }
        }
        Registering::Shutdown { cb } => Listener::Shutdown { cb },
    };
    let mut host = host();
    let id = host.next_reg;
    host.next_reg += 1;
    host.regs.insert(
        id,
        Reg {
            owner,
            target,
            user: user as usize,
            drop_user,
            dead: false,
        },
    );
    encode(KIND_REG, id)
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
    let removed = {
        let mut host = host();
        let Some(reg) = host.regs.get(&id) else {
            drop(host);
            set_code_error(abi::E_STALE, "remove: the registration was already removed");
            return abi::E_STALE;
        };
        let owner_thread = host.ctxs.get(&reg.owner).map(|ctx| ctx.thread);
        if owner_thread != Some(thread::current().id()) {
            drop(host);
            set_code_error(
                abi::E_THREAD,
                "remove must be called on the shell thread of the registration's generation",
            );
            return abi::E_THREAD;
        }
        if host.in_flight == Some(id) {
            if let Some(reg) = host.regs.get_mut(&id) {
                reg.dead = true;
            }
            None
        } else {
            host.regs.remove(&id)
        }
    };
    if let Some(reg) = removed {
        run_drop(reg.drop_user, reg.user);
    }
    abi::E_OK
}

/// `post`: queues `f(user)` for the shell thread of `ctx`.
pub(crate) fn post(ctx: GnoblinCtx, f: GnoblinPostFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> i32 {
    let owner = match check_ctx(ctx, "post", false) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    host().posts.push_back(Post {
        owner,
        f,
        user: user as usize,
        drop_user,
    });
    WAKE.notify_all();
    abi::E_OK
}

/// `stats`.
pub(crate) fn stats(ctx: GnoblinCtx) -> Result<GnoblinStats, i32> {
    check_ctx(ctx, "stats", false)?;
    let registrations = host().regs.values().filter(|reg| !reg.dead).count() as u64;
    Ok(GnoblinStats {
        size: size_of::<GnoblinStats>() as u32,
        reserved: 0,
        registrations,
        callbacks: CALLBACKS.load(Ordering::Relaxed),
        panics: PANICS.load(Ordering::Relaxed),
        retired_generations: RETIRED.load(Ordering::Relaxed),
        stale_calls: STALE_CALLS.load(Ordering::Relaxed),
    })
}

/// `report_panic`: logs a panic a shell caught, tagged with its generation. A report from a retired generation is still
/// logged (marked as such): a panic is diagnostic output, not an action on the compositor.
pub(crate) fn report_panic(ctx: GnoblinCtx, message: &str) -> i32 {
    let Some(index) = decode(KIND_CTX, ctx) else {
        log::error!("PANIC in a shell with an unknown context: {message}");
        return abi::E_STALE;
    };
    let retired = host()
        .ctxs
        .get(&index)
        .is_some_and(|record| record.state == GenerationState::Retired);
    if retired {
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
    {
        let mut host = host();
        if host.events.len() >= MAILBOX_CAP {
            host.events.pop_front();
            log::warn!("the shell mailbox is full: dropped the oldest event");
        }
        host.events.push_back(delivery);
    }
    WAKE.notify_all();
}

/// How many events are waiting.
pub(crate) fn queued_events() -> usize {
    host().events.len()
}

/// `dispatch`: runs the posts of `ctx`, then delivers every queued event, waiting up to `timeout_ms` for the first.
/// Returns the number of callbacks run, or a negative error code.
pub(crate) fn dispatch(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    let owner = match check_ctx(ctx, "dispatch", true) {
        Ok(owner) => owner,
        Err(code) => return code,
    };
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    let (posts, events) = {
        let mut host = host();
        loop {
            let active = host
                .ctxs
                .get(&owner)
                .is_some_and(|record| record.state == GenerationState::Active);
            let has_posts = host.posts.iter().any(|post| post.owner == owner);
            if has_posts || (active && !host.events.is_empty()) {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return 0;
            }
            host = WAKE.wait_timeout(host, left).unwrap_or_else(PoisonError::into_inner).0;
        }
        let active = host
            .ctxs
            .get(&owner)
            .is_some_and(|record| record.state == GenerationState::Active);
        let (mine, rest): (VecDeque<Post>, VecDeque<Post>) = std::mem::take(&mut host.posts)
            .into_iter()
            .partition(|post| post.owner == owner);
        host.posts = rest;
        let events: Vec<SignalDelivery> = if active {
            host.events.drain(..).collect()
        } else {
            Vec::new()
        };
        (mine, events)
    };
    let mut called = 0;
    for post in posts {
        // SAFETY: `f` and `user` came from one `post` call; the post was taken off the queue just above, so this is the
        // only call it gets, and it consumes `user` (the shell's trampoline reclaims its box).
        contained((), "post", || unsafe { (post.f)(post.user as *mut c_void) });
        CALLBACKS.fetch_add(1, Ordering::Relaxed);
        called += 1;
    }
    for event in &events {
        called += deliver(event);
    }
    called
}

/// Delivers one event to every matching registration of every active context, in registration order.
fn deliver(event: &SignalDelivery) -> i32 {
    let matching: Vec<u64> = {
        let host = host();
        host.regs
            .iter()
            .filter(|(_, reg)| !reg.dead && matches_event(reg, event) && owner_is_active(&host, reg.owner))
            .map(|(id, _)| *id)
            .collect()
    };
    if matching.is_empty() {
        return 0;
    }
    // The payload as the ABI struct: values point into `event`, which outlives every callback below.
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
    let mut called = 0;
    for id in matching {
        let (cb, user) = {
            let mut host = host();
            let Some(reg) = host.regs.get(&id) else { continue };
            if reg.dead || !owner_is_active(&host, reg.owner) {
                continue;
            }
            let Listener::Signal { cb, .. } = reg.target else {
                continue;
            };
            let user = reg.user;
            host.in_flight = Some(id);
            (cb, user)
        };
        // SAFETY: `cb` and `user` came from one `on_signal` call. `raw` and the `values` it points at are live locals,
        // valid for the whole call, as the ABI promises the callback.
        let rc = contained(abi::E_PANIC, "callback", || unsafe {
            cb(user as *mut c_void, &raw const raw)
        });
        CALLBACKS.fetch_add(1, Ordering::Relaxed);
        called += 1;
        if rc == abi::E_PANIC {
            PANICS.fetch_add(1, Ordering::Relaxed);
            log::error!("callback of registration {id} panicked; contained, the next event is still delivered");
        }
        let finished = {
            let mut host = host();
            host.in_flight = None;
            match host.regs.get(&id) {
                Some(reg) if reg.dead => host.regs.remove(&id),
                _ => None,
            }
        };
        if let Some(reg) = finished {
            run_drop(reg.drop_user, reg.user);
        }
    }
    called
}

fn owner_is_active(host: &Registrar, owner: u64) -> bool {
    host.ctxs
        .get(&owner)
        .is_some_and(|record| record.state == GenerationState::Active)
}

fn matches_event(reg: &Reg, event: &SignalDelivery) -> bool {
    match reg.target {
        Listener::Signal {
            node, class, signal, ..
        } => class == event.class && signal == event.signal && (node == 0 || node == event.node),
        Listener::Shutdown { .. } => false,
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
    host().regs.len()
}
