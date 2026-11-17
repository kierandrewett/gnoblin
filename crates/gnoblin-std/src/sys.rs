//! The three symbols the installed std library exports, declared for the linker.
//!
//! Nothing else of the library is a direct symbol: everything else is reached through the function table
//! `gnoblin_std_get_api` returns. The list is `gnoblin-std-abi/exports.txt`, and the build script's link stub defines exactly those
//! symbols, so a symbol declared here and missing from that list fails to link rather than failing at run time.
#![allow(unsafe_code)]

use gnoblin_std_abi::{GnoblinStdApi, StdAbiVersionFn, StdGetApiFn, StdLastErrorFn};

// SAFETY: these are the exports of `libgnoblin-std`, with the signatures declared by `gnoblin_std_abi` (the constants below make the
// compiler check that). The stub the build script links, and the real library at run time, both define them.
unsafe extern "C" {
    pub(crate) fn gnoblin_std_abi_version(major: *mut u32, minor: *mut u32, patch: *mut u32);
    pub(crate) fn gnoblin_std_get_api(want_major: u32, built_minor: u32, out: *mut *const GnoblinStdApi) -> i32;
    pub(crate) fn gnoblin_std_last_error(buf: *mut u8, cap: usize) -> usize;
}

// The declarations above must have the signatures the ABI crate publishes.
const _: StdAbiVersionFn = gnoblin_std_abi_version;
const _: StdGetApiFn = gnoblin_std_get_api;
const _: StdLastErrorFn = gnoblin_std_last_error;
