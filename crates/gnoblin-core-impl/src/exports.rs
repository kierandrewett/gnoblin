//! The symbols `libgnoblin-core.so.1` exports. Five of them; everything else is reached through the function table.
//!
//! Nothing else in the library is visible to the dynamic linker: rustc's own export list for a cdylib is its `#[no_mangle]`
//! items, and these are the only ones. `cargo xtask dist-check` compares the library's defined dynamic symbols with
//! `gnoblin-core-abi/exports.txt` and fails on any difference.
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int};
use std::ffi::{CStr, OsString};
use std::os::unix::ffi::OsStringExt as _;

use gnoblin_core_abi::{self as abi, AbiVersionFn, CatalogHashFn, GetApiFn, GnoblinApi, LastErrorFn, MainFn};

use crate::guard::{guard, last_error, set_code_error};
use crate::table::{API, note_table_request};

// The definitions below must have the signatures `gnoblin-core-abi` publishes; the compiler checks it here.
const _: AbiVersionFn = gnoblin_abi_version;
const _: GetApiFn = gnoblin_get_api;
const _: LastErrorFn = gnoblin_last_error;
const _: CatalogHashFn = gnoblin_catalog_hash;
const _: MainFn = gnoblin_main;

/// `gnoblin_abi_version(major, minor, patch)`: the ABI version of this core. Any pointer may be null.
///
/// # Safety
///
/// Each pointer must be null or point at a writable `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_abi_version(major: *mut u32, minor: *mut u32, patch: *mut u32) {
    guard((), "gnoblin_abi_version", || {
        // SAFETY: the caller guarantees each pointer is null or writable.
        unsafe {
            if let Some(major) = major.as_mut() {
                *major = abi::ABI_MAJOR;
            }
            if let Some(minor) = minor.as_mut() {
                *minor = abi::ABI_MINOR;
            }
            if let Some(patch) = patch.as_mut() {
                *patch = abi::ABI_PATCH;
            }
        }
    });
}

/// `gnoblin_get_api(want_major, built_minor, out)`: the function table.
///
/// A different major is refused with `E_ABI` and a last-error text naming both versions. A `built_minor` newer than this
/// core's is accepted: the table's `size` tells the caller which functions it has.
///
/// # Safety
///
/// `out` must point at a writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_get_api(want_major: u32, built_minor: u32, out: *mut *const GnoblinApi) -> i32 {
    guard(abi::E_PANIC, "gnoblin_get_api", || {
        if want_major != abi::ABI_MAJOR {
            set_code_error(
                abi::E_ABI,
                format!(
                    "incompatible ABI: this shell was built for gnoblin ABI {want_major}.{built_minor}; the installed core is {}.{}",
                    abi::ABI_MAJOR,
                    abi::ABI_MINOR
                ),
            );
            return abi::E_ABI;
        }
        // SAFETY: the caller guarantees `out` is null or writable.
        let Some(out) = (unsafe { out.as_mut() }) else {
            set_code_error(abi::E_ERR, "gnoblin_get_api: a null output pointer");
            return abi::E_ERR;
        };
        note_table_request(want_major, built_minor);
        *out = &raw const API;
        abi::E_OK
    })
}

/// `gnoblin_last_error(buf, cap)`: copies this thread's last error text into `buf`, NUL terminated and truncated to
/// `cap` bytes, and returns the text's full length. Works with a null `buf` or a zero `cap` to ask only for the length.
///
/// # Safety
///
/// `buf` must be null or point at `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_last_error(buf: *mut u8, cap: usize) -> usize {
    guard(0, "gnoblin_last_error", || {
        let text = last_error();
        if !buf.is_null() && cap > 0 {
            let kept = text.len().min(cap - 1);
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes; `kept + 1 <= cap`, and the text is a live
            // local that cannot overlap the caller's buffer.
            unsafe {
                core::ptr::copy_nonoverlapping(text.as_ptr(), buf, kept);
                buf.add(kept).write(0);
            }
        }
        text.len()
    })
}

/// `gnoblin_catalog_hash()`: a hash of the schema catalog this core was built with.
#[unsafe(no_mangle)]
pub extern "C" fn gnoblin_catalog_hash() -> u64 {
    crate::catalog::CATALOG_HASH
}

/// `gnoblin_main(argc, argv)`: the launcher's entry. Serves `--role compositor` (see `role`) on the calling thread until the
/// session ends, and returns the process's exit code.
///
/// A panic that reaches this boundary is contained: it is logged by the panic hook the core installed and the exit code is
/// 101, the code Rust itself gives a process that panics.
///
/// # Safety
///
/// `argv` must point at `argc` pointers to NUL-terminated strings that stay valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_main(argc: c_int, argv: *const *const c_char) -> c_int {
    guard(PANIC_EXIT, "gnoblin_main", || {
        let args: Vec<OsString> = (0..usize::try_from(argc).unwrap_or(0))
            .map(|index| {
                // SAFETY: the caller guarantees `argv` holds `argc` pointers to NUL-terminated strings.
                let word = unsafe { CStr::from_ptr(*argv.add(index)) };
                OsString::from_vec(word.to_bytes().to_vec())
            })
            .collect();
        crate::role::run(&args)
    })
}

/// The exit code of a process whose `gnoblin_main` panicked.
const PANIC_EXIT: c_int = 101;
