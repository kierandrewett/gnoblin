//! The shell's handle on the compositor: [`Gnoblin`], and the registration wrappers behind it.
//!
//! Each registration the core accepts holds a `user` pointer and a `drop_user` function. The wrappers here box the
//! closure, hand the core the box as `user`, and give it a monomorphised trampoline and drop function, so a closure is
//! called on the shell thread, dropped exactly once, and never called after its generation retires. The trampolines
//! also catch panics: a panic never unwinds out of an `extern "C"` function, and the panic hook installed at `init`
//! (`private::init`) has already reported it to the core's log by the time the trampoline returns `E_PANIC`.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinApi, GnoblinBuf, GnoblinCtx, GnoblinCtxInfo, GnoblinEvent, GnoblinMember, GnoblinReg,
    GnoblinStats,
};

use crate::error::{CoreError, Result, check};
use crate::sys;
use crate::value::{CatalogMember, Class, MemberKind, NodeId, SignalEvent};

/// Registrations this shell copy made, callbacks of it that ran, and `drop_user` calls that ran. The lifetime tests read
/// them through the entry descriptor's `stats` to prove a retired generation is never called again.
pub(crate) static REGISTERED: AtomicU64 = AtomicU64::new(0);
pub(crate) static CALLS: AtomicU64 = AtomicU64::new(0);
pub(crate) static DROPS: AtomicU64 = AtomicU64::new(0);

/// A log level for [`Gnoblin::log_at`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Something failed.
    CoreError,
    /// Something is wrong but the shell carries on.
    Warn,
    /// Ordinary progress.
    Info,
    /// Detail for a developer.
    Debug,
    /// Very fine detail.
    Trace,
}

impl LogLevel {
    fn raw(self) -> u32 {
        match self {
            LogLevel::CoreError => abi::LOG_ERROR,
            LogLevel::Warn => abi::LOG_WARN,
            LogLevel::Info => abi::LOG_INFO,
            LogLevel::Debug => abi::LOG_DEBUG,
            LogLevel::Trace => abi::LOG_TRACE,
        }
    }
}

/// What the core says about this generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleInfo {
    /// The generation number of this load.
    pub generation: u64,
    /// One of the `LANG_*` constants of the ABI.
    pub language: u32,
    /// How many times this shell was reloaded before this load.
    pub reload_count: u32,
    /// Whether the shell host runs in the isolated child process.
    pub isolate: bool,
    /// The path of the module as it was asked for.
    pub module_path: String,
    /// A directory the shell may keep state in across reloads, empty when there is none.
    pub state_dir: String,
}

/// The core's counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreStats {
    /// Live registrations across every generation.
    pub registrations: u64,
    /// Callbacks the core has run.
    pub callbacks: u64,
    /// Panics the core caught at the boundary.
    pub panics: u64,
    /// Generations retired so far.
    pub retired_generations: u64,
    /// Calls refused because the calling generation had retired.
    pub stale_calls: u64,
}

/// A registration the core holds for this generation: a callback, a timer, an input claim. It lasts until the generation
/// retires, or until [`unsubscribe`](Subscription::unsubscribe).
///
/// Dropping a `Subscription` does **not** unsubscribe: `g.on_client_created(...)` as a statement is the ordinary way to listen
/// for the life of the shell, and a guard that undid it silently would be a trap. [`scoped`](Subscription::scoped) is the guard
/// that does unsubscribe on drop.
#[derive(Clone)]
pub struct Subscription {
    g: Gnoblin,
    reg: GnoblinReg,
}

impl Subscription {
    /// Removes the registration. Its closure is dropped now, or when the callback that is running returns. Removing it twice,
    /// or after the generation retired, is an error.
    pub fn unsubscribe(self) -> Result<()> {
        // SAFETY: `remove` takes only the registration handle.
        check(unsafe { (self.g.api().remove)(self.reg) })
    }

    /// A guard that unsubscribes when it is dropped.
    pub fn scoped(self) -> ScopedSubscription {
        ScopedSubscription(Some(self))
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription").field("reg", &self.reg).finish()
    }
}

/// A [`Subscription`] that unsubscribes when dropped. Made by [`Subscription::scoped`].
#[must_use = "a scoped subscription unsubscribes when it is dropped"]
pub struct ScopedSubscription(Option<Subscription>);

impl ScopedSubscription {
    /// Gives up the guard and keeps the subscription for the life of the generation.
    pub fn keep(mut self) -> Subscription {
        self.0
            .take()
            .expect("a scoped subscription holds its subscription until it is kept or dropped")
    }
}

impl Drop for ScopedSubscription {
    fn drop(&mut self) {
        if let Some(subscription) = self.0.take() {
            // The generation may have retired already, in which case the core removed it and there is nothing to undo.
            let _ = subscription.unsubscribe();
        }
    }
}

/// What every clone of a [`Gnoblin`] shares: the table, the context, and what the SDK has looked up so far.
struct Shared {
    api: &'static GnoblinApi,
    ctx: GnoblinCtx,
    /// Whether this generation has attached to the tree.
    attached: Cell<bool>,
    classes: RefCell<HashMap<String, Class>>,
    members: RefCell<HashMap<(u32, u32, String), CatalogMember>>,
    /// The seat the shell's input belongs to, once found.
    seat: Cell<Option<NodeId>>,
}

/// The handle a shell uses to talk to the compositor. It belongs to one generation of the shell and to the shell thread:
/// it is `Clone` (cheap, and every clone shares what the SDK has looked up) but neither `Send` nor `Sync`. To act from
/// another thread, take a [`Poster`] and post work back.
///
/// Once the generation retires, every call through it (or through a [`Node`](crate::Node) or [`Window`](crate::Window) made
/// from it) fails with [`ErrorKind::Retired`](crate::ErrorKind::Retired): a handle kept past a reload is safe to hold and
/// useless to use.
#[derive(Clone)]
pub struct Gnoblin {
    shared: Rc<Shared>,
}

impl Gnoblin {
    pub(crate) fn new(api: &'static GnoblinApi, ctx: GnoblinCtx) -> Gnoblin {
        Gnoblin {
            shared: Rc::new(Shared {
                api,
                ctx,
                attached: Cell::new(false),
                classes: RefCell::new(HashMap::new()),
                members: RefCell::new(HashMap::new()),
                seat: Cell::new(None),
            }),
        }
    }

    /// The function table of the installed core.
    pub(crate) fn api(&self) -> &'static GnoblinApi {
        self.shared.api
    }

    /// The generation's context handle: the first argument of every table function.
    pub(crate) fn ctx(&self) -> GnoblinCtx {
        self.shared.ctx
    }

    #[cfg(feature = "next-minor")]
    /// Whether the installed core has the function whose field ends at `field_end` (see `gnoblin_core_abi::table_has`). A shell
    /// built against a newer minor than the core it runs on asks before it calls a function the older core lacks.
    pub(crate) fn has(&self, field_end: usize) -> bool {
        abi::table_has(self.shared.api.size, field_end)
    }

    /// Turns a table function's failure code into an [`CoreError`].
    pub(crate) fn fail(code: i32) -> CoreError {
        CoreError::from_code(code)
    }

    /// The ABI version of the installed core: `(major, minor, patch)`.
    pub fn core_abi(&self) -> (u32, u32, u32) {
        installed_abi_version()
    }

    /// A handle other threads may hold to log and to post work to the shell thread.
    pub fn poster(&self) -> Poster {
        Poster {
            api: self.shared.api,
            ctx: self.shared.ctx,
        }
    }

    /// Writes an informational line to the core's log, tagged with this generation.
    pub fn log(&self, message: &str) {
        self.log_at(LogLevel::Info, message);
    }

    /// Writes a line at `level` to the core's log.
    pub fn log_at(&self, level: LogLevel, message: &str) {
        self.poster().log_at(level, message);
    }

    /// What the core says about this generation: its number, module path and language.
    pub fn info(&self) -> Result<ModuleInfo> {
        let mut raw = GnoblinCtxInfo {
            size: size_of::<GnoblinCtxInfo>() as u32,
            language: 0,
            generation: 0,
            reload_count: 0,
            isolate: 0,
            module_path: abi::GnoblinStr::EMPTY,
            state_dir: abi::GnoblinStr::EMPTY,
        };
        // SAFETY: `ctx_info` fills the struct it is given up to the `size` the caller set; `raw` is a live local.
        check(unsafe { (self.api().ctx_info)(self.ctx(), &mut raw) })?;
        Ok(ModuleInfo {
            generation: raw.generation,
            language: raw.language,
            reload_count: raw.reload_count,
            isolate: raw.isolate != 0,
            module_path: copy_str(&raw.module_path),
            state_dir: copy_str(&raw.state_dir),
        })
    }

    /// The core's counters.
    pub fn stats(&self) -> Result<CoreStats> {
        let mut raw = GnoblinStats {
            size: size_of::<GnoblinStats>() as u32,
            reserved: 0,
            registrations: 0,
            callbacks: 0,
            panics: 0,
            retired_generations: 0,
            stale_calls: 0,
        };
        // SAFETY: `stats` fills the struct it is given up to the `size` the caller set; `raw` is a live local.
        check(unsafe { (self.api().stats)(self.ctx(), &mut raw) })?;
        Ok(CoreStats {
            registrations: raw.registrations,
            callbacks: raw.callbacks,
            panics: raw.panics,
            retired_generations: raw.retired_generations,
            stale_calls: raw.stale_calls,
        })
    }

    /// Asks the host to reload this shell. Takes effect after the current callback returns.
    pub fn request_reload(&self) -> Result<()> {
        // SAFETY: `request_reload` takes only the context handle.
        check(unsafe { (self.api().request_reload)(self.ctx()) })
    }

    /// Asks the host to exit the compositor with `code`.
    pub fn request_exit(&self, code: i32) -> Result<()> {
        // SAFETY: `request_exit` takes the context handle and an integer.
        check(unsafe { (self.api().request_exit)(self.ctx(), code) })
    }

    /// Runs `hook` once when this generation retires or the compositor exits, before the generation's registrations are
    /// removed.
    pub fn on_shutdown<F>(&self, hook: F) -> Result<Subscription>
    where
        F: FnOnce() + 'static,
    {
        let slot = Box::into_raw(Box::new(CallbackSlot { f: Some(hook) }));
        // SAFETY: `slot` is a leaked `Box<CallbackSlot<Option<F>>>`; the core hands it back to `shutdown_tramp::<F>` on the shell
        // thread and to `drop_slot::<Option<F>>` exactly once. On failure the core has not adopted it and it is
        // reclaimed below.
        let reg = unsafe {
            (self.api().on_shutdown)(
                self.ctx(),
                shutdown_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<CallbackSlot<Option<F>>>),
            )
        };
        self.adopt(reg, slot)
    }

    /// Looks a class up by name (`window`, or `namespace.object` for a class outside the tree). The answer is remembered.
    pub fn class(&self, name: &str) -> Result<Class> {
        if let Some(class) = self.shared.classes.borrow().get(name) {
            return Ok(*class);
        }
        // SAFETY: `catalog_class` reads `len` bytes at `name` for the duration of the call.
        let id = unsafe { (self.api().catalog_class)(self.ctx(), name.as_ptr(), name.len()) };
        if id == 0 {
            Err(CoreError::new(
                crate::ErrorKind::NoSuchMember,
                last_text_or(&format!("no class `{name}` in the catalog")),
            ))
        } else {
            let class = Class(id);
            self.shared.classes.borrow_mut().insert(name.to_owned(), class);
            Ok(class)
        }
    }

    /// Looks a member of `class` up by name. The answer is remembered.
    pub fn member(&self, class: Class, kind: MemberKind, name: &str) -> Result<CatalogMember> {
        let key = (class.0, kind.raw(), name.to_owned());
        if let Some(member) = self.shared.members.borrow().get(&key) {
            return Ok(member.clone());
        }
        let mut raw = blank_member();
        // SAFETY: `catalog_member` reads `len` bytes at `name` and fills the struct it is given up to its `size`; both
        // are live for the call.
        check(unsafe {
            (self.api().catalog_member)(self.ctx(), class.0, kind.raw(), name.as_ptr(), name.len(), &mut raw)
        })?;
        let member = CatalogMember::from_raw(&raw, kind);
        self.shared.members.borrow_mut().insert(key, member.clone());
        Ok(member)
    }

    /// Every member of one kind of `class`.
    pub fn members(&self, class: Class, kind: MemberKind) -> Result<Vec<CatalogMember>> {
        struct Collect {
            kind: MemberKind,
            members: Vec<CatalogMember>,
        }
        unsafe extern "C" fn each(user: *mut c_void, member: *const GnoblinMember) -> i32 {
            // SAFETY: `user` is the `Collect` below, which outlives the `catalog_each` call; `member` is valid for the
            // call and was checked for null.
            let (collect, member) = unsafe { (&mut *user.cast::<Collect>(), member.as_ref()) };
            if let Some(member) = member {
                collect.members.push(CatalogMember::from_raw(member, collect.kind));
            }
            abi::E_OK
        }
        let mut collect = Collect {
            kind,
            members: Vec::new(),
        };
        // SAFETY: the callback only runs during this call, and `collect` lives across it.
        check(unsafe { (self.api().catalog_each)(self.ctx(), class.0, kind.raw(), each, (&raw mut collect).cast()) })?;
        Ok(collect.members)
    }

    /// Calls `f` for every `signal` of `class`: of one node when `node` is given, of every node of the class when it is
    /// `None`. `signal` is a member id from [`Gnoblin::member`].
    pub fn on_signal<F>(&self, node: Option<NodeId>, class: Class, signal: u32, f: F) -> Result<Subscription>
    where
        F: FnMut(&SignalEvent<'_>) + 'static,
    {
        let slot = Box::into_raw(Box::new(CallbackSlot { f }));
        // SAFETY: as for `on_shutdown`: `slot` is a leaked box the core hands back to `event_tramp::<F>` and
        // `drop_slot::<CallbackSlot<F>>`.
        let reg = unsafe {
            (self.api().on_signal)(
                self.ctx(),
                node.unwrap_or(NodeId::NIL).0,
                class.0,
                signal,
                event_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<CallbackSlot<F>>),
            )
        };
        self.adopt(reg, slot)
    }

    /// [`Gnoblin::on_signal`] with the class and signal looked up by name.
    pub fn on_signal_named<F>(&self, class: &str, signal: &str, f: F) -> Result<Subscription>
    where
        F: FnMut(&SignalEvent<'_>) + 'static,
    {
        let class = self.class(class)?;
        let signal = self.member(class, MemberKind::Signal, signal)?;
        self.on_signal(None, class, signal.id, f)
    }

    /// Runs queued work and delivers queued events, waiting up to `timeout_ms` for the first. Only a shell that runs its
    /// own loop calls this; the default shell never does. Returns how many callbacks ran.
    pub fn dispatch(&self, timeout_ms: u32) -> Result<u32> {
        // SAFETY: `dispatch` takes the context handle and an integer.
        let rc = unsafe { (self.api().dispatch)(self.ctx(), timeout_ms) };
        if rc < abi::E_OK {
            Err(Self::fail(rc))
        } else {
            Ok(rc.unsigned_abs())
        }
    }

    /// Milliseconds since the installed core started. Present only in a core of ABI 1.1 or later; `None` on an older
    /// one, which is how a newer SDK degrades on an older core.
    #[cfg(feature = "next-minor")]
    pub fn uptime_ms(&self) -> Option<u64> {
        let end = core::mem::offset_of!(GnoblinApi, uptime_ms) + size_of::<usize>();
        if self.has(end) {
            // SAFETY: the table is long enough to hold `uptime_ms`, so the field was filled by the core.
            Some(unsafe { (self.api().uptime_ms)() })
        } else {
            None
        }
    }

    /// Keeps `bytes` under `key` for the next generation of this shell (a reload), with a `version` of the caller's choosing. The
    /// store is dropped when the compositor restarts and holds at most 16 MiB.
    pub fn keep_state(&self, key: &str, bytes: &[u8], version: u32) -> Result<()> {
        // SAFETY: `state_put` reads `key_len` and `len` bytes at the pointers for the call.
        check(unsafe {
            (self.api().state_put)(
                self.ctx(),
                key.as_ptr(),
                key.len(),
                bytes.as_ptr(),
                bytes.len(),
                version,
            )
        })
    }

    /// What an earlier generation kept under `key`, with the version it kept it at; `None` when nothing is kept.
    pub fn kept_state(&self, key: &str) -> Result<Option<(Vec<u8>, u32)>> {
        let mut buf = GnoblinBuf {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let mut version = 0;
        // SAFETY: `state_get` reads `key_len` bytes at `key` and fills `buf` and `version`, live locals.
        let rc = unsafe { (self.api().state_get)(self.ctx(), key.as_ptr(), key.len(), &mut buf, &mut version) };
        if rc == abi::E_NO_SUCH_MEMBER {
            return Ok(None);
        }
        check(rc)?;
        let bytes = if buf.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: the core lent `len` bytes at `ptr`.
            unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
        };
        // SAFETY: `buf` is a buffer the core lent, freed once.
        let _ = unsafe { (self.api().buf_free)(&mut buf) };
        Ok(Some((bytes, version)))
    }

    // ---- the tree -------------------------------------------------------------------------------------------------------

    /// Attaches this generation to the compositor's tree, once. Every call that reads or writes the tree attaches first, so a
    /// shell never needs to; it is public for a shell that wants to see the failure at a place of its own choosing.
    pub fn attach(&self) -> Result<()> {
        if self.shared.attached.get() {
            return Ok(());
        }
        // SAFETY: `tree_attach` takes the context handle and a mode.
        check(unsafe { (self.api().tree_attach)(self.ctx(), abi::ATTACH_HOLDER) })?;
        self.shared.attached.set(true);
        Ok(())
    }

    /// The seat the shell's input is on: the first seat of the tree, remembered.
    pub(crate) fn main_seat(&self) -> Result<crate::tree::Node> {
        if let Some(seat) = self.shared.seat.get() {
            return Ok(self.node(seat));
        }
        let seat_class = self.class("seat")?;
        let root = self.root()?;
        for child in root.children()? {
            if child.class()? == seat_class {
                self.shared.seat.set(Some(child.id()));
                return Ok(child);
            }
        }
        Err(CoreError::new(
            crate::ErrorKind::NoSuchNode,
            "the tree has no seat to take input from",
        ))
    }

    /// Turns the result of a registering function that was given no closure into a [`Subscription`].
    pub(crate) fn adopt_bare(&self, reg: GnoblinReg) -> Result<Subscription> {
        if reg == 0 {
            Err(CoreError::new(
                crate::ErrorKind::Failed,
                last_text_or("the core refused the registration"),
            ))
        } else {
            Ok(Subscription { g: self.clone(), reg })
        }
    }

    /// Turns a registering function's result into a [`Subscription`], reclaiming the box when the core refused it.
    pub(crate) fn adopt<T>(&self, reg: GnoblinReg, slot: *mut T) -> Result<Subscription> {
        if reg == 0 {
            // SAFETY: the core returned zero, so it has not adopted `slot` and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            Err(CoreError::new(
                crate::ErrorKind::Failed,
                last_text_or("the core refused the registration"),
            ))
        } else {
            REGISTERED.fetch_add(1, Ordering::Relaxed);
            Ok(Subscription { g: self.clone(), reg })
        }
    }
}

/// A handle that is `Send` and `Sync`: log from any thread, and post work to run on the shell thread.
#[derive(Clone, Copy)]
pub struct Poster {
    api: &'static GnoblinApi,
    ctx: GnoblinCtx,
}

impl Poster {
    /// Writes a line at `level` to the core's log.
    pub fn log_at(&self, level: LogLevel, message: &str) {
        // SAFETY: `log` reads `len` bytes at `msg` for the call and is callable from any thread.
        let _ = unsafe { (self.api.log)(self.ctx, level.raw(), message.as_ptr(), message.len()) };
    }

    /// Runs `f` on the shell thread at its next dispatch. If the generation retires first, `f` is dropped unrun.
    pub fn post<F>(&self, f: F) -> Result<()>
    where
        F: FnOnce(&Gnoblin) + Send + 'static,
    {
        let slot = Box::into_raw(Box::new(PostSlot {
            f: Some(f),
            api: self.api,
            ctx: self.ctx,
        }));
        // SAFETY: `slot` is a leaked box. The core either runs `post_tramp::<F>`, which takes it back, or calls
        // `drop_post::<F>` when the context retires first; if `post` fails the core has not adopted it and it is
        // reclaimed below.
        let rc = unsafe { (self.api.post)(self.ctx, post_tramp::<F>, slot.cast(), Some(drop_post::<F>)) };
        if rc < abi::E_OK {
            // SAFETY: the core refused the work, so nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            return Err(CoreError::from_code(rc));
        }
        Ok(())
    }
}

/// A closure behind a `user` pointer.
pub(crate) struct CallbackSlot<F> {
    pub(crate) f: F,
}

/// Work posted to the shell thread, with what is needed to rebuild a `Gnoblin` there.
struct PostSlot<F> {
    f: Option<F>,
    api: &'static GnoblinApi,
    ctx: GnoblinCtx,
}

unsafe extern "C" fn event_tramp<F: FnMut(&SignalEvent<'_>) + 'static>(
    user: *mut c_void,
    event: *const GnoblinEvent,
) -> i32 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<CallbackSlot<F>>` leaked by `on_signal`. The core passes it back unchanged, on the shell thread,
    // and does not free it before `drop_slot` runs, so it is live and not aliased during this call.
    let slot = unsafe { &mut *user.cast::<CallbackSlot<F>>() };
    // SAFETY: the core passes a pointer that is valid until this callback returns, or null.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return abi::E_ERR;
    };
    let view = SignalEvent::new(event);
    match catch_unwind(AssertUnwindSafe(|| (slot.f)(&view))) {
        Ok(()) => abi::E_OK,
        Err(_) => abi::E_PANIC,
    }
}

unsafe extern "C" fn shutdown_tramp<F: FnOnce() + 'static>(user: *mut c_void) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: as for `event_tramp`: the leaked `Box<CallbackSlot<Option<F>>>` of `on_shutdown`.
    let slot = unsafe { &mut *user.cast::<CallbackSlot<Option<F>>>() };
    if let Some(hook) = slot.f.take() {
        // A panic in a shutdown hook has been reported by the panic hook; nothing is left to do with it.
        let _ = catch_unwind(AssertUnwindSafe(hook));
    }
}

/// Frees the box behind a `user` pointer. The core calls it exactly once per registration.
pub(crate) unsafe extern "C" fn drop_slot<T>(user: *mut c_void) {
    DROPS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<T>` leaked at registration and the core calls this exactly once, so the box is
    // reclaimed exactly once.
    drop(unsafe { Box::from_raw(user.cast::<T>()) });
}

unsafe extern "C" fn post_tramp<F: FnOnce(&Gnoblin)>(user: *mut c_void) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<PostSlot<F>>` leaked by `post`; the core calls either this or `drop_post`, never both,
    // so the box is taken back exactly once.
    let mut slot = unsafe { Box::from_raw(user.cast::<PostSlot<F>>()) };
    if let Some(f) = slot.f.take() {
        let g = Gnoblin::new(slot.api, slot.ctx);
        let _ = catch_unwind(AssertUnwindSafe(|| f(&g)));
    }
}

unsafe extern "C" fn drop_post<F>(user: *mut c_void) {
    // SAFETY: the core calls this instead of `post_tramp` when the context retires with the work still queued, so the
    // box leaked by `post` is still owned by the core and is reclaimed here once.
    drop(unsafe { Box::from_raw(user.cast::<PostSlot<F>>()) });
}

fn blank_member() -> GnoblinMember {
    GnoblinMember {
        size: size_of::<GnoblinMember>() as u32,
        id: 0,
        kind: 0,
        value_kind: 0,
        flags: 0,
        arg_count: 0,
        arg_kinds: core::ptr::null(),
    }
}

/// A copy of a string the core lent for the call.
pub(crate) fn copy_str(raw: &abi::GnoblinStr) -> String {
    if raw.ptr.is_null() || raw.len == 0 {
        return String::new();
    }
    // SAFETY: the core documents the string as `len` bytes at `ptr`, valid for the call.
    let bytes = unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// The core's last-error text, or `fallback` when it has none.
pub(crate) fn last_text_or(fallback: &str) -> String {
    let text = crate::error::last_error_text();
    if text.is_empty() { fallback.to_owned() } else { text }
}

/// The installed core's ABI version, from `gnoblin_abi_version`.
pub(crate) fn installed_abi_version() -> (u32, u32, u32) {
    let (mut major, mut minor, mut patch) = (0, 0, 0);
    // SAFETY: `gnoblin_abi_version` writes three `u32`s through the pointers it is given; they are live locals.
    unsafe { sys::gnoblin_abi_version(&mut major, &mut minor, &mut patch) };
    (major, minor, patch)
}
