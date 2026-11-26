//! The write stage: holding the writes an object's setters make, so a library can send them together.
//!
//! The object model's setters (`client.opacity = 0.5`) write a node property of the core's own. A library that stages the writes of a
//! callback (the transition engine of `libgnoblin-std` does, to send them as timed writes of one `tween`) installs a write stage with
//! [`Gnoblin::stage_writes`]: the core offers every such write to it first, and a write it keeps is not sent. The writes the node API
//! makes ([`Node::set`](crate::Node::set)) are never offered, so what the library staged goes out through them.
//!
//! A core built before the function has no write stage: [`Gnoblin::stage_writes`] fails with [`ErrorKind::Unsupported`](crate::ErrorKind)
//! and the writes of an object's setters go out as they always did.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use gnoblin_core_abi::{self as abi, GnoblinApi, GnoblinNode, GnoblinStr, GnoblinValue};

use crate::error::{CoreError, ErrorKind, Result, check};
use crate::gnoblin::Gnoblin;
use crate::owned::{Lent, Value};
use crate::value::NodeId;

/// Offered the write of a property: true when it keeps it.
type Stage = Box<dyn Fn(NodeId, &str, &Value) -> bool>;
/// Asked for the value it holds for a property, if it holds one.
type Peek = Box<dyn Fn(NodeId, &str) -> Option<Value>>;

/// What the core's trampolines call: the two closures and the value last lent to a read.
struct StageCallbacks {
    stage: Stage,
    peek: Peek,
    /// The value the last `peek` answered with, which the core reads before its next call.
    lent: RefCell<Option<Lent>>,
}

/// An installed write stage. Dropping it clears the hook, so every path that ends the staging (a commit, an abort, a failed callback, a
/// generation that goes away) leaves the core's setters writing as they did.
pub struct WriteStage {
    g: Gnoblin,
    hooks: Box<StageCallbacks>,
}

impl std::fmt::Debug for WriteStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteStage").finish_non_exhaustive()
    }
}

impl WriteStage {
    fn user(&self) -> *mut c_void {
        core::ptr::from_ref::<StageCallbacks>(&self.hooks).cast_mut().cast()
    }
}

impl Drop for WriteStage {
    fn drop(&mut self) {
        // SAFETY: clearing takes no value; the core compares `user` with the hook it holds and leaves another caller's alone.
        unsafe { (self.g.api().set_write_stage)(self.g.ctx(), None, None, self.user()) };
    }
}

/// The text of a borrowed key, or `None` for text that is not UTF-8.
///
/// # Safety
///
/// `key` must point at `len` readable bytes, or have length zero.
unsafe fn key_of<'a>(key: GnoblinStr) -> Option<&'a str> {
    if key.len == 0 {
        return Some("");
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    std::str::from_utf8(unsafe { core::slice::from_raw_parts(key.ptr, key.len) }).ok()
}

unsafe extern "C" fn stage_write(
    user: *mut c_void,
    node: GnoblinNode,
    key: GnoblinStr,
    value: *const GnoblinValue,
) -> i32 {
    // SAFETY: `user` is the `StageCallbacks` the `WriteStage` holds until it clears the hook, the key and the value are
    // borrowed for the call.
    let (hooks, key, value) = unsafe {
        (
            &*user.cast::<StageCallbacks>(),
            key_of(key),
            value.as_ref().and_then(|raw| Value::from_raw(raw)),
        )
    };
    let (Some(key), Some(value)) = (key, value) else {
        return 0;
    };
    catch_unwind(AssertUnwindSafe(|| (hooks.stage)(NodeId(node), key, &value))).map_or(0, i32::from)
}

unsafe extern "C" fn peek_write(user: *mut c_void, node: GnoblinNode, key: GnoblinStr, out: *mut GnoblinValue) -> i32 {
    // SAFETY: as for `stage_write`; `out` is a writable value.
    let (hooks, key) = unsafe { (&*user.cast::<StageCallbacks>(), key_of(key)) };
    let Some(key) = key else {
        return 0;
    };
    let Ok(Some(value)) = catch_unwind(AssertUnwindSafe(|| (hooks.peek)(NodeId(node), key))) else {
        return 0;
    };
    let lent = Lent::new(value);
    // SAFETY: the core passes a writable value; what it points at is owned by `lent`, which is kept until the next peek.
    unsafe { *out = lent.raw() };
    *hooks.lent.borrow_mut() = Some(lent);
    1
}

impl Gnoblin {
    /// Installs a write stage on this thread: until the returned guard is dropped, every write an object's setter of the core makes to a
    /// node property (a window's `opacity`, `x`, `rotation`) is first offered to `stage(node, key, value)`, and the core does not send it
    /// when it returns true. Reads of those properties ask `peek(node, key)` first and answer with the value it returns. A timed write
    /// is never offered. A second stage replaces the first.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unsupported`](crate::ErrorKind) on a core built before the write stage: its setters send every write.
    pub fn stage_writes(
        &self,
        stage: impl Fn(NodeId, &str, &Value) -> bool + 'static,
        peek: impl Fn(NodeId, &str) -> Option<Value> + 'static,
    ) -> Result<WriteStage> {
        if !supported(self.api().size) {
            return Err(CoreError::new(
                ErrorKind::Unsupported,
                "the installed core predates the write stage, so the writes of an object's setters cannot be staged",
            ));
        }
        let guard = WriteStage {
            g: self.clone(),
            hooks: Box::new(StageCallbacks {
                stage: Box::new(stage),
                peek: Box::new(peek),
                lent: RefCell::new(None),
            }),
        };
        // SAFETY: `user` stays valid until the guard drops, which clears the hook; the trampolines read it as the `StageCallbacks` it is.
        check(unsafe { (self.api().set_write_stage)(self.ctx(), Some(stage_write), Some(peek_write), guard.user()) })?;
        Ok(guard)
    }
}

/// Whether a core whose table is `table_size` bytes has the write stage: the table ends after its field.
fn supported(table_size: u32) -> bool {
    abi::table_has(
        table_size,
        core::mem::offset_of!(GnoblinApi, set_write_stage) + size_of::<usize>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_core_whose_table_ends_before_the_field_has_no_write_stage() {
        let field = core::mem::offset_of!(GnoblinApi, set_write_stage);
        // The table of the first ABI 1.0 cores ended where this field begins: `object_api` was the last function.
        assert!(!supported(field as u32), "a table of the older size");
        assert!(!supported(field as u32 + 7), "a table that holds part of the field");
        assert!(
            supported((field + size_of::<usize>()) as u32),
            "a table that ends with the field"
        );
        assert!(supported(size_of::<GnoblinApi>() as u32), "this SDK's own table");
    }

    #[test]
    fn a_peeked_value_stays_valid_while_it_is_lent() {
        let lent = Lent::new(Value::List(vec![0.0, 0.5, 1.0]));
        let raw = lent.raw();
        // SAFETY: the raw value points into `lent`, which is alive.
        assert_eq!(unsafe { Value::from_raw(&raw) }, Some(Value::List(vec![0.0, 0.5, 1.0])));
        let lent = Lent::new(Value::Text("ease".to_owned()));
        let raw = lent.raw();
        // SAFETY: as above.
        assert_eq!(unsafe { Value::from_raw(&raw) }, Some(Value::Text("ease".to_owned())));
    }
}
