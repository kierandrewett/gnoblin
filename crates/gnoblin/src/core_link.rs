//! The one call this launcher makes into `libgnoblin-core.so.1`: `gnoblin_main`.
//!
//! `build.rs` links the core through a generated stub that carries its SONAME, so the binary records `NEEDED
//! libgnoblin-core.so.1` and the loader resolves it to the real library at run time (`design/modular-framework.md`
//! section 4.3). This module is the whole `unsafe` surface of the launcher: a C call with a vector of NUL-terminated strings.
#![allow(unsafe_code)]

use std::ffi::{CString, OsString, c_char, c_int};
use std::os::unix::ffi::OsStringExt as _;

// SAFETY: `gnoblin_main` is the core's fifth export (`gnoblin-core-abi/exports.txt`); the stub of `build.rs` declares it and the
// staged or installed `libgnoblin-core.so.1` defines it. It takes `argc` NUL-terminated strings and returns an exit code.
unsafe extern "C" {
    fn gnoblin_main(argc: c_int, argv: *const *const c_char) -> c_int;
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
