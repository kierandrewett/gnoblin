//! The function table `gnoblin_get_api` hands out.
//!
//! Each function here is the C-ABI face of one operation in `registry` or `catalog`: it reads the raw arguments, checks
//! them, calls the Rust operation, and turns the result into the ABI's integer codes. Every body runs inside `guard`, so
//! a panic in the core becomes `E_PANIC`, a log line and a last-error text, never an abort.
//!
//! Structs the core writes back (`GnoblinCtxInfo`, `GnoblinStats`, `GnoblinMember`) follow the size convention: the caller
//! sets the `size` of its struct, the core writes at most that many bytes and sets `size` to what it wrote, and a caller
//! built against an older, shorter layout simply does not see the newer fields.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::slice;

use gnoblin_core_abi::{
    self as abi, GnoblinAnswerFn, GnoblinApi, GnoblinBuf, GnoblinClaim, GnoblinClaimFn, GnoblinCtx, GnoblinCtxInfo,
    GnoblinDropFn, GnoblinEventFn, GnoblinExitFn, GnoblinMember, GnoblinMemberFn, GnoblinNode, GnoblinNodeFn,
    GnoblinNodes, GnoblinPending, GnoblinPostFn, GnoblinProc, GnoblinReg, GnoblinShutdownFn, GnoblinSpawn,
    GnoblinStats, GnoblinStr, GnoblinTimerFn, GnoblinValue,
};

use crate::guard::{guard, set_code_error};
use crate::listen::{self, NodeFilter};
use crate::registry::{self, Registering};
use crate::{catalog, link, log, memory, procs, providers, tree, values, verbs_abi};

/// The table of this core, built at compile time. Its `size` is what lets a caller built against another minor probe it.
pub(crate) static API: GnoblinApi = GnoblinApi {
    size: size_of::<GnoblinApi>() as u32,
    major: abi::ABI_MAJOR,
    minor: abi::ABI_MINOR,
    reserved: 0,
    ctx_info: api_ctx_info,
    log: api_log,
    report_panic: api_report_panic,
    set_last_error: api_set_last_error,
    on_shutdown: api_on_shutdown,
    request_reload: api_request_reload,
    request_exit: api_request_exit,
    buf_free: api_buf_free,
    value_free: api_value_free,
    catalog_class: api_catalog_class,
    catalog_member: api_catalog_member,
    catalog_each: api_catalog_each,
    on_signal: api_on_signal,
    remove: api_remove,
    post: api_post,
    dispatch: api_dispatch,
    stats: api_stats,
    tree_attach: api_tree_attach,
    tree_root: api_tree_root,
    node_exists: api_node_exists,
    node_class: api_node_class,
    node_parent: api_node_parent,
    node_children: api_node_children,
    node_get: api_node_get,
    class_name: api_class_name,
    write_create: api_write_create,
    write_destroy: api_write_destroy,
    write_insert: api_write_insert,
    write_set: api_write_set,
    write_call: api_write_call,
    flush: api_flush,
    cancel: api_cancel,
    on_change: api_on_change,
    on_node_created: api_on_node_created,
    on_node_destroyed: api_on_node_destroyed,
    claim: api_claim,
    timer_add: api_timer_add,
    spawn: api_spawn,
    proc_kill: api_proc_kill,
    state_put: api_state_put,
    state_get: api_state_get,
    node_adopt: api_node_adopt,
    tree_link_open: api_tree_link_open,
    tree_link_attach: api_tree_link_attach,
    tree_link_send: api_tree_link_send,
    tree_link_recv: api_tree_link_recv,
    tree_link_fd: api_tree_link_fd,
    write_set_timed: api_write_set_timed,
    class_nodes: api_class_nodes,
    provider_register: api_provider_register,
    setting_set: api_setting_set,
    report_config: api_report_config,
    report_config_rejected: api_report_config_rejected,
    wake_fd: api_wake_fd,
    call: api_call,
    subscribe: api_subscribe,
    #[cfg(feature = "next-minor")]
    uptime_ms: api_uptime_ms,
};

/// UTF-8 text the shell lent for the call, or a recorded `E_ERR` for a null pointer or invalid text.
///
/// # Safety
///
/// `ptr` must be null or point at `len` readable bytes for the lifetime of the returned reference.
unsafe fn text<'a>(ptr: *const u8, len: usize, what: &str) -> Result<&'a str, i32> {
    if len == 0 {
        return Ok("");
    }
    if ptr.is_null() {
        set_code_error(abi::E_ERR, format!("{what}: a null string with a nonzero length"));
        return Err(abi::E_ERR);
    }
    // SAFETY: the caller guarantees `ptr` points at `len` readable bytes.
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).map_err(|error| {
        set_code_error(abi::E_ERR, format!("{what}: not valid UTF-8 ({error})"));
        abi::E_ERR
    })
}

/// Writes `value` through `out` under the size convention: at most the `size` the caller set (zero means the current
/// layout), then `size` is set to what was written.
///
/// # Safety
///
/// `out` must be null or point at a writable struct that starts with a `u32` size field and is at least that many bytes.
unsafe fn write_sized<T: Copy>(out: *mut T, value: T, what: &str) -> i32 {
    if out.is_null() {
        set_code_error(abi::E_ERR, format!("{what}: a null output pointer"));
        return abi::E_ERR;
    }
    let ours = size_of::<T>();
    // SAFETY: the caller guarantees `out` is writable and starts with a `u32`; every output struct does.
    let caller = unsafe { out.cast::<u32>().read() } as usize;
    let size = if caller == 0 { ours } else { caller.min(ours) };
    if size < size_of::<u32>() {
        set_code_error(
            abi::E_ABI,
            format!("{what}: the caller's struct is {size} bytes, too small to carry its size"),
        );
        return abi::E_ABI;
    }
    // SAFETY: `value` is a live local of `ours` bytes, `out` is writable for the caller's `size` bytes, and `size` does
    // not exceed either. The two cannot overlap: `value` is on this stack and `out` is the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(core::ptr::from_ref(&value).cast::<u8>(), out.cast::<u8>(), size);
        out.cast::<u32>().write(size as u32);
    }
    abi::E_OK
}

unsafe extern "C" fn api_ctx_info(ctx: GnoblinCtx, out: *mut GnoblinCtxInfo) -> i32 {
    guard(abi::E_PANIC, "ctx_info", || match registry::ctx_info(ctx) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinCtxInfo`.
        Ok(info) => unsafe { write_sized(out, info, "ctx_info") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_log(ctx: GnoblinCtx, level: u32, msg: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "log", || {
        // SAFETY: the table's contract passes `msg` as `len` readable bytes.
        match unsafe { text(msg, len, "log") } {
            Ok(message) => registry::log_line(ctx, level, message),
            Err(code) => code,
        }
    })
}

unsafe extern "C" fn api_report_panic(ctx: GnoblinCtx, msg: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "report_panic", || {
        // SAFETY: the table's contract passes `msg` as `len` readable bytes.
        let message = unsafe { text(msg, len, "report_panic") }.unwrap_or("a panic message that was not valid text");
        registry::report_panic(ctx, message)
    })
}

unsafe extern "C" fn api_set_last_error(ctx: GnoblinCtx, msg: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "set_last_error", || {
        // SAFETY: the table's contract passes `msg` as `len` readable bytes.
        match unsafe { text(msg, len, "set_last_error") } {
            Ok(message) => registry::set_last_error(ctx, message),
            Err(code) => code,
        }
    })
}

unsafe extern "C" fn api_on_shutdown(
    ctx: GnoblinCtx,
    cb: GnoblinShutdownFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "on_shutdown", || {
        registry::register(ctx, "on_shutdown", Registering::Shutdown { cb }, user, drop_user)
    })
}

unsafe extern "C" fn api_request_reload(ctx: GnoblinCtx) -> i32 {
    guard(abi::E_PANIC, "request_reload", || crate::host::request_reload(ctx))
}

unsafe extern "C" fn api_request_exit(ctx: GnoblinCtx, code: i32) -> i32 {
    guard(abi::E_PANIC, "request_exit", || crate::host::request_exit(ctx, code))
}

unsafe extern "C" fn api_report_config(ctx: GnoblinCtx, revision: u64, document: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "report_config", || {
        if len != 0 && document.is_null() {
            set_code_error(abi::E_ERR, "report_config: a null document with a nonzero length");
            return abi::E_ERR;
        }
        // SAFETY: the table's contract passes `document` as `len` readable bytes for the call; a zero length reads none.
        let bytes = if len == 0 {
            &[][..]
        } else {
            unsafe { slice::from_raw_parts(document, len) }
        };
        crate::host::report_config(ctx, revision, bytes)
    })
}

unsafe extern "C" fn api_report_config_rejected(ctx: GnoblinCtx, revision: u64, reason: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "report_config_rejected", || {
        // SAFETY: the table's contract passes `reason` as `len` readable bytes for the call.
        match unsafe { text(reason, len, "report_config_rejected") } {
            Ok(reason) => crate::host::report_config_rejected(ctx, revision, reason),
            Err(code) => code,
        }
    })
}

unsafe extern "C" fn api_buf_free(buf: *mut GnoblinBuf) -> i32 {
    guard(abi::E_PANIC, "buf_free", || {
        // SAFETY: the table's contract passes a buffer the core returned, or null; the memory module checks the address
        // against the allocations it lent before it frees anything.
        match unsafe { buf.as_mut() } {
            Some(buf) => memory::free_buf(buf),
            None => abi::E_OK,
        }
    })
}

unsafe extern "C" fn api_value_free(value: *mut GnoblinValue) -> i32 {
    guard(abi::E_PANIC, "value_free", || {
        // SAFETY: as for `buf_free`: a value the core returned, or null, checked by the memory module.
        match unsafe { value.as_mut() } {
            Some(value) => memory::free_value(value),
            None => abi::E_OK,
        }
    })
}

unsafe extern "C" fn api_catalog_class(ctx: GnoblinCtx, name: *const u8, len: usize) -> u32 {
    guard(0, "catalog_class", || {
        if registry::state_of(ctx).is_none() {
            set_code_error(abi::E_STALE, "catalog_class: not a context handle");
            return 0;
        }
        // SAFETY: the table's contract passes `name` as `len` readable bytes.
        let Ok(name) = (unsafe { text(name, len, "catalog_class") }) else {
            return 0;
        };
        match catalog::class_id(name) {
            Some(id) => id,
            None => {
                set_code_error(
                    abi::E_NO_SUCH_MEMBER,
                    format!("catalog_class: no class `{name}` in the catalog"),
                );
                0
            }
        }
    })
}

unsafe extern "C" fn api_catalog_member(
    ctx: GnoblinCtx,
    class: u32,
    kind: u32,
    name: *const u8,
    len: usize,
    out: *mut GnoblinMember,
) -> i32 {
    guard(abi::E_PANIC, "catalog_member", || {
        if registry::state_of(ctx).is_none() {
            set_code_error(abi::E_STALE, "catalog_member: not a context handle");
            return abi::E_STALE;
        }
        // SAFETY: the table's contract passes `name` as `len` readable bytes.
        let name = match unsafe { text(name, len, "catalog_member") } {
            Ok(name) => name,
            Err(code) => return code,
        };
        match catalog::member(class, kind, name) {
            // SAFETY: the table's contract passes `out` as a writable `GnoblinMember`.
            Ok(member) => unsafe { write_sized(out, member, "catalog_member") },
            Err(message) => {
                set_code_error(abi::E_NO_SUCH_MEMBER, format!("catalog_member: {message}"));
                abi::E_NO_SUCH_MEMBER
            }
        }
    })
}

unsafe extern "C" fn api_catalog_each(
    ctx: GnoblinCtx,
    class: u32,
    kind: u32,
    cb: GnoblinMemberFn,
    user: *mut c_void,
) -> i32 {
    guard(abi::E_PANIC, "catalog_each", || {
        if registry::state_of(ctx).is_none() {
            set_code_error(abi::E_STALE, "catalog_each: not a context handle");
            return abi::E_STALE;
        }
        let members = match catalog::members(class, kind) {
            Ok(members) => members,
            Err(message) => {
                set_code_error(abi::E_NO_SUCH_MEMBER, format!("catalog_each: {message}"));
                return abi::E_NO_SUCH_MEMBER;
            }
        };
        for member in &members {
            // SAFETY: `cb` and `user` came from the caller of `catalog_each`; `member` is a live local for the call.
            let rc = unsafe { cb(user, core::ptr::from_ref(member)) };
            if rc != abi::E_OK {
                return rc;
            }
        }
        abi::E_OK
    })
}

unsafe extern "C" fn api_on_signal(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    class: u32,
    signal: u32,
    cb: GnoblinEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "on_signal", || {
        registry::register(
            ctx,
            "on_signal",
            Registering::Signal {
                node,
                class,
                signal,
                cb,
            },
            user,
            drop_user,
        )
    })
}

unsafe extern "C" fn api_remove(reg: GnoblinReg) -> i32 {
    guard(abi::E_PANIC, "remove", || registry::remove(reg))
}

unsafe extern "C" fn api_post(
    ctx: GnoblinCtx,
    f: GnoblinPostFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> i32 {
    guard(abi::E_PANIC, "post", || registry::post(ctx, f, user, drop_user))
}

unsafe extern "C" fn api_dispatch(ctx: GnoblinCtx, timeout_ms: u32) -> i32 {
    guard(abi::E_PANIC, "dispatch", || registry::dispatch(ctx, timeout_ms))
}

unsafe extern "C" fn api_stats(ctx: GnoblinCtx, out: *mut GnoblinStats) -> i32 {
    guard(abi::E_PANIC, "stats", || match registry::stats(ctx) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinStats`.
        Ok(stats) => unsafe { write_sized(out, stats, "stats") },
        Err(code) => code,
    })
}

// ---- the tree, claims, timers, processes and state ------------------------------------------------------------------------

/// Writes `value` through `out` after checking it is not null: a function that returns a value through a pointer.
///
/// # Safety
///
/// `out` must be null or writable.
unsafe fn put<T>(out: *mut T, value: T, what: &str) -> i32 {
    if out.is_null() {
        set_code_error(abi::E_ERR, format!("{what}: a null output pointer"));
        return abi::E_ERR;
    }
    // SAFETY: the caller guarantees a non-null `out` is writable.
    unsafe { out.write(value) };
    abi::E_OK
}

unsafe extern "C" fn api_tree_attach(ctx: GnoblinCtx, mode: u32) -> i32 {
    guard(abi::E_PANIC, "tree_attach", || tree::attach(ctx, mode))
}

unsafe extern "C" fn api_tree_root(ctx: GnoblinCtx, out: *mut GnoblinNode) -> i32 {
    guard(abi::E_PANIC, "tree_root", || match tree::root(ctx) {
        // SAFETY: the table's contract passes `out` as a writable node id.
        Ok(root) => unsafe { put(out, root, "tree_root") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_node_exists(ctx: GnoblinCtx, node: GnoblinNode) -> i32 {
    guard(abi::E_PANIC, "node_exists", || tree::node_exists(ctx, node))
}

unsafe extern "C" fn api_node_class(ctx: GnoblinCtx, node: GnoblinNode) -> u32 {
    guard(0, "node_class", || tree::node_class(ctx, node))
}

unsafe extern "C" fn api_node_parent(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNode) -> i32 {
    guard(abi::E_PANIC, "node_parent", || match tree::node_parent(ctx, node) {
        // SAFETY: the table's contract passes `out` as a writable node id.
        Ok(parent) => unsafe { put(out, parent, "node_parent") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_node_children(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNodes) -> i32 {
    guard(abi::E_PANIC, "node_children", || match tree::node_children(ctx, node) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinNodes`.
        Ok(children) => unsafe { put(out, values::lend_nodes(&children), "node_children") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_provider_register(
    ctx: GnoblinCtx,
    kind: u32,
    vtable: *const c_void,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "provider_register", || {
        providers::register(ctx, kind, vtable, user, drop_user)
    })
}

unsafe extern "C" fn api_setting_set(
    ctx: GnoblinCtx,
    key: *const u8,
    key_len: usize,
    value: *const GnoblinValue,
) -> i32 {
    guard(abi::E_PANIC, "setting_set", || {
        // SAFETY: the table's contract passes `key` as `key_len` readable bytes.
        match unsafe { text(key, key_len, "setting_set") } {
            Ok(key) => providers::setting_set(ctx, key, value),
            Err(code) => code,
        }
    })
}

unsafe extern "C" fn api_class_nodes(ctx: GnoblinCtx, class: u32, out: *mut GnoblinNodes) -> i32 {
    guard(abi::E_PANIC, "class_nodes", || match tree::class_nodes(ctx, class) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinNodes`.
        Ok(nodes) => unsafe { put(out, values::lend_nodes(&nodes), "class_nodes") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_node_get(ctx: GnoblinCtx, node: GnoblinNode, prop: u32, out: *mut GnoblinValue) -> i32 {
    guard(abi::E_PANIC, "node_get", || match tree::node_get(ctx, node, prop) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinValue`.
        Ok(value) => unsafe { put(out, value, "node_get") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_class_name(ctx: GnoblinCtx, class: u32, out: *mut GnoblinStr) -> i32 {
    guard(abi::E_PANIC, "class_name", || match tree::class_name(ctx, class) {
        // SAFETY: the table's contract passes `out` as a writable `GnoblinStr`.
        Ok(name) => unsafe { put(out, name, "class_name") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_write_create(ctx: GnoblinCtx, class: u32, out: *mut GnoblinNode) -> i32 {
    guard(abi::E_PANIC, "write_create", || match tree::create(ctx, class) {
        // SAFETY: the table's contract passes `out` as a writable node id.
        Ok(node) => unsafe { put(out, node, "write_create") },
        Err(code) => code,
    })
}

unsafe extern "C" fn api_write_destroy(ctx: GnoblinCtx, node: GnoblinNode) -> i32 {
    guard(abi::E_PANIC, "write_destroy", || tree::destroy(ctx, node))
}

unsafe extern "C" fn api_write_insert(ctx: GnoblinCtx, parent: GnoblinNode, node: GnoblinNode, index: i64) -> i32 {
    guard(abi::E_PANIC, "write_insert", || tree::insert(ctx, parent, node, index))
}

unsafe extern "C" fn api_write_set(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    prop: u32,
    value: *const GnoblinValue,
    seconds: f64,
) -> i32 {
    guard(abi::E_PANIC, "write_set", || {
        // SAFETY: the table's contract passes `value` as a readable `GnoblinValue` valid for the call.
        unsafe { tree::set(ctx, node, prop, value, seconds) }
    })
}

unsafe extern "C" fn api_write_set_timed(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    prop: u32,
    value: *const GnoblinValue,
    timing: *const abi::GnoblinTiming,
) -> i32 {
    guard(abi::E_PANIC, "write_set_timed", || {
        // SAFETY: the table's contract passes `value` and `timing` as readable structs valid for the call.
        unsafe { tree::set_timed(ctx, node, prop, value, timing) }
    })
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn api_write_call(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    method: u32,
    args: *const GnoblinValue,
    n_args: usize,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    guard(abi::E_PANIC, "write_call", || {
        // SAFETY: the table's contract passes `args` as `n_args` readable values and `out` as null or writable.
        unsafe { tree::call(ctx, node, method, args, n_args, cb, user, drop_user, out) }
    })
}

unsafe extern "C" fn api_flush(
    ctx: GnoblinCtx,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    guard(abi::E_PANIC, "flush", || {
        // SAFETY: the table's contract passes `out` as null or writable.
        unsafe { tree::flush(ctx, cb, user, drop_user, out) }
    })
}

unsafe extern "C" fn api_cancel(pending: GnoblinPending) -> i32 {
    guard(abi::E_PANIC, "cancel", || tree::cancel(pending))
}

unsafe extern "C" fn api_on_change(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    class: u32,
    prop: u32,
    cb: GnoblinNodeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "on_change", || {
        listen::on_node(
            ctx,
            "on_change",
            NodeFilter {
                kind: abi::NODE_CHANGED,
                node,
                class,
                property: prop,
            },
            cb,
            user,
            drop_user,
        )
    })
}

unsafe extern "C" fn api_on_node_created(
    ctx: GnoblinCtx,
    class: u32,
    cb: GnoblinNodeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "on_node_created", || {
        listen::on_node(
            ctx,
            "on_node_created",
            NodeFilter {
                kind: abi::NODE_CREATED,
                node: 0,
                class,
                property: 0,
            },
            cb,
            user,
            drop_user,
        )
    })
}

unsafe extern "C" fn api_on_node_destroyed(
    ctx: GnoblinCtx,
    class: u32,
    cb: GnoblinNodeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "on_node_destroyed", || {
        listen::on_node(
            ctx,
            "on_node_destroyed",
            NodeFilter {
                kind: abi::NODE_DESTROYED,
                node: 0,
                class,
                property: 0,
            },
            cb,
            user,
            drop_user,
        )
    })
}

unsafe extern "C" fn api_claim(
    ctx: GnoblinCtx,
    seat: GnoblinNode,
    spec: *const GnoblinClaim,
    cb: Option<GnoblinClaimFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "claim", || {
        // SAFETY: the table's contract passes `spec` as a readable `GnoblinClaim` with valid strings.
        unsafe { listen::claim(ctx, seat, spec, cb, user, drop_user) }
    })
}

unsafe extern "C" fn api_timer_add(
    ctx: GnoblinCtx,
    ms: u32,
    repeat: u32,
    cb: GnoblinTimerFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "timer_add", || {
        listen::timer_add(ctx, ms, repeat, cb, user, drop_user)
    })
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn api_spawn(
    ctx: GnoblinCtx,
    argv: *const GnoblinStr,
    n_argv: usize,
    options: *const GnoblinSpawn,
    on_exit: Option<GnoblinExitFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinProc,
) -> i32 {
    guard(abi::E_PANIC, "spawn", || {
        // SAFETY: the table's contract passes `argv` as `n_argv` readable strings, `options` as null or readable, and `out`
        // as null or writable.
        unsafe { procs::spawn(ctx, argv, n_argv, options, on_exit, user, drop_user, out) }
    })
}

unsafe extern "C" fn api_proc_kill(ctx: GnoblinCtx, process: GnoblinProc, signal: *const u8, len: usize) -> i32 {
    guard(abi::E_PANIC, "proc_kill", || {
        // SAFETY: the table's contract passes `signal` as `len` readable bytes.
        unsafe { procs::kill(ctx, process, signal, len) }
    })
}

unsafe extern "C" fn api_state_put(
    ctx: GnoblinCtx,
    key: *const u8,
    key_len: usize,
    bytes: *const u8,
    len: usize,
    version: u32,
) -> i32 {
    guard(abi::E_PANIC, "state_put", || {
        // SAFETY: the table's contract passes `key` and `bytes` as readable byte ranges of the lengths given.
        unsafe { procs::state_put(ctx, key, key_len, bytes, len, version) }
    })
}

unsafe extern "C" fn api_state_get(
    ctx: GnoblinCtx,
    key: *const u8,
    key_len: usize,
    out: *mut GnoblinBuf,
    version: *mut u32,
) -> i32 {
    guard(abi::E_PANIC, "state_get", || {
        // SAFETY: the table's contract passes `key` as `key_len` readable bytes and `out` and `version` as writable.
        unsafe { procs::state_get(ctx, key, key_len, out, version) }
    })
}

unsafe extern "C" fn api_node_adopt(
    ctx: GnoblinCtx,
    class: u32,
    name: *const u8,
    name_len: usize,
    flags: u32,
    out: *mut GnoblinNode,
) -> i32 {
    guard(abi::E_PANIC, "node_adopt", || {
        // SAFETY: the table's contract passes `name` as `name_len` readable bytes.
        let name = match unsafe { text(name, name_len, "node_adopt name") } {
            Ok(name) => name,
            Err(code) => return code,
        };
        match tree::adopt(ctx, class, name, flags) {
            // SAFETY: the table's contract passes `out` as a writable node id.
            Ok((node, adopted)) => match unsafe { put(out, node, "node_adopt") } {
                abi::E_OK if adopted => abi::ADOPTED,
                code => code,
            },
            Err(code) => code,
        }
    })
}

/// `wake_fd`.
unsafe extern "C" fn api_wake_fd(ctx: GnoblinCtx) -> i32 {
    guard(abi::E_PANIC, "wake_fd", || registry::wake_fd(ctx))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn api_call(
    ctx: GnoblinCtx,
    object: u64,
    class: u32,
    method: u32,
    args: *const GnoblinValue,
    n_args: usize,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    // SAFETY: the table's contract passes `args` as `n_args` values and `out` as a writable handle, both valid for the call.
    guard(abi::E_PANIC, "call", || unsafe {
        verbs_abi::call(ctx, object, class, method, args, n_args, cb, user, drop_user, out)
    })
}

unsafe extern "C" fn api_subscribe(
    ctx: GnoblinCtx,
    class: u32,
    signal: u32,
    cb: GnoblinEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    guard(0, "subscribe", || {
        verbs_abi::subscribe(ctx, class, signal, cb, user, drop_user)
    })
}

#[cfg(feature = "next-minor")]
unsafe extern "C" fn api_uptime_ms() -> u64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    guard(0, "uptime_ms", || {
        u64::try_from(START.elapsed().as_millis()).unwrap_or(u64::MAX)
    })
}

/// Logs that a table was handed out, once per request: useful when a shell fails to find a core of the right major.
pub(crate) fn note_table_request(want_major: u32, built_minor: u32) {
    log::debug!("table requested for ABI {want_major}.{built_minor}");
}

unsafe extern "C" fn api_tree_link_open(ctx: GnoblinCtx, mode: u32) -> i32 {
    guard(abi::E_PANIC, "tree_link_open", || link::open(ctx, mode))
}

unsafe extern "C" fn api_tree_link_attach(ctx: GnoblinCtx, version: u64) -> i32 {
    guard(abi::E_PANIC, "tree_link_attach", || link::attach(ctx, version))
}

unsafe extern "C" fn api_tree_link_send(ctx: GnoblinCtx, bytes: *const u8, len: usize) -> i32 {
    // SAFETY: the table's contract passes `bytes` as `len` readable bytes.
    guard(abi::E_PANIC, "tree_link_send", || unsafe {
        link::send(ctx, bytes, len)
    })
}

unsafe extern "C" fn api_tree_link_recv(ctx: GnoblinCtx, out: *mut GnoblinBuf) -> i32 {
    // SAFETY: the table's contract passes `out` as a writable buffer.
    guard(abi::E_PANIC, "tree_link_recv", || unsafe { link::recv(ctx, out) })
}

unsafe extern "C" fn api_tree_link_fd(ctx: GnoblinCtx) -> i32 {
    guard(-1, "tree_link_fd", || link::fd(ctx))
}
