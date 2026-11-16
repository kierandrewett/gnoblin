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
    self as abi, GnoblinApi, GnoblinBuf, GnoblinCtx, GnoblinCtxInfo, GnoblinDropFn, GnoblinEventFn, GnoblinMember,
    GnoblinMemberFn, GnoblinNode, GnoblinPostFn, GnoblinReg, GnoblinShutdownFn, GnoblinStats, GnoblinValue,
};

use crate::guard::{guard, set_code_error};
use crate::registry::{self, Registering};
use crate::{catalog, log, memory};

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
