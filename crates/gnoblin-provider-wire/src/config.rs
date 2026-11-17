//! The configuration observer across the ABI: [`export_config`] on std's side and [`ForeignConfig`] on the core's.
//!
//! The core owns the accepted document and what it means to the mechanisms (a zoom, a colour matrix, a flash); what a section
//! of it means to the desktop's policy is std's. The observer is the one call between them: the core hands over the document as
//! JSON, and the provider answers with `setting_set` calls. One call, one string, so the observer is the smallest vtable.

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{self as abi, GnoblinConfigVtable, GnoblinDropFn, GnoblinStr};

use crate::{Exported, Report, guarded};

/// What an observer is: a function of the accepted document that returns why it could not apply it.
pub type Observe = Box<dyn FnMut(&str) -> Result<(), String>>;

/// What `user` points at.
struct ConfigHost {
    observe: Observe,
}

static VTABLE: GnoblinConfigVtable = GnoblinConfigVtable {
    size: size_of::<GnoblinConfigVtable>() as u32,
    reserved: 0,
    apply: host_apply,
};

/// Turns `observe` into a provider the core can register with `provider_register(PROVIDER_CONFIG)`.
#[must_use]
pub fn export_config(observe: Observe) -> Exported<GnoblinConfigVtable> {
    Exported {
        vtable: &VTABLE,
        user: Box::into_raw(Box::new(ConfigHost { observe })).cast(),
        drop_user: host_drop,
    }
}

unsafe extern "C" fn host_drop(user: *mut c_void) {
    // SAFETY: the core calls this once, with the `user` pointer `export_config` made.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(user.cast::<ConfigHost>()) })
    }));
}

unsafe extern "C" fn host_apply(user: *mut c_void, document: GnoblinStr) -> i32 {
    guarded(|| {
        let bytes = if document.ptr.is_null() || document.len == 0 {
            &[][..]
        } else {
            // SAFETY: the vtable's contract: the document is readable for the call.
            unsafe { std::slice::from_raw_parts(document.ptr, document.len) }
        };
        let Ok(text) = std::str::from_utf8(bytes) else {
            return abi::E_ERR;
        };
        // SAFETY: `user` is the pointer `export_config` made, used by one thread at a time.
        let host = unsafe { &mut *user.cast::<ConfigHost>() };
        match (host.observe)(text) {
            Ok(()) => abi::E_OK,
            Err(_) => abi::E_ERR,
        }
    })
}

/// A configuration observer registered through the ABI.
pub struct ForeignConfig {
    vtable: GnoblinConfigVtable,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    report: Report,
}

// SAFETY: `ForeignConfig` holds a raw `user` pointer and a function pointer. The registration's contract says `apply` runs on
// one thread at a time, and the registry that holds this serialises its calls behind a lock.
unsafe impl Send for ForeignConfig {}

impl Drop for ForeignConfig {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: `drop_user` and `user` came from one registration; this is the one call it is owed.
            unsafe { drop_user(self.user) };
        }
    }
}

impl ForeignConfig {
    /// Wraps the observer a registration named. `report` says what the observer could not apply. The wrapper does not own
    /// `user` yet: until [`ForeignConfig::adopt`] it is the registrant's.
    ///
    /// # Errors
    /// The vtable is shorter than this build's.
    ///
    /// # Safety
    /// `vtable` points at a vtable whose `apply` may be called with `user` from one thread at a time for as long as the
    /// result lives.
    pub unsafe fn new(vtable: *const GnoblinConfigVtable, user: *mut c_void, report: Report) -> Result<Self, String> {
        // SAFETY: the caller guarantees the pointer is null or valid for `size` bytes.
        let Some(table) = (unsafe { vtable.as_ref() }) else {
            return Err("a null configuration vtable".to_owned());
        };
        if (table.size as usize) < size_of::<GnoblinConfigVtable>() {
            return Err(format!(
                "the configuration vtable is {} bytes; this core needs {}",
                table.size,
                size_of::<GnoblinConfigVtable>()
            ));
        }
        Ok(Self {
            vtable: *table,
            user,
            drop_user: None,
            report,
        })
    }

    /// Takes ownership of `user`: `drop_user` runs on it once, when the observer ends.
    pub fn adopt(&mut self, drop_user: Option<GnoblinDropFn>) {
        self.drop_user = drop_user;
    }

    /// Tells the observer the accepted document. A failure is reported and does not stop the session: the settings the
    /// observer did not reach keep their last values.
    pub fn apply(&mut self, document: &str) {
        // SAFETY: the registration's contract; the borrowed document lives for the call.
        let rc = unsafe { (self.vtable.apply)(self.user, GnoblinStr::borrowed(document)) };
        if rc != abi::E_OK {
            (self.report)(&format!(
                "the configuration observer could not apply the accepted document (answered {rc}); its settings keep their last values"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn the_document_reaches_the_observer_and_a_refusal_is_reported() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let exported = export_config(Box::new(move |document| {
            sink.borrow_mut().push(document.to_owned());
            if document.contains("bad") {
                Err("no".to_owned())
            } else {
                Ok(())
            }
        }));
        let lines = Arc::new(Mutex::new(Vec::new()));
        let said = Arc::clone(&lines);
        let report: Report = Box::new(move |line| said.lock().expect("lock").push(line.to_owned()));
        // SAFETY: the vtable and state are `export_config`'s; `drop_user` frees the state once.
        let mut foreign = unsafe { ForeignConfig::new(exported.vtable, exported.user, report) }.expect("wraps");
        foreign.adopt(Some(exported.drop_user));
        foreign.apply("{\"a\":1}");
        assert!(lines.lock().expect("lock").is_empty());
        foreign.apply("{\"bad\":1}");
        assert_eq!(*seen.borrow(), ["{\"a\":1}", "{\"bad\":1}"]);
        assert_eq!(lines.lock().expect("lock").len(), 1);
    }

    #[test]
    fn a_panicking_observer_is_an_error_not_an_abort() {
        let exported = export_config(Box::new(|_| panic!("boom")));
        let lines = Arc::new(Mutex::new(Vec::new()));
        let said = Arc::clone(&lines);
        let report: Report = Box::new(move |line| said.lock().expect("lock").push(line.to_owned()));
        // SAFETY: as above.
        let mut foreign = unsafe { ForeignConfig::new(exported.vtable, exported.user, report) }.expect("wraps");
        foreign.adopt(Some(exported.drop_user));
        foreign.apply("{}");
        assert_eq!(lines.lock().expect("lock").len(), 1);
    }
}
