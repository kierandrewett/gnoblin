//! The one call this launcher makes into `libgnoblin-core.so.1`: `gnoblin_main`.
//!
//! `build.rs` links the core through a generated stub that carries its SONAME, so the binary records `NEEDED
//! libgnoblin-core.so.1` and the loader resolves it to the real library at run time (`design/modular-framework.md`
//! section 4.3). This module is the whole `unsafe` surface of the launcher: a C call with a vector of NUL-terminated strings.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, OsStr, OsString, c_char, c_int, c_void};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::PathBuf;

// SAFETY: `gnoblin_main` is the core's fifth export (`gnoblin-core-abi/exports.txt`); the stub of `build.rs` declares it and the
// staged or installed `libgnoblin-core.so.1` defines it. It takes `argc` NUL-terminated strings and returns an exit code.
unsafe extern "C" {
    fn gnoblin_main(argc: c_int, argv: *const *const c_char) -> c_int;
}

/// What `dladdr` fills in about the library an address is in (`<dlfcn.h>`'s `Dl_info`).
#[repr(C)]
struct LoadedObject {
    file_name: *const c_char,
    base: *mut c_void,
    symbol_name: *const c_char,
    symbol: *mut c_void,
}

// SAFETY: `dladdr` is in the C library every Rust binary on a Unix links; it reads an address and fills `info`.
unsafe extern "C" {
    fn dladdr(address: *const c_void, info: *mut LoadedObject) -> c_int;
}

/// The directory `libgnoblin-core.so.1` was loaded from, which is where an installed shell's `module` key is relative to
/// (`<directory>/gnoblin/shells`). Found by asking the dynamic loader where the core's `gnoblin_main` is, so a relocated
/// prefix or a staged build tree answers for itself. `None` when the loader will not say.
pub fn core_directory() -> Option<PathBuf> {
    let mut info = LoadedObject {
        file_name: std::ptr::null(),
        base: std::ptr::null_mut(),
        symbol_name: std::ptr::null(),
        symbol: std::ptr::null_mut(),
    };
    let address = gnoblin_main as *const c_void;
    // SAFETY: `info` is a valid, writable `Dl_info`; `address` is the address of a function this binary links.
    let found = unsafe { dladdr(address, &raw mut info) };
    if found == 0 || info.file_name.is_null() {
        return None;
    }
    // SAFETY: on success `dli_fname` points at a NUL-terminated path the loader owns for the life of the process.
    let path = unsafe { CStr::from_ptr(info.file_name) };
    let directory = PathBuf::from(OsStr::from_bytes(path.to_bytes()))
        .parent()
        .map(PathBuf::from)?;
    // The loader names the file by the run path that found it (`<prefix>/bin/../lib/libgnoblin-core.so.1`); the directory it
    // is in is where an installed shell's `module` is looked for, and is printed, so it is the plain one.
    Some(std::fs::canonicalize(&directory).unwrap_or(directory))
}

/// Serves the compositor role in the core library on this thread and returns its exit code.
pub fn main(arguments: &[OsString]) -> i32 {
    // An argument with a NUL in it cannot be a command line word of a process; it is dropped to the empty word, which the
    // core refuses as a shell path rather than reading a truncated one.
    let owned: Vec<CString> = arguments
        .iter()
        .map(|argument| CString::new(argument.clone().into_vec()).unwrap_or_default())
        .collect();
    let pointers: Vec<*const c_char> = owned.iter().map(|argument| argument.as_ptr()).collect();
    let argc = c_int::try_from(pointers.len()).unwrap_or(c_int::MAX);
    // SAFETY: `pointers` holds `argc` pointers to NUL-terminated strings owned by `owned`, which lives until the call returns.
    unsafe { gnoblin_main(argc, pointers.as_ptr()) }
}
