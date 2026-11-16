//! The programs a shell starts, and the state it keeps across reloads.
//!
//! A program is started through `gnoblin-shell-host`'s `Processes` (one waiter thread per child, a process group of its own, a
//! timeout and bounded capture), which the shell's session owns. Ownership follows the generation: a program a generation's `init`
//! starts waits until that generation takes over (`Session::activated`), and one a reload's generation starts again with the same
//! arguments is adopted, not doubled; a program a handler starts belongs to the user and a reload leaves it running.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinBuf, GnoblinCtx, GnoblinDropFn, GnoblinExitFn, GnoblinProc, GnoblinProcessExit, GnoblinSpawn,
    GnoblinStr,
};
use gnoblin_shell_host::generation::CallbackOutcome;
use gnoblin_shell_host::handle::{decode, encode};
use gnoblin_shell_host::process::{ProcessCode, SpawnOptions, StreamMode};

use crate::events::ShellEvent;
use crate::guard::set_code_error;
use crate::memory;
use crate::registry::{self, Subscription, check_ctx};
use crate::session;
use crate::values::read_text;

/// The kind of a process handle.
const KIND_PROC: u64 = 4;

fn stream(mode: u32, what: &str) -> Result<StreamMode, String> {
    match mode {
        abi::STREAM_INHERIT => Ok(StreamMode::Log),
        abi::STREAM_DISCARD => Ok(StreamMode::Drop),
        abi::STREAM_CAPTURE => Ok(StreamMode::Keep),
        other => Err(format!("{other} is not a stream mode for {what}")),
    }
}

/// The options of a spawn, read from the shell's struct (or the defaults when there is none).
///
/// # Safety
///
/// `options` must be null or point at a `GnoblinSpawn` valid for the call, with the strings and the array it holds.
unsafe fn read_options(options: *const GnoblinSpawn) -> Result<SpawnOptions, String> {
    // SAFETY: the caller guarantees `options` is null or valid for the call.
    let Some(raw) = (unsafe { options.as_ref() }) else {
        return Ok(SpawnOptions::default());
    };
    if (raw.size as usize) < size_of::<GnoblinSpawn>() {
        return Err(format!(
            "spawn options of {} bytes are shorter than this core's ({})",
            raw.size,
            size_of::<GnoblinSpawn>()
        ));
    }
    let text = |value: &GnoblinStr, what: &str| {
        // SAFETY: the caller guarantees the strings of `options` are valid for the call.
        unsafe { read_text(value.ptr, value.len, what) }.map_err(|error| error.message().to_owned())
    };
    let cwd = text(&raw.cwd, "cwd")?;
    let mut env = BTreeMap::new();
    if raw.n_env > 0 {
        if raw.env.is_null() {
            return Err("spawn options with an environment count and no environment".to_owned());
        }
        // SAFETY: the caller guarantees `env` points at `n_env` strings.
        let pairs = unsafe { slice::from_raw_parts(raw.env, raw.n_env) };
        for pair in pairs {
            let pair = text(pair, "env")?;
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("`{pair}` is not NAME=value"))?;
            if name.is_empty() {
                return Err(format!("`{pair}` has no variable name"));
            }
            env.insert(name.to_owned(), value.to_owned());
        }
    }
    let stdin = if raw.stdin.ptr.is_null() {
        None
    } else {
        Some(text(&raw.stdin, "stdin")?)
    };
    Ok(SpawnOptions {
        cwd: (!cwd.is_empty()).then(|| PathBuf::from(cwd)),
        env,
        stdin,
        stdout: stream(raw.stdout, "stdout")?,
        stderr: stream(raw.stderr, "stderr")?,
        output_limit: (raw.output_limit > 0).then_some(raw.output_limit),
        timeout: (raw.timeout_ms > 0.0).then_some(raw.timeout_ms),
    })
}

/// `spawn`.
///
/// # Safety
///
/// `argv` must point at `n_argv` strings and `options` be null or valid, all valid for the call; `out` null or writable.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn spawn(
    ctx: GnoblinCtx,
    argv: *const GnoblinStr,
    n_argv: usize,
    options: *const GnoblinSpawn,
    on_exit: Option<GnoblinExitFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinProc,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "spawn", true) {
        return code;
    }
    if argv.is_null() || n_argv == 0 {
        set_code_error(abi::E_ERR, "spawn: a program needs at least its name");
        return abi::E_ERR;
    }
    // SAFETY: the caller guarantees `argv` points at `n_argv` strings valid for the call.
    let parts = unsafe { slice::from_raw_parts(argv, n_argv) };
    let mut words = Vec::with_capacity(n_argv);
    for part in parts {
        // SAFETY: the caller guarantees each string is valid for the call.
        match unsafe { read_text(part.ptr, part.len, "spawn argv") } {
            Ok(word) => words.push(word),
            Err(error) => {
                set_code_error(error.code(), error.message());
                return error.code();
            }
        }
    }
    // SAFETY: the caller guarantees `options` is null or valid for the call.
    let options = match unsafe { read_options(options) } {
        Ok(options) => options,
        Err(text) => {
            set_code_error(abi::E_ERR, format!("spawn: {text}"));
            return abi::E_ERR;
        }
    };
    let started = match session::with(|session| session.spawn(&words, &options)) {
        Ok(started) => started,
        Err(error) => {
            let code = if error.code == ProcessCode::Busy {
                abi::E_BUSY
            } else {
                abi::E_ERR
            };
            set_code_error(code, format!("spawn: {} ({})", error.message, error.code.as_str()));
            return code;
        }
    };
    let handle = encode(KIND_PROC, started.id);
    if let Some(cb) = on_exit {
        let process = started.id;
        let reg = Arc::new(AtomicU64::new(0));
        let own = Arc::clone(&reg);
        let user_word = user as usize;
        let subscription = Subscription {
            matches: Box::new(
                move |event: &ShellEvent| matches!(event, ShellEvent::Exit(exit) if exit.process == process),
            ),
            call: Arc::new(move |event: &ShellEvent| {
                let ShellEvent::Exit(exit) = event else {
                    return CallbackOutcome::Handled;
                };
                let text = |value: &Option<String>| value.as_deref().map_or(GnoblinStr::EMPTY, GnoblinStr::borrowed);
                let raw = GnoblinProcessExit {
                    size: size_of::<GnoblinProcessExit>() as u32,
                    has_code: u32::from(exit.code.is_some()),
                    code: exit.code.unwrap_or(0),
                    timed_out: u32::from(exit.timed_out),
                    process: handle,
                    signal: text(&exit.signal),
                    stdout: text(&exit.stdout),
                    stderr: text(&exit.stderr),
                    truncated: exit.truncated,
                    reserved: 0,
                };
                // SAFETY: `cb` and `user_word` came from one `spawn` call; `raw` and the strings it points at are live for the call.
                let rc = unsafe { cb(user_word as *mut c_void, &raw const raw) };
                // A program ends once: the registration goes, and `drop_user` with it, after this callback returns.
                registry::remove_id(own.load(Ordering::Acquire));
                registry::outcome_of(rc)
            }),
            cleanup: None,
        };
        match registry::subscribe(ctx, "spawn", subscription, user, drop_user) {
            Ok(id) => reg.store(id, Ordering::Release),
            Err(code) => {
                // The program is running and nobody can hear it end; it is not the shell's any more.
                let _ = session::with(|session| session.kill(started.id, "term"));
                return code;
            }
        }
    }
    if !out.is_null() {
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { out.write(handle) };
    }
    abi::E_OK
}

/// `proc_kill`.
///
/// # Safety
///
/// `signal` must point at `len` readable bytes (or be null with a zero length).
pub(crate) unsafe fn kill(ctx: GnoblinCtx, process: GnoblinProc, signal: *const u8, len: usize) -> i32 {
    if let Err(code) = check_ctx(ctx, "proc_kill", true) {
        return code;
    }
    let Some(id) = decode(KIND_PROC, process) else {
        set_code_error(abi::E_STALE, format!("proc_kill: {process:#x} is not a process handle"));
        return abi::E_STALE;
    };
    // SAFETY: the caller guarantees `signal` is `len` readable bytes.
    let name = match unsafe { read_text(signal, len, "proc_kill signal") } {
        Ok(name) => name,
        Err(error) => {
            set_code_error(error.code(), error.message());
            return error.code();
        }
    };
    match session::with(|session| session.kill(id, &name)) {
        Ok(()) => abi::E_OK,
        Err(error) => {
            let code = if error.code == ProcessCode::NotFound {
                abi::E_STALE
            } else {
                abi::E_ERR
            };
            set_code_error(code, format!("proc_kill: {}", error.message));
            code
        }
    }
}

/// `state_put`.
///
/// # Safety
///
/// `key` and `bytes` must point at `key_len` and `len` readable bytes (or be null with a zero length).
pub(crate) unsafe fn state_put(
    ctx: GnoblinCtx,
    key: *const u8,
    key_len: usize,
    bytes: *const u8,
    len: usize,
    version: u32,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "state_put", true) {
        return code;
    }
    // SAFETY: the caller guarantees `key` is `key_len` readable bytes.
    let key = match unsafe { read_text(key, key_len, "state_put key") } {
        Ok(key) => key,
        Err(error) => {
            set_code_error(error.code(), error.message());
            return error.code();
        }
    };
    let bytes = if len == 0 {
        Vec::new()
    } else if bytes.is_null() {
        set_code_error(abi::E_ERR, "state_put: a null buffer with a nonzero length");
        return abi::E_ERR;
    } else {
        // SAFETY: the caller guarantees `bytes` is `len` readable bytes.
        unsafe { slice::from_raw_parts(bytes, len) }.to_vec()
    };
    match session::with(|session| session.put_state(&key, bytes, version)) {
        Ok(()) => abi::E_OK,
        Err(text) => {
            set_code_error(abi::E_ERR, format!("state_put: {text}"));
            abi::E_ERR
        }
    }
}

/// `state_get`.
///
/// # Safety
///
/// `key` must point at `key_len` readable bytes (or be null with a zero length); `out` and `version` must be writable (or null).
pub(crate) unsafe fn state_get(
    ctx: GnoblinCtx,
    key: *const u8,
    key_len: usize,
    out: *mut GnoblinBuf,
    version: *mut u32,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "state_get", true) {
        return code;
    }
    // SAFETY: the caller guarantees `key` is `key_len` readable bytes.
    let key = match unsafe { read_text(key, key_len, "state_get key") } {
        Ok(key) => key,
        Err(error) => {
            set_code_error(error.code(), error.message());
            return error.code();
        }
    };
    let Some((bytes, kept)) = session::with(|session| session.get_state(&key)) else {
        set_code_error(
            abi::E_NO_SUCH_MEMBER,
            format!("state_get: nothing is kept under `{key}`"),
        );
        return abi::E_NO_SUCH_MEMBER;
    };
    if out.is_null() {
        set_code_error(abi::E_ERR, "state_get: a null output buffer");
        return abi::E_ERR;
    }
    // SAFETY: the caller guarantees `out` is writable, and `version` is null or writable.
    unsafe {
        out.write(memory::lend_bytes(bytes));
        if let Some(version) = version.as_mut() {
            *version = kept;
        }
    }
    abi::E_OK
}
