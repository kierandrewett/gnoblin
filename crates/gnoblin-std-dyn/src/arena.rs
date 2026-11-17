//! What one call keeps alive: the text and records its arguments point at, and the objects the library may adopt.

use core::ffi::c_void;
use std::any::Any;
use std::rc::Rc;

use gnoblin_core_abi::GnoblinStr;

use crate::{Callable, drop_user};

/// Owns what a call's `repr(C)` arguments point into, for the length of the call.
///
/// Text is copied into a `String` whose heap buffer does not move when the arena grows, and a record is boxed, so a pointer
/// made here is valid until the arena drops. A script object given to the library as a `user` is boxed too; the library adopts
/// it when the call succeeds ([`Arena::adopt`]) and frees it through `drop_user` later. When the call fails the library has not
/// adopted it, so dropping the arena frees it.
#[derive(Default)]
pub struct Arena {
    texts: Vec<String>,
    kept: Vec<Box<dyn Any>>,
    users: Vec<*mut c_void>,
    adopted: bool,
}

impl Arena {
    /// A fresh arena.
    #[must_use]
    pub fn new() -> Arena {
        Arena::default()
    }

    /// Copies `text` and lends it as the ABI's string.
    pub fn text(&mut self, text: &str) -> GnoblinStr {
        self.texts.push(text.to_owned());
        let stored = &self.texts[self.texts.len() - 1];
        GnoblinStr {
            ptr: stored.as_ptr(),
            len: stored.len(),
        }
    }

    /// Keeps `value` for the call and returns a pointer to it.
    pub fn keep<T: 'static>(&mut self, value: T) -> *const T {
        let boxed = Box::new(value);
        let pointer: *const T = &raw const *boxed;
        self.kept.push(boxed);
        pointer
    }

    /// Boxes a script object as the `user` of a registration or a layout.
    pub fn user(&mut self, object: Rc<dyn Callable>) -> *mut c_void {
        let raw = Box::into_raw(Box::new(object)).cast::<c_void>();
        self.users.push(raw);
        raw
    }

    /// The call succeeded and the library holds every `user` made here: dropping the arena frees none of them.
    pub fn adopt(&mut self) {
        self.adopted = true;
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        if !self.adopted {
            for user in self.users.drain(..) {
                // SAFETY: `user` came from `Arena::user` and the library did not adopt it, so this is its only owner.
                unsafe { drop_user(user) };
            }
        }
    }
}
