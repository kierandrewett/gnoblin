//! The four symbols the installed core exports, declared for the linker.
//!
//! Nothing else of the core is a direct symbol: everything else is reached through the function table
//! `gnoblin_get_api` returns. The list is `gnoblin-core-abi/exports.txt`, and the build script's link stub defines exactly
//! those symbols, so a symbol declared here and missing from that list fails to link rather than failing at run time.
#![allow(unsafe_code)]

use gnoblin_core_abi::{AbiVersionFn, CatalogHashFn, GetApiFn, GnoblinApi, LastErrorFn};

// SAFETY: these are the exports of `libgnoblin-core`, with the signatures declared by `gnoblin_core_abi` (the constants
// below make the compiler check that). The stub the build script links, and the real core at run time, both define them.
unsafe extern "C" {
    pub(crate) fn gnoblin_abi_version(major: *mut u32, minor: *mut u32, patch: *mut u32);
    pub(crate) fn gnoblin_get_api(want_major: u32, built_minor: u32, out: *mut *const GnoblinApi) -> i32;
    pub(crate) fn gnoblin_last_error(buf: *mut u8, cap: usize) -> usize;
    pub(crate) fn gnoblin_catalog_hash() -> u64;
}

// The declarations above must have the signatures the ABI crate publishes.
const _: AbiVersionFn = gnoblin_abi_version;
const _: GetApiFn = gnoblin_get_api;
const _: LastErrorFn = gnoblin_last_error;
const _: CatalogHashFn = gnoblin_catalog_hash;
