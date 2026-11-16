//! Panic containment and the per-thread last-error text.
//!
//! Rust aborts the process when a panic leaves an `extern "C"` function, so every function the core exports, and every
//! call the host makes into shell code, runs inside [`guard`], [`contained`] or `catch_unwind`: a panic becomes an error
//! code, a log line and a last-error text, and the process carries on (`design/modular-framework.md` section 5.7).

use std::any::Any;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::types::CONSTS;

use crate::log;

/// Panics caught at the boundary, on either side.
pub static PANICS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Records `text` as this thread's last error.
pub fn set_error(text: impl Into<String>) {
    LAST.with(|last| *last.borrow_mut() = text.into());
}

/// Records the last error for `code`. The text starts with the code's symbolic name and a colon (`E_RETIRED: ...`), so a
/// caller whose function returns a handle, and so has no code to read, can still tell what failed.
pub fn set_code_error(code: i32, text: impl AsRef<str>) {
    set_error(format!("{}: {}", code_name(code), text.as_ref()));
}

/// This thread's last error text.
pub fn last_error() -> String {
    LAST.with(|last| last.borrow().clone())
}

/// The symbolic name of an error code.
pub fn code_name(code: i32) -> &'static str {
    CONSTS
        .iter()
        .find(|constant| constant.name.starts_with("E_") && constant.value == i128::from(code))
        .map_or("E_ERR", |constant| constant.name)
}

/// The text of a caught panic payload.
pub fn payload_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with a non-text payload".to_owned())
}

/// Runs `body`; a panic in it is logged, recorded as the last error and counted, and `on_panic` is returned instead.
pub fn guard<T>(on_panic: T, what: &str, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let text = format!("panic in core export {what}: {}", payload_text(payload.as_ref()));
            PANICS.fetch_add(1, Ordering::Relaxed);
            log::error!(what = what, "{text}");
            set_error(text);
            on_panic
        }
    }
}

/// Calls a function a shell gave the host, containing a panic that unwinds out of it: the panic is counted and logged
/// and `on_panic` is returned instead.
pub fn contained<T>(on_panic: T, what: &str, call: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(value) => value,
        Err(payload) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            log::error!(
                what = what,
                "{what} unwound across the ABI: {}",
                payload_text(payload.as_ref())
            );
            on_panic
        }
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_core_abi as abi;

    use super::*;

    #[test]
    fn a_panic_becomes_the_fallback_and_a_last_error() {
        let before = PANICS.load(Ordering::Relaxed);
        let rc = guard(abi::E_PANIC, "selftest", || panic!("inside the guard"));
        assert_eq!(rc, abi::E_PANIC);
        assert_eq!(last_error(), "panic in core export selftest: inside the guard");
        assert!(PANICS.load(Ordering::Relaxed) > before);
    }

    #[test]
    fn a_value_passes_through_untouched() {
        assert_eq!(guard(-1, "selftest", || 7), 7);
    }

    #[test]
    fn a_contained_call_returns_the_fallback_and_counts() {
        let before = PANICS.load(Ordering::Relaxed);
        assert_eq!(contained(-5, "selftest", || panic!("inside the call")), -5);
        assert_eq!(contained(-5, "selftest", || 9), 9);
        assert!(PANICS.load(Ordering::Relaxed) > before);
    }

    #[test]
    fn code_errors_start_with_the_symbolic_name() {
        set_code_error(abi::E_RETIRED, "generation 3 has been retired");
        assert_eq!(last_error(), "E_RETIRED: generation 3 has been retired");
        assert_eq!(code_name(abi::E_THREAD), "E_THREAD");
        assert_eq!(code_name(12345), "E_ERR");
    }

    #[test]
    fn the_last_error_is_per_thread() {
        set_error("main thread text");
        std::thread::spawn(|| assert_eq!(last_error(), "")).join().unwrap();
        assert_eq!(last_error(), "main thread text");
    }
}
