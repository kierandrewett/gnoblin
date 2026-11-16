//! A launcher that hands its arguments to the core's scenario driver and exits with its code.
#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int};

// SAFETY: `gnoblin_test_main` is the test driver's export of the core, which the build script's stub declares and the
// staged `libgnoblin-core.so.1` defines. It takes `argc` NUL-terminated strings.
unsafe extern "C" {
    fn gnoblin_test_main(argc: c_int, argv: *const *const c_char) -> c_int;
}

fn main() {
    let args: Vec<CString> = std::env::args_os()
        .map(|arg| CString::new(arg.as_encoded_bytes()).unwrap_or_default())
        .collect();
    let argv: Vec<*const c_char> = args.iter().map(|arg| arg.as_ptr()).collect();
    let argc = c_int::try_from(argv.len()).unwrap_or(c_int::MAX);
    // SAFETY: `argv` holds `argc` pointers to NUL-terminated strings that live until the call returns.
    let code = unsafe { gnoblin_test_main(argc, argv.as_ptr()) };
    std::process::exit(code);
}
