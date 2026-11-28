//! The interpreter: brought up once, on the shell thread, the first time a `.py` shell is started.
//!
//! Three things happen before any script runs. libpython's symbols are made visible to the C extension modules the script imports
//! (the loader opens this library with `RTLD_LOCAL`, so its `NEEDED` libpython would otherwise stay private and `import math`
//! would fail with an unresolved `PyFloat_Type`). The built-in modules `_gnoblin_core` and `_gnoblin_objects` are
//! registered, so they are importable and nothing else can import them. And the package `gnoblin` is served from this library by a
//! finder on `sys.meta_path` of this interpreter ([`crate::finder`]): no directory is searched for it, and nothing of it is read from disk.
//! Python's own reporting (an uncaught exception, a warning, a `logging` record) is routed into the core's log ([`crate::logging`]).
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::PathBuf;
use std::sync::OnceLock;

use pyo3::prelude::*;

use crate::callback::traceback_text;
use crate::module::_gnoblin_core;
use crate::object_native::_gnoblin_objects;

#[repr(C)]
struct AddressInfo {
    fname: *const c_char,
    fbase: *mut c_void,
    sname: *const c_char,
    saddr: *mut c_void,
}

unsafe extern "C" {
    fn dladdr(addr: *const c_void, info: *mut AddressInfo) -> c_int;
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const RTLD_NOLOAD: c_int = 4;
const RTLD_GLOBAL: c_int = 0x100;

/// The library that holds the code at `address`, as the dynamic loader knows it.
fn library_of(address: *const c_void) -> Option<PathBuf> {
    let mut info = AddressInfo {
        fname: core::ptr::null(),
        fbase: core::ptr::null_mut(),
        sname: core::ptr::null(),
        saddr: core::ptr::null_mut(),
    };
    // SAFETY: `info` is a live local the call fills in; `address` is only looked up, never dereferenced.
    if unsafe { dladdr(address, &raw mut info) } == 0 || info.fname.is_null() {
        return None;
    }
    // SAFETY: on success `fname` is a NUL-terminated path the dynamic loader owns for the life of the library.
    let name = unsafe { CStr::from_ptr(info.fname) };
    Some(PathBuf::from(name.to_string_lossy().into_owned()))
}

/// Makes libpython's symbols global: the loader opened this library `RTLD_LOCAL`, and a C extension module of Python expects the
/// interpreter's symbols in the global scope. The library is already mapped (it is a dependency of this one), so this only changes
/// its visibility.
fn promote_libpython() {
    let address = pyo3::ffi::Py_IsInitialized as *const c_void;
    let Some(path) = library_of(address) else {
        return;
    };
    let Ok(path) = std::ffi::CString::new(path.to_string_lossy().into_owned()) else {
        return;
    };
    // SAFETY: `path` is a valid NUL-terminated string; `RTLD_NOLOAD` opens only a library that is already mapped.
    unsafe { dlopen(path.as_ptr(), RTLD_NOW | RTLD_NOLOAD | RTLD_GLOBAL) };
}

/// Brings the interpreter up, once for the process. The outcome is remembered: a failed boot fails every later `start` with the
/// same text.
pub(crate) fn boot() -> Result<(), String> {
    static BOOT: OnceLock<Result<(), String>> = OnceLock::new();
    BOOT.get_or_init(boot_once).clone()
}

fn boot_once() -> Result<(), String> {
    promote_libpython();
    pyo3::append_to_inittab!(_gnoblin_core);
    pyo3::append_to_inittab!(_gnoblin_objects);
    Python::initialize();
    Python::attach(|py| -> PyResult<()> {
        // The package comes from this library, ahead of anything on the path.
        crate::finder::install(py)?;
        crate::logging::install(py)?;
        py.import("gnoblin")?;
        Ok(())
    })
    .map_err(|error| Python::attach(|py| format!("the Python runtime could not start: {}", traceback_text(py, &error))))
}
