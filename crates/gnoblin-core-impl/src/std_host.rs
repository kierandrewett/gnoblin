//! Std in the compositor's own process, for the stock session.
//!
//! A session whose shell is not a native module run in process (the script runtime is a child process of the guardian) has no
//! shell that links std, so nothing would register the stock window manager, portal dialogs or configuration observer, and the
//! desktop users get would run `NullWm` and refuse every portal request `no-ui`. The compositor role therefore opens
//! `libgnoblin-std.so.1` itself, when the library is installed, and asks it to register its defaults on a context of the
//! host's own: not a shell generation, so the registrations live as long as the process and survive every shell reload.
//!
//! The library is found beside the core (`<libdir>/libgnoblin-std.so.1`, which is where an install and a build tree put it),
//! then through `$GNOBLIN_STD_DIR`, then by its name through the dynamic loader's own search. A session that cannot find it
//! runs core-only, says so once, and works: `NullWm` and `no-ui` are the safe defaults, not failures.
//!
//! This module names no std crate. It opens a library by name and calls the one function the library exports for this
//! (`gnoblin_std_init`), so the core is the same bytes whether std is installed or not.
#![allow(unsafe_code)]

use core::ffi::{CStr, c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;

use gnoblin_core_abi::{GnoblinCtx, LANG_NATIVE};

use crate::log;
use crate::registry;

/// The file name of the std library.
const SONAME: &str = "libgnoblin-std.so.1";

/// The environment variable that names the directory std is looked for in first.
pub const DIR_VARIABLE: &str = "GNOBLIN_STD_DIR";

/// The symbol the library exports to register its defaults: `int gnoblin_std_init(uint64_t ctx)`.
const INIT_SYMBOL: &CStr = c"gnoblin_std_init";

/// `dlopen`'s flags: bind now, and keep the library's symbols out of the global scope.
const RTLD_NOW: c_int = 2;

unsafe extern "C" {
    fn dlopen(file: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

/// What the host did.
#[derive(Debug, PartialEq, Eq)]
pub enum StdHostOutcome {
    /// Std registered its defaults, from this file.
    Installed(PathBuf),
    /// Std is not installed, or would not load, or would not register: the session runs core-only.
    CoreOnly(String),
}

/// The places std is looked for, in order.
fn candidates() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(dir) = std::env::var_os(DIR_VARIABLE).filter(|dir| !dir.is_empty()) {
        found.push(PathBuf::from(dir).join(SONAME));
    }
    if let Some(core) = gnoblin_shell_host::runtime::library_containing(candidates as fn() -> Vec<PathBuf> as *const ())
        && let Some(dir) = core.parent()
    {
        found.push(dir.join(SONAME));
    }
    // By name, through the dynamic loader's own search path.
    found.push(PathBuf::from(SONAME));
    found
}

fn error_text() -> String {
    // SAFETY: `dlerror` returns null or a NUL-terminated message owned by the loader, read at once.
    let message = unsafe { dlerror() };
    if message.is_null() {
        "unknown error".to_owned()
    } else {
        // SAFETY: as above.
        unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned()
    }
}

/// Opens std and registers its defaults on a context of the host's own. Call it on the thread that then runs the compositor:
/// the context belongs to the calling thread, as a shell's belongs to its shell thread.
pub fn install() -> StdHostOutcome {
    let tried = candidates();
    let mut why = Vec::new();
    for path in &tried {
        // A path that is a bare name is searched for; a path that names a file is opened only when it exists, so a missing
        // file reads as "not there" and not as a loader error.
        if path.components().count() > 1 && !path.is_file() {
            why.push(format!("{}: no such file", path.display()));
            continue;
        }
        let Ok(name) = CString::new(path.as_os_str().as_encoded_bytes()) else {
            continue;
        };
        // SAFETY: `name` is a valid NUL-terminated path for the call. `dlopen` runs the library's constructors, which std
        // has none of (a provider library has no ELF constructor).
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            why.push(format!("{}: {}", path.display(), error_text()));
            continue;
        }
        // SAFETY: `handle` is a live handle from `dlopen`, and the symbol name is NUL-terminated.
        let init = unsafe { dlsym(handle, INIT_SYMBOL.as_ptr()) };
        if init.is_null() {
            return StdHostOutcome::CoreOnly(format!("{} does not export gnoblin_std_init", path.display()));
        }
        // SAFETY: the symbol has the signature this module documents; std is built against the same ABI major, which
        // `gnoblin_std_init` itself checks through the core's table.
        let init: unsafe extern "C" fn(GnoblinCtx) -> i32 = unsafe { core::mem::transmute(init) };
        let ctx = new_context(path);
        // SAFETY: `ctx` is a live context made for this call on this thread.
        let rc = unsafe { init(ctx) };
        if rc != gnoblin_core_abi::E_OK {
            return StdHostOutcome::CoreOnly(format!(
                "{} refused to register its providers ({rc}): {}",
                path.display(),
                crate::guard::last_error()
            ));
        }
        return StdHostOutcome::Installed(path.clone());
    }
    StdHostOutcome::CoreOnly(format!("not found ({})", why.join("; ")))
}

/// A context for the std library: a generation of the host's own that is active from the start and never retires, so what
/// std registers on it lives as long as the process.
fn new_context(path: &std::path::Path) -> GnoblinCtx {
    let ctx = registry::new_context(&path.display().to_string(), LANG_NATIVE, 0, false);
    // An active generation is one whose registrations receive events; this one registers no listener, but a context that is
    // staged forever would be refused by the checks that tell a live generation from one being built.
    if let Err(why) = registry::activate(ctx) {
        log::warn!(area = "std", "the std context did not activate: {why}");
    }
    ctx
}

/// Opens std for the stock session and says what came of it, once, in the core's log.
pub fn install_stock() {
    match install() {
        StdHostOutcome::Installed(path) => {
            log::info!(
                "the stock desktop's policy is std's ({}): window management, portal dialogs and the visual settings",
                path.display()
            );
        }
        StdHostOutcome::CoreOnly(why) => {
            log::info!(
                "libgnoblin-std is not loaded ({why}): this session runs core-only, so windows are placed at the origin by NullWm and portal requests are refused no-ui"
            );
        }
    }
}
