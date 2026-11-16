//! Owned buffers and values the core lends to a shell and takes back.
//!
//! The shell and the core are separate libraries with separate allocator state, so the core never frees shell memory and
//! the shell never frees core memory (`design/modular-framework.md` section 5.2, rule 4). A function that returns owned
//! data hands out a `GnoblinBuf` or an owned `GnoblinValue`, and the shell gives it back through `buf_free` or
//! `value_free`. Every allocation lent out is recorded here by address, so a pointer the shell forges, or frees twice, is
//! refused (`E_STALE`) instead of reaching the allocator.
#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use gnoblin_core_abi::{self as abi, GnoblinBuf, GnoblinValue};

use crate::guard::set_code_error;

/// Address of each live lent allocation, and its capacity.
static LIVE: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

fn live() -> MutexGuard<'static, BTreeMap<usize, usize>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Lends `bytes` to the shell. The buffer is freed with `buf_free`.
pub fn lend_bytes(mut bytes: Vec<u8>) -> GnoblinBuf {
    bytes.shrink_to_fit();
    let (len, cap) = (bytes.len(), bytes.capacity());
    if cap == 0 {
        return GnoblinBuf {
            ptr: core::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
    }
    let ptr = bytes.as_mut_ptr();
    std::mem::forget(bytes);
    live().insert(ptr as usize, cap);
    GnoblinBuf { ptr, len, cap }
}

/// Lends `text` to the shell as an owned `VALUE_TEXT`. The value is freed with `value_free`.
pub fn lend_text(text: String) -> GnoblinValue {
    let buf = lend_bytes(text.into_bytes());
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind: abi::VALUE_TEXT,
        flags: if buf.ptr.is_null() { 0 } else { abi::VALUE_OWNED },
        reserved: 0,
        bits: 0,
        ptr: buf.ptr.cast_const().cast(),
        len: buf.len,
    }
}

/// Takes an allocation back: removes it from the live set and frees it. `false` when the address was never lent or was
/// already returned.
fn release(ptr: *mut u8) -> bool {
    let Some(cap) = live().remove(&(ptr as usize)) else {
        return false;
    };
    // SAFETY: `ptr` and `cap` are exactly what `lend_bytes` took from a `Vec<u8>` and forgot, and the entry was just
    // removed from the live set, so this is the only reconstruction. Length zero because `u8` has no destructor and the
    // caller's `len` is untrusted.
    drop(unsafe { Vec::from_raw_parts(ptr, 0, cap) });
    true
}

/// `buf_free`.
pub(crate) fn free_buf(buf: &mut GnoblinBuf) -> i32 {
    if buf.ptr.is_null() {
        return abi::E_OK;
    }
    if !release(buf.ptr) {
        set_code_error(abi::E_STALE, "buf_free: not a buffer the core lent, or already freed");
        return abi::E_STALE;
    }
    buf.ptr = core::ptr::null_mut();
    buf.len = 0;
    buf.cap = 0;
    abi::E_OK
}

/// `value_free`.
pub(crate) fn free_value(value: &mut GnoblinValue) -> i32 {
    if value.flags & abi::VALUE_OWNED != 0 && !value.ptr.is_null() {
        if !release(value.ptr.cast::<u8>().cast_mut()) {
            set_code_error(abi::E_STALE, "value_free: not a value the core lent, or already freed");
            return abi::E_STALE;
        }
        value.ptr = core::ptr::null();
        value.len = 0;
        value.flags &= !abi::VALUE_OWNED;
    }
    abi::E_OK
}

/// How many lent allocations are outstanding.
#[cfg(test)]
pub(crate) fn outstanding() -> usize {
    live().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests share one allocation table and the allocator reuses addresses, so they run one at a time.
    fn serial() -> MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn a_lent_buffer_is_freed_once() {
        let _serial = serial();
        let mut buf = lend_bytes(b"hello".to_vec());
        assert_eq!(buf.len, 5);
        let address = buf.ptr;
        assert_eq!(free_buf(&mut buf), abi::E_OK);
        assert!(buf.ptr.is_null());
        // A second free of the same address is refused, not forwarded to the allocator.
        let mut again = GnoblinBuf {
            ptr: address,
            len: 5,
            cap: 5,
        };
        assert_eq!(free_buf(&mut again), abi::E_STALE);
    }

    #[test]
    fn a_forged_pointer_is_refused() {
        let _serial = serial();
        let mut local = [0u8; 4];
        let mut forged = GnoblinBuf {
            ptr: local.as_mut_ptr(),
            len: 4,
            cap: 4,
        };
        assert_eq!(free_buf(&mut forged), abi::E_STALE);
    }

    #[test]
    fn an_empty_buffer_is_null_and_freeing_it_is_fine() {
        let _serial = serial();
        let mut buf = lend_bytes(Vec::new());
        assert!(buf.ptr.is_null());
        assert_eq!(free_buf(&mut buf), abi::E_OK);
    }

    #[test]
    fn a_lent_text_value_round_trips_and_is_cleared_by_value_free() {
        let _serial = serial();
        let before = outstanding();
        let mut value = lend_text("owned text".to_owned());
        assert_eq!(value.kind, abi::VALUE_TEXT);
        assert_eq!(value.flags & abi::VALUE_OWNED, abi::VALUE_OWNED);
        assert_eq!(value.len, 10);
        assert!(outstanding() > before);
        assert_eq!(free_value(&mut value), abi::E_OK);
        assert!(value.ptr.is_null());
        assert_eq!(value.flags & abi::VALUE_OWNED, 0);
        // Freeing a value that is no longer owned does nothing.
        assert_eq!(free_value(&mut value), abi::E_OK);
    }

    #[test]
    fn a_value_that_claims_ownership_of_foreign_memory_is_refused() {
        let _serial = serial();
        let text = *b"not ours";
        let mut value = GnoblinValue {
            size: size_of::<GnoblinValue>() as u32,
            kind: abi::VALUE_TEXT,
            flags: abi::VALUE_OWNED,
            reserved: 0,
            bits: 0,
            ptr: text.as_ptr().cast(),
            len: text.len(),
        };
        assert_eq!(free_value(&mut value), abi::E_STALE);
    }
}
