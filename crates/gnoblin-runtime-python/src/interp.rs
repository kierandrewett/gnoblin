//! The interpreter: brought up once, on the shell thread, the first time a `.py` shell is started.
//!
//! Three things happen before any script runs. libpython's symbols are made visible to the C extension modules the script imports
//! (the loader opens this library with `RTLD_LOCAL`, so its `NEEDED` libpython would otherwise stay private and `import math`
//! would fail with an unresolved `PyFloat_Type`). The built-in module `_gnoblin_core` is registered, so it is importable and nothing
//! else can import it. And the package `gnoblin` is put on `sys.path` of this interpreter only, from the first place that holds it:
//! `$GNOBLIN_PYTHON_DIR`, the prefix of the core library this process mapped (`<prefix>/share/gnoblin/python`), the prefix of the
//! executable, then the data directories (`$XDG_DATA_HOME`, `$XDG_DATA_DIRS`). A source tree is a prefix too: `data/python` is found
//! the same way, so a build tree runs the package it was built from.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gnoblin_core_abi::GnoblinApi;
use pyo3::prelude::*;

use crate::callback::traceback_text;
use crate::module::_gnoblin_core;

#[repr(C)]
struct DlInfo {
    fname: *const c_char,
    fbase: *mut c_void,
    sname: *const c_char,
    saddr: *mut c_void,
}

unsafe extern "C" {
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
}

const RTLD_NOW: c_int = 2;
const RTLD_NOLOAD: c_int = 4;
const RTLD_GLOBAL: c_int = 0x100;

/// The library that holds the code at `address`, as the dynamic loader knows it.
fn library_of(address: *const c_void) -> Option<PathBuf> {
    let mut info = DlInfo {
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

/// Each prefix a library or an executable lives under: the file's directory and its ancestors, nearest first, four levels.
fn prefixes(file: &Path) -> impl Iterator<Item = &Path> {
    file.ancestors().skip(1).take(4)
}

/// The directories that may hold the `gnoblin` package, in the order they are tried, and which one does.
fn locate(api: &GnoblinApi) -> Result<PathBuf, Vec<PathBuf>> {
    let mut tried: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("GNOBLIN_PYTHON_DIR").filter(|dir| !dir.is_empty()) {
        tried.push(PathBuf::from(dir));
    }
    let mut anchors: Vec<PathBuf> = Vec::new();
    anchors.extend(library_of(api.log as *const c_void));
    anchors.extend(std::env::current_exe().ok());
    for anchor in &anchors {
        for prefix in prefixes(anchor) {
            tried.push(prefix.join("share/gnoblin/python"));
            tried.push(prefix.join("data/python"));
        }
    }
    if let Some(home) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        tried.push(PathBuf::from(home).join("gnoblin/python"));
    } else if let Some(home) = std::env::var_os("HOME") {
        tried.push(PathBuf::from(home).join(".local/share/gnoblin/python"));
    }
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    tried.extend(
        dirs.split(':')
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir).join("gnoblin/python")),
    );
    match tried.iter().find(|dir| dir.join("gnoblin/__init__.py").is_file()) {
        Some(found) => Ok(found.clone()),
        None => Err(tried),
    }
}

/// Brings the interpreter up, once for the process. The outcome is remembered: a failed boot fails every later `start` with the
/// same text.
pub(crate) fn boot(api: &GnoblinApi) -> Result<(), String> {
    static BOOT: OnceLock<Result<(), String>> = OnceLock::new();
    BOOT.get_or_init(|| boot_once(api)).clone()
}

fn boot_once(api: &GnoblinApi) -> Result<(), String> {
    let package = locate(api).map_err(|tried| {
        format!(
            "the Python package `gnoblin` was not found (looked for gnoblin/__init__.py in: {}); set GNOBLIN_PYTHON_DIR to its directory",
            tried.iter().map(|dir| dir.display().to_string()).collect::<Vec<_>>().join(", ")
        )
    })?;
    promote_libpython();
    pyo3::append_to_inittab!(_gnoblin_core);
    Python::initialize();
    Python::attach(|py| -> PyResult<()> {
        let path = py.import("sys")?.getattr("path")?;
        path.call_method1("insert", (0, package.display().to_string()))?;
        py.import("gnoblin._runtime")?.call_method0("install")?;
        py.import("gnoblin")?;
        Ok(())
    })
    .map_err(|error| Python::attach(|py| format!("the Python runtime could not start: {}", traceback_text(py, &error))))
}
