//! The core as the runtime reaches it: the function table `start` hands over, and the one export the table has no function for.
//!
//! Every call a runtime makes into the core is here, so the `unsafe` is in one module and each call states what makes it sound.
#![allow(unsafe_code)]

use std::cell::Cell;
use std::os::fd::{FromRawFd as _, OwnedFd};

use core::ffi::c_void;

use gnoblin_core_abi::{
    self as abi, GnoblinAnswerFn, GnoblinApi, GnoblinBuf, GnoblinCtx, GnoblinDropFn, GnoblinEventFn, GnoblinMember,
    GnoblinPending, GnoblinReg, GnoblinStr, GnoblinValue, LastErrorFn,
};

unsafe extern "C" {
    fn gnoblin_last_error(buf: *mut u8, cap: usize) -> usize;
}

// The declaration above must have the signature the ABI crate publishes.
const _: LastErrorFn = gnoblin_last_error;

/// The text the core left for the last failure on this thread.
pub(crate) fn last_error() -> String {
    // SAFETY: with a null buffer the call only reports the text's length.
    let len = unsafe { gnoblin_last_error(core::ptr::null_mut(), 0) };
    if len == 0 {
        return String::new();
    }
    let mut bytes = vec![0_u8; len + 1];
    // SAFETY: `bytes` is writable for `len + 1` bytes, which is the capacity passed.
    unsafe { gnoblin_last_error(bytes.as_mut_ptr(), bytes.len()) };
    bytes.truncate(len);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What the core said about a call that failed: its code and the text it left.
pub(crate) fn failure(what: &str, code: i32) -> String {
    let text = last_error();
    if text.is_empty() {
        format!("{what} failed with code {code}")
    } else {
        text
    }
}

/// The core's table, which lives for the rest of the process (the core is linked with `-z nodelete`).
#[derive(Clone, Copy)]
pub(crate) struct CoreTable {
    api: &'static GnoblinApi,
}

impl CoreTable {
    /// The table of a core this process holds (`embed`).
    pub(crate) fn new(api: &'static GnoblinApi) -> Self {
        Self { api }
    }

    /// The table a runtime's `start` was handed.
    ///
    /// # Safety
    ///
    /// `api` must be null or the pointer the loader passes to `start`: the core's static table.
    pub(crate) unsafe fn from_table(api: *const GnoblinApi) -> Option<Self> {
        // SAFETY: the caller guarantees `api` is null or the core's static table, which is never unmapped, so it is valid for
        // `'static`.
        let api: Option<&'static GnoblinApi> = unsafe { api.as_ref() };
        api.map(|api| Self { api })
    }

    /// The file of the core library this table belongs to: where the installation the runtime was started from is. The runtime's
    /// own file is the loader's private copy, which says nothing about the prefix.
    pub(crate) fn core_file(self) -> Option<std::path::PathBuf> {
        gnoblin_shell_host::runtime::library_containing(self.api.log as *const ())
    }

    /// Writes `text` to the core's log at `level` (`LOG_*`), tagged with the generation of `ctx`.
    pub(crate) fn log(self, ctx: GnoblinCtx, level: u32, text: &str) {
        // SAFETY: `text` is a live string for the call.
        unsafe { (self.api.log)(ctx, level, text.as_ptr(), text.len()) };
    }

    /// Sets the text `gnoblin_last_error` returns, so the loader can say why a `start` failed.
    pub(crate) fn set_last_error(self, ctx: GnoblinCtx, text: &str) {
        // SAFETY: `text` is a live string for the call.
        unsafe { (self.api.set_last_error)(ctx, text.as_ptr(), text.len()) };
    }

    /// An address inside the core: where the core is, for finding the std library beside it.
    pub(crate) fn anchor(self) -> *const std::ffi::c_void {
        self.api.log as *const std::ffi::c_void
    }

    /// Runs what the core queued for the generation `ctx`: what the std library announces and what a layout is asked.
    pub(crate) fn dispatch(self, ctx: GnoblinCtx) -> i32 {
        // SAFETY: the function takes the context and a timeout; a zero timeout does not wait.
        unsafe { (self.api.dispatch)(ctx, 0) }
    }

    /// Reports a panic the runtime caught, so the core logs it with the generation.
    pub(crate) fn report_panic(self, ctx: GnoblinCtx, text: &str) {
        // SAFETY: `text` is a live string for the call.
        unsafe { (self.api.report_panic)(ctx, text.as_ptr(), text.len()) };
    }

    /// Asks the host to reload this shell. Any thread.
    pub(crate) fn request_reload(self, ctx: GnoblinCtx) -> i32 {
        // SAFETY: the function takes only the context.
        unsafe { (self.api.request_reload)(ctx) }
    }

    /// Tells the host the shell committed `document` as configuration `revision`; the guardian hears it (`report_config`).
    pub(crate) fn report_config(self, ctx: GnoblinCtx, revision: u64, document: &[u8]) -> Result<(), String> {
        // SAFETY: `document` is a live slice for the call.
        match unsafe { (self.api.report_config)(ctx, revision, document.as_ptr(), document.len()) } {
            abi::E_OK => Ok(()),
            code => Err(failure("report_config", code)),
        }
    }

    /// Tells the host the shell refused a configuration revision (`report_config_rejected`).
    pub(crate) fn report_config_rejected(self, ctx: GnoblinCtx, revision: u64, reason: &str) -> Result<(), String> {
        // SAFETY: `reason` is a live string for the call.
        match unsafe { (self.api.report_config_rejected)(ctx, revision, reason.as_ptr(), reason.len()) } {
            abi::E_OK => Ok(()),
            code => Err(failure("report_config_rejected", code)),
        }
    }

    /// Opens the compositor's link as the shell.
    pub(crate) fn link_open(self, ctx: GnoblinCtx) -> Result<(), String> {
        // SAFETY: the function takes the context and a mode constant.
        match unsafe { (self.api.tree_link_open)(ctx, abi::ATTACH_HOLDER) } {
            abi::E_OK => Ok(()),
            code => Err(failure("tree_link_open", code)),
        }
    }

    /// Sends the attach request.
    pub(crate) fn link_attach(self, ctx: GnoblinCtx, version: u64) -> Result<(), String> {
        // SAFETY: the function takes the context and a number.
        match unsafe { (self.api.tree_link_attach)(ctx, version) } {
            abi::E_OK => Ok(()),
            code => Err(failure("tree_link_attach", code)),
        }
    }

    /// Sends one encoded transaction.
    pub(crate) fn link_send(self, ctx: GnoblinCtx, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: `bytes` is a live slice for the call.
        match unsafe { (self.api.tree_link_send)(ctx, bytes.as_ptr(), bytes.len()) } {
            abi::E_OK => Ok(()),
            code => Err(failure("tree_link_send", code)),
        }
    }

    /// Takes the encoded batch of everything that arrived, copying it out and freeing the core's buffer.
    pub(crate) fn link_recv(self, ctx: GnoblinCtx) -> Result<Vec<u8>, String> {
        let mut buf = GnoblinBuf {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        // SAFETY: `buf` is a live local the call fills in.
        let code = unsafe { (self.api.tree_link_recv)(ctx, &raw mut buf) };
        if code != abi::E_OK {
            return Err(failure("tree_link_recv", code));
        }
        let bytes = if buf.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: the core filled `buf` with `len` readable bytes it allocated.
            unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
        };
        // SAFETY: `buf` is the buffer the core returned, freed once through its own function.
        unsafe { (self.api.buf_free)(&raw mut buf) };
        Ok(bytes)
    }

    /// The object protocol's table of `namespace` (`gnoblin.model`) for the generation `ctx`: the classes the core serves.
    pub(crate) fn object_api(self, ctx: GnoblinCtx, namespace: &str) -> Result<&'static abi::GnoblinObjectApi, String> {
        let mut table: *const abi::GnoblinObjectApi = core::ptr::null();
        // SAFETY: `namespace` is borrowed for the call and `table` is a live local the call writes.
        let code = unsafe { (self.api.object_api)(ctx, GnoblinStr::borrowed(namespace), &raw mut table) };
        if code != abi::E_OK {
            return Err(failure("object_api", code));
        }
        // SAFETY: on success the core wrote a pointer to its static table, which is never unmapped.
        unsafe { table.as_ref() }.ok_or_else(|| "the core gave no object table".to_owned())
    }

    /// The catalog's id of the class `name` (`gnoblin.session`), or `None` when the core has no such class.
    pub(crate) fn class_id(self, ctx: GnoblinCtx, name: &str) -> Option<u32> {
        // SAFETY: `name` is a live string for the call.
        let id = unsafe { (self.api.catalog_class)(ctx, name.as_ptr(), name.len()) };
        (id != 0).then_some(id)
    }

    /// The catalog's member `name` of `kind` (`MEMBER_*`) of `class`.
    pub(crate) fn member(self, ctx: GnoblinCtx, class: u32, kind: u32, name: &str) -> Result<GnoblinMember, String> {
        let mut member = GnoblinMember {
            size: size_of::<GnoblinMember>() as u32,
            id: 0,
            kind: 0,
            value_kind: 0,
            flags: 0,
            arg_count: 0,
            arg_kinds: core::ptr::null(),
            arg_names: GnoblinStr::EMPTY,
        };
        // SAFETY: `name` is a live string and `member` a live local the call fills in, both for the call.
        match unsafe { (self.api.catalog_member)(ctx, class, kind, name.as_ptr(), name.len(), &raw mut member) } {
            abi::E_OK => Ok(member),
            code => Err(failure("catalog_member", code)),
        }
    }

    /// Calls a verb of the compositor's `gnoblin` namespace (`call`). The core adopts `user` only when this returns `Ok`.
    ///
    /// # Safety
    ///
    /// `user` must be what `cb` and `drop_user` expect, and stay valid until the core calls `drop_user` or refuses the call.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn call(
        self,
        ctx: GnoblinCtx,
        object: u64,
        class: u32,
        method: u32,
        args: &[GnoblinValue],
        cb: Option<GnoblinAnswerFn>,
        user: *mut c_void,
        drop_user: Option<GnoblinDropFn>,
    ) -> Result<(), String> {
        let mut pending: GnoblinPending = 0;
        // SAFETY: `args` is a live slice for the call; `user` is the caller's, per the contract above.
        let code = unsafe {
            (self.api.call)(
                ctx,
                object,
                class,
                method,
                args.as_ptr(),
                args.len(),
                cb,
                user,
                drop_user,
                &raw mut pending,
            )
        };
        if code < abi::E_OK {
            Err(failure("call", code))
        } else {
            Ok(())
        }
    }

    /// Calls `cb` for every `signal` of `class` the compositor raises (`subscribe`). The core adopts `user` only when this
    /// returns `Ok`, and the registration it returns removes it.
    ///
    /// # Safety
    ///
    /// `user` must be what `cb` and `drop_user` expect, and stay valid until the core calls `drop_user` or refuses.
    pub(crate) unsafe fn subscribe(
        self,
        ctx: GnoblinCtx,
        class: u32,
        signal: u32,
        cb: GnoblinEventFn,
        user: *mut c_void,
        drop_user: Option<GnoblinDropFn>,
    ) -> Result<GnoblinReg, String> {
        // SAFETY: `user` is the caller's, per the contract above.
        let reg = unsafe { (self.api.subscribe)(ctx, class, signal, cb, user, drop_user) };
        if reg == 0 {
            Err(failure("subscribe", abi::E_ERR))
        } else {
            Ok(reg)
        }
    }

    /// Removes a registration the core returned.
    pub(crate) fn remove(self, reg: GnoblinReg) -> i32 {
        // SAFETY: the function takes only the registration.
        unsafe { (self.api.remove)(reg) }
    }

    /// The core's wake descriptor, owned by the caller: readable when `dispatch` has something to do.
    pub(crate) fn wake_fd(self, ctx: GnoblinCtx) -> Option<OwnedFd> {
        // SAFETY: the function takes only the context.
        let fd = unsafe { (self.api.wake_fd)(ctx) };
        // SAFETY: a non-negative result is a duplicate the core made for the caller, who owns it.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// A descriptor that turns readable when the link has something to take, owned by the caller; `None` when the link has none.
    pub(crate) fn link_fd(self, ctx: GnoblinCtx) -> Option<OwnedFd> {
        // SAFETY: the function takes only the context.
        let fd = unsafe { (self.api.tree_link_fd)(ctx) };
        // SAFETY: a non-negative result is a duplicate the core made for the caller, who owns it.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

thread_local! {
    /// The context the runtime's log lines are tagged with: the generation that started last.
    static LOG_CTX: Cell<GnoblinCtx> = const { Cell::new(0) };
}

/// Sets the context log lines are tagged with.
pub(crate) fn set_log_context(ctx: GnoblinCtx) {
    LOG_CTX.with(|cell| cell.set(ctx));
}

/// The context log lines are tagged with.
pub(crate) fn log_context() -> GnoblinCtx {
    LOG_CTX.with(Cell::get)
}

thread_local! {
    /// The table this thread's shell was started with, for the entry points that are not handed it (`turn`, `shutdown`).
    static REMEMBERED: Cell<Option<CoreTable>> = const { Cell::new(None) };
}

/// Keeps `table` for the entry points that are not handed one.
pub(crate) fn remember(table: CoreTable) {
    REMEMBERED.with(|cell| cell.set(Some(table)));
}

/// The table `start` was handed on this thread, once it has run.
pub(crate) fn remembered() -> Option<CoreTable> {
    REMEMBERED.with(Cell::get)
}
