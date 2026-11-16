//! The shell's handle on the compositor: [`Gnoblin`], and the registration wrappers behind it.
//!
//! Each registration the core accepts holds a `user` pointer and a `drop_user` function. The wrappers here box the
//! closure, hand the core the box as `user`, and give it a monomorphised trampoline and drop function, so a closure is
//! called on the shell thread, dropped exactly once, and never called after its generation retires. The trampolines
//! also catch panics: a panic never unwinds out of an `extern "C"` function, and the panic hook installed at `init`
//! (`private::init`) has already reported it to the core's log by the time the trampoline returns `E_PANIC`.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinApi, GnoblinCtx, GnoblinCtxInfo, GnoblinEvent, GnoblinMember, GnoblinReg, GnoblinStats,
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

/// A registration the core holds for this generation: a callback, a shutdown hook. It is removed by
/// [`Gnoblin::remove`] or, when the generation retires, by the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationHandle(GnoblinReg);

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

/// The handle a shell uses to talk to the compositor. It belongs to one generation of the shell and to the shell thread:
/// it is `Clone` (cheap) but neither `Send` nor `Sync`. To act from another thread, take a [`Poster`] and post work back.
#[derive(Clone)]
pub struct Gnoblin {
    api: &'static GnoblinApi,
    ctx: GnoblinCtx,
    /// A raw pointer makes the type neither `Send` nor `Sync`.
    _shell_thread: PhantomData<*const ()>,
}

impl Gnoblin {
    pub(crate) fn new(api: &'static GnoblinApi, ctx: GnoblinCtx) -> Gnoblin {
        Gnoblin {
            api,
            ctx,
            _shell_thread: PhantomData,
        }
    }

    /// Turns a table function's failure code into an [`CoreError`].
    fn fail(code: i32) -> CoreError {
        CoreError::from_code(code)
    }

    /// The ABI version of the installed core: `(major, minor, patch)`.
    pub fn core_abi(&self) -> (u32, u32, u32) {
        installed_abi_version()
    }

    /// A handle other threads may hold to log and to post work to the shell thread.
    pub fn poster(&self) -> Poster {
        Poster {
            api: self.api,
            ctx: self.ctx,
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
        check(unsafe { (self.api.ctx_info)(self.ctx, &mut raw) })?;
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
        check(unsafe { (self.api.stats)(self.ctx, &mut raw) })?;
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
        check(unsafe { (self.api.request_reload)(self.ctx) })
    }

    /// Asks the host to exit the compositor with `code`.
    pub fn request_exit(&self, code: i32) -> Result<()> {
        // SAFETY: `request_exit` takes the context handle and an integer.
        check(unsafe { (self.api.request_exit)(self.ctx, code) })
    }

    /// Runs `hook` once when this generation retires or the compositor exits, before the generation's registrations are
    /// removed.
    pub fn on_shutdown<F>(&self, hook: F) -> Result<RegistrationHandle>
    where
        F: FnOnce() + 'static,
    {
        let slot = Box::into_raw(Box::new(CallbackSlot { f: Some(hook) }));
        // SAFETY: `slot` is a leaked `Box<CallbackSlot<Option<F>>>`; the core hands it back to `shutdown_tramp::<F>` on the shell
        // thread and to `drop_slot::<Option<F>>` exactly once. On failure the core has not adopted it and it is
        // reclaimed below.
        let reg =
            unsafe { (self.api.on_shutdown)(self.ctx, shutdown_tramp::<F>, slot.cast(), Some(drop_slot::<Option<F>>)) };
        self.adopt(reg, slot)
    }

    /// Looks a class up by name (`window`, or `namespace.object` for a class outside the tree).
    pub fn class(&self, name: &str) -> Result<Class> {
        // SAFETY: `catalog_class` reads `len` bytes at `name` for the duration of the call.
        let id = unsafe { (self.api.catalog_class)(self.ctx, name.as_ptr(), name.len()) };
        if id == 0 {
            Err(CoreError::new(
                crate::ErrorKind::NoSuchMember,
                last_text_or(&format!("no class `{name}` in the catalog")),
            ))
        } else {
            Ok(Class(id))
        }
    }

    /// Looks a member of `class` up by name.
    pub fn member(&self, class: Class, kind: MemberKind, name: &str) -> Result<CatalogMember> {
        let mut raw = blank_member();
        // SAFETY: `catalog_member` reads `len` bytes at `name` and fills the struct it is given up to its `size`; both
        // are live for the call.
        check(unsafe {
            (self.api.catalog_member)(self.ctx, class.0, kind.raw(), name.as_ptr(), name.len(), &mut raw)
        })?;
        Ok(CatalogMember::from_raw(&raw, kind))
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
        check(unsafe { (self.api.catalog_each)(self.ctx, class.0, kind.raw(), each, (&raw mut collect).cast()) })?;
        Ok(collect.members)
    }

    /// Calls `f` for every `signal` of `class`: of one node when `node` is given, of every node of the class when it is
    /// `None`. `signal` is a member id from [`Gnoblin::member`].
    pub fn on_signal<F>(&self, node: Option<NodeId>, class: Class, signal: u32, f: F) -> Result<RegistrationHandle>
    where
        F: FnMut(&SignalEvent<'_>) + 'static,
    {
        let slot = Box::into_raw(Box::new(CallbackSlot { f }));
        // SAFETY: as for `on_shutdown`: `slot` is a leaked box the core hands back to `event_tramp::<F>` and
        // `drop_slot::<F>`.
        let reg = unsafe {
            (self.api.on_signal)(
                self.ctx,
                node.unwrap_or(NodeId::NIL).0,
                class.0,
                signal,
                event_tramp::<F>,
                slot.cast(),
                Some(drop_slot::<F>),
            )
        };
        self.adopt(reg, slot)
    }

    /// [`Gnoblin::on_signal`] with the class and signal looked up by name.
    pub fn on_signal_named<F>(&self, class: &str, signal: &str, f: F) -> Result<RegistrationHandle>
    where
        F: FnMut(&SignalEvent<'_>) + 'static,
    {
        let class = self.class(class)?;
        let signal = self.member(class, MemberKind::Signal, signal)?;
        self.on_signal(None, class, signal.id, f)
    }

    /// Removes a registration. Its closure is dropped now, or when the callback that is running returns.
    pub fn remove(&self, registration: RegistrationHandle) -> Result<()> {
        // SAFETY: `remove` takes only the registration handle.
        check(unsafe { (self.api.remove)(registration.0) })
    }

    /// Runs queued work and delivers queued events, waiting up to `timeout_ms` for the first. Only a shell that runs its
    /// own loop calls this; the default shell never does. Returns how many callbacks ran.
    pub fn dispatch(&self, timeout_ms: u32) -> Result<u32> {
        // SAFETY: `dispatch` takes the context handle and an integer.
        let rc = unsafe { (self.api.dispatch)(self.ctx, timeout_ms) };
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
        if abi::table_has(self.api.size, end) {
            // SAFETY: the table is long enough to hold `uptime_ms`, so the field was filled by the core.
            Some(unsafe { (self.api.uptime_ms)() })
        } else {
            None
        }
    }

    /// Turns a registering function's result into a [`RegistrationHandle`], reclaiming the box when the core refused it.
    fn adopt<T>(&self, reg: GnoblinReg, slot: *mut T) -> Result<RegistrationHandle> {
        if reg == 0 {
            // SAFETY: the core returned zero, so it has not adopted `slot` and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            Err(CoreError::new(
                crate::ErrorKind::Failed,
                last_text_or("the core refused the registration"),
            ))
        } else {
            REGISTERED.fetch_add(1, Ordering::Relaxed);
            Ok(RegistrationHandle(reg))
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
struct CallbackSlot<F> {
    f: F,
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

unsafe extern "C" fn drop_slot<T>(user: *mut c_void) {
    DROPS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: `user` is the `Box<CallbackSlot<T>>` leaked at registration and the core calls this exactly once, so the box is
    // reclaimed exactly once.
    drop(unsafe { Box::from_raw(user.cast::<CallbackSlot<T>>()) });
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
fn copy_str(raw: &abi::GnoblinStr) -> String {
    if raw.ptr.is_null() || raw.len == 0 {
        return String::new();
    }
    // SAFETY: the core documents the string as `len` bytes at `ptr`, valid for the life of the process.
    let bytes = unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// The core's last-error text, or `fallback` when it has none.
fn last_text_or(fallback: &str) -> String {
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
