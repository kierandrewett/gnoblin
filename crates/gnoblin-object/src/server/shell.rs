//! An object the shell implements: the methods of an interface class that run in the shell's language.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{self as abi, GnoblinDropFn, GnoblinInvokeFn, GnoblinValue};

use crate::error::{Fail, Res};
use crate::value::{MemberId, Val};
use crate::wire::{Arena, from_raw, none_value, to_raw};

/// What `implement` adopted: the shell's `invoke`, its `user`, and which members it defined.
pub struct ShellImpl {
    invoke: GnoblinInvokeFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    implemented: Vec<u64>,
}

impl ShellImpl {
    /// Takes over a shell's implementation. Call it only when nothing can fail any more: the shell keeps `user` until then.
    pub(crate) fn adopt(
        invoke: GnoblinInvokeFn,
        user: *mut c_void,
        drop_user: Option<GnoblinDropFn>,
        implemented: Vec<u64>,
    ) -> ShellImpl {
        ShellImpl {
            invoke,
            user,
            drop_user,
            implemented,
        }
    }

    /// Whether the shell defined `member`. A member it did not is never called across the ABI: the library runs its own default.
    pub fn has(&self, member: MemberId) -> bool {
        let word = (member / 64) as usize;
        self.implemented
            .get(word)
            .is_some_and(|bits| bits & (1_u64 << (member % 64)) != 0)
    }

    /// Runs the shell's method (or reads its property) `member`. The caller checks [`ShellImpl::has`] first and runs this from the
    /// turn, never inside a call of the library.
    ///
    /// # Errors
    ///
    /// The shell failed, or a script raised.
    pub fn call(&self, member: MemberId, args: &[Val]) -> Res<Val> {
        let arena = Arena::new();
        let raws: Vec<GnoblinValue> = args.iter().map(|arg| to_raw(arg, &arena)).collect();
        let mut out = none_value();
        // SAFETY: `invoke` and `user` are what the shell gave `implement`, kept alive until `drop`; the arguments and the arena outlive
        // the call.
        let code = catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.invoke)(self.user, member, raws.as_ptr(), raws.len(), &raw mut out)
        }))
        .unwrap_or(abi::E_PANIC);
        arena.finish(false);
        if code != abi::E_OK {
            return Err(Fail::new(code, "an implemented method of the shell failed"));
        }
        // SAFETY: the shell wrote `out`, and it stays valid until the next call into the shell, which comes after this copy.
        unsafe { from_raw(&out, false) }
    }
}

impl Drop for ShellImpl {
    fn drop(&mut self) {
        if let Some(drop_user) = self.drop_user.take() {
            // SAFETY: the shell gave `user` with its `drop_user` and the library adopted it; this is the one call.
            let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop_user(self.user) }));
        }
    }
}
