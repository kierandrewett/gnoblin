//! The symbols `libgnoblin-std.so.1` exports. Three of them; everything else is reached through the function table.
//!
//! Nothing else in the library is visible to the dynamic linker: rustc's own export list for a cdylib is its `#[no_mangle]` items,
//! and these are the only ones. `cargo xtask dist-check` compares the library's defined dynamic symbols with
//! `gnoblin-std-abi/exports.txt` and fails on any difference.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi as abi;
use gnoblin_std_abi::{ABI_MAJOR, ABI_MINOR, ABI_PATCH, GnoblinStdApi, StdAbiVersionFn, StdGetApiFn, StdLastErrorFn};

use crate::error::{last_text, set_last};
use crate::table::API;

// The definitions below must have the signatures `gnoblin-std-abi` publishes; the compiler checks it here.
const _: StdAbiVersionFn = gnoblin_std_abi_version;
const _: StdGetApiFn = gnoblin_std_get_api;
const _: StdLastErrorFn = gnoblin_std_last_error;

/// `gnoblin_std_abi_version(major, minor, patch)`: the std ABI version of this library. Any pointer may be null.
///
/// # Safety
///
/// Each pointer must be null or point at a writable `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_std_abi_version(major: *mut u32, minor: *mut u32, patch: *mut u32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller guarantees each pointer is null or writable.
        unsafe {
            if let Some(major) = major.as_mut() {
                *major = ABI_MAJOR;
            }
            if let Some(minor) = minor.as_mut() {
                *minor = ABI_MINOR;
            }
            if let Some(patch) = patch.as_mut() {
                *patch = ABI_PATCH;
            }
        }
    }));
}

/// `gnoblin_std_get_api(want_major, built_minor, out)`: the function table.
///
/// A different major is refused with `E_ABI` and a last-error text naming both versions. A `built_minor` newer than this library's is
/// accepted: the table's `size` tells the caller which functions it has.
///
/// # Safety
///
/// `out` must point at a writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_std_get_api(want_major: u32, built_minor: u32, out: *mut *const GnoblinStdApi) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if want_major != ABI_MAJOR {
            set_last(format!(
                "E_ABI: incompatible ABI: this shell was built for gnoblin std ABI {want_major}.{built_minor}; the installed libgnoblin-std is {ABI_MAJOR}.{ABI_MINOR}"
            ));
            return abi::E_ABI;
        }
        // SAFETY: the caller guarantees `out` is null or writable.
        let Some(out) = (unsafe { out.as_mut() }) else {
            set_last("E_ERR: gnoblin_std_get_api: a null output pointer".to_owned());
            return abi::E_ERR;
        };
        *out = &raw const API;
        abi::E_OK
    }))
    .unwrap_or(abi::E_PANIC)
}

/// `gnoblin_std_last_error(buf, cap)`: copies this thread's last std error text into `buf`, NUL terminated and truncated to `cap`
/// bytes, and returns the text's full length. Works with a null `buf` or a zero `cap` to ask only for the length.
///
/// # Safety
///
/// `buf` must be null or point at `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gnoblin_std_last_error(buf: *mut u8, cap: usize) -> usize {
    catch_unwind(AssertUnwindSafe(|| {
        let text = last_text();
        if !buf.is_null() && cap > 0 {
            let kept = text.len().min(cap - 1);
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes; `kept + 1 <= cap`, and the text is a live local that
            // cannot overlap the caller's buffer.
            unsafe {
                core::ptr::copy_nonoverlapping(text.as_ptr(), buf, kept);
                buf.add(kept).write(0);
            }
        }
        text.len()
    }))
    .unwrap_or(0)
}
