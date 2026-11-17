//! The smallest language runtime: it runs `.gnotest` scripts.
//!
//! The first line of a script says what the runtime does with it, so each path of the loader has a script that takes it:
//!
//! | first line   | `start`                                                 | other entry points                      |
//! | ------------ | ------------------------------------------------------- | --------------------------------------- |
//! | `ok`         | succeeds and registers a shutdown hook                  | as the others                           |
//! | `fail`       | returns `E_ERR` saying `the script says fail`           |                                         |
//! | `panic`      | panics inside, contains it as the SDK's trampoline does | returns `E_PANIC` and reports it         |
//! | `noactivate` | succeeds                                                | `activate` returns `E_ERR`              |
//! | `turn-ends`  | succeeds                                                | the first `turn` returns a negative code |
//!
//! Every entry point prints one `[runtime-test] ...` line, so a test reads the order the loader called them in.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::collections::HashMap;
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, PoisonError};

use gnoblin_core_abi::{
    ABI_MAJOR, ABI_MINOR, E_ERR, E_OK, E_PANIC, GnoblinApi, GnoblinCtx, GnoblinCtxInfo, GnoblinRuntimeEntry, GnoblinStr,
};

fn say(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "[runtime-test] {line}");
}

/// The core's table, kept from the first `start`: `shutdown`, `activate` and `turn` are not handed it.
static API: AtomicPtr<GnoblinApi> = AtomicPtr::new(core::ptr::null_mut());

/// What each live context was told to do by its script's first line.
static MODES: Mutex<Option<HashMap<u64, String>>> = Mutex::new(None);

fn mode_of(ctx: GnoblinCtx) -> String {
    MODES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|modes| modes.get(&ctx).cloned())
        .unwrap_or_default()
}

fn api<'a>() -> Option<&'a GnoblinApi> {
    // SAFETY: the pointer is the core's static table, stored by `start`, valid for the life of the process.
    unsafe { API.load(Ordering::Acquire).as_ref() }
}

fn fail_with(ctx: GnoblinCtx, code: i32, message: &str) -> i32 {
    if let Some(api) = api() {
        // SAFETY: `message` is a live string for the call.
        unsafe { (api.set_last_error)(ctx, message.as_ptr(), message.len()) };
    }
    code
}

/// `drop_user` of the shutdown hook: counts that the core released the registration.
unsafe extern "C" fn drop_hook(user: *mut c_void) {
    // SAFETY: `user` is the `Box<String>` leaked in `start`, released exactly once.
    let label = unsafe { Box::from_raw(user.cast::<String>()) };
    say(&format!("drop hook {label}"));
}

unsafe extern "C" fn hook(user: *mut c_void) {
    // SAFETY: `user` is the `Box<String>` leaked in `start`, alive until `drop_hook`.
    let label = unsafe { &*user.cast::<String>() };
    say(&format!("hook {label}"));
}

fn info_of(api: &GnoblinApi, ctx: GnoblinCtx) -> Option<GnoblinCtxInfo> {
    let mut info = GnoblinCtxInfo {
        size: size_of::<GnoblinCtxInfo>() as u32,
        language: 0,
        generation: 0,
        reload_count: 0,
        isolate: 0,
        module_path: GnoblinStr {
            ptr: core::ptr::null(),
            len: 0,
        },
        state_dir: GnoblinStr {
            ptr: core::ptr::null(),
            len: 0,
        },
    };
    // SAFETY: `info` is a live local the call fills in.
    (unsafe { (api.ctx_info)(ctx, &raw mut info) } == E_OK).then_some(info)
}

unsafe extern "C" fn start(api: *const GnoblinApi, ctx: GnoblinCtx, script: GnoblinStr) -> i32 {
    API.store(api.cast_mut(), Ordering::Release);
    // SAFETY: the loader passes the core's table, which lives for the process.
    let Some(api) = (unsafe { api.as_ref() }) else {
        return E_ERR;
    };
    // SAFETY: the loader passes the script's path as `len` bytes that live through the call.
    let path = String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(script.ptr, script.len) }).into_owned();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => return fail_with(ctx, E_ERR, &format!("cannot read {path}: {error}")),
    };
    let mode = text.lines().next().unwrap_or("").trim().to_owned();
    let (generation, language, reload_count) =
        info_of(api, ctx).map_or((0, 0, 0), |info| (info.generation, info.language, info.reload_count));
    say(&format!(
        "start generation={generation} language={language} reload_count={reload_count} mode={mode} path={path}"
    ));
    match mode.as_str() {
        "fail" => return fail_with(ctx, E_ERR, "the script says fail"),
        "panic" => {
            let panicked = catch_unwind(AssertUnwindSafe(|| panic!("boom in the runtime's start")));
            if panicked.is_err() {
                let message = "the runtime's start panicked: boom in the runtime's start";
                // SAFETY: `message` is a live string for the call.
                unsafe { (api.report_panic)(ctx, message.as_ptr(), message.len()) };
                return fail_with(ctx, E_PANIC, message);
            }
        }
        _ => {}
    }
    let label = Box::new(format!("of generation {generation}"));
    // SAFETY: `hook` and `drop_hook` match the callback types; the core owns the box from here and releases it exactly once.
    let registered = unsafe { (api.on_shutdown)(ctx, hook, Box::into_raw(label).cast(), Some(drop_hook)) };
    if registered == 0 {
        return fail_with(ctx, E_ERR, "the shutdown hook was refused");
    }
    MODES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(ctx, mode);
    E_OK
}

unsafe extern "C" fn activate(ctx: GnoblinCtx) -> i32 {
    say(&format!("activate mode={}", mode_of(ctx)));
    if mode_of(ctx) == "noactivate" {
        return fail_with(ctx, E_ERR, "cannot activate this generation");
    }
    E_OK
}

unsafe extern "C" fn abandon(ctx: GnoblinCtx) {
    say(&format!("abandon mode={}", mode_of(ctx)));
    if let Some(modes) = MODES.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        modes.remove(&ctx);
    }
}

unsafe extern "C" fn shutdown(ctx: GnoblinCtx) {
    say(&format!("shutdown mode={}", mode_of(ctx)));
    if let Some(modes) = MODES.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        modes.remove(&ctx);
    }
}

unsafe extern "C" fn turn(ctx: GnoblinCtx, _timeout_ms: u32) -> i32 {
    let mode = mode_of(ctx);
    say(&format!("turn mode={mode}"));
    if mode == "turn-ends" {
        return fail_with(ctx, E_ERR, "the runtime's loop ended");
    }
    1
}

static ENTRY: GnoblinRuntimeEntry = GnoblinRuntimeEntry {
    size: size_of::<GnoblinRuntimeEntry>() as u32,
    abi_major: ABI_MAJOR,
    abi_minor_built: ABI_MINOR,
    flags: 0,
    name: GnoblinStr {
        ptr: b"gnotest".as_ptr(),
        len: 7,
    },
    extensions: GnoblinStr {
        ptr: b"gnotest".as_ptr(),
        len: 7,
    },
    version: GnoblinStr {
        ptr: b"runtime-test 1".as_ptr(),
        len: 14,
    },
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
