//! The write stage hook: how a library built on the core holds the writes of an object's setters.
//!
//! The object model (`gnoblin.model`) writes a node property when a shell sets `client.opacity`. A library that wants those writes
//! to travel together (the transition engine of `libgnoblin-std` stages the writes of a `requestTransition` callback and sends them as
//! timed writes of one `tween`) cannot reach into this one, so it installs a hook (`GnoblinApi::set_write_stage`). The classes that write
//! a property of a node through their setters offer the write to the hook first ([`offer`]) and send it only when the hook did not keep
//! it; reads of the same properties ask the hook for the value it holds ([`peek`]), so the callback reads back what it wrote.
//!
//! One hook per thread, the shell thread's. Nothing here is offered a write the node API makes (`write_set`): the hook's owner sends
//! what it staged through that, and a write that goes through the hook again would never leave it.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::Cell;

use gnoblin_core_abi::{GnoblinStr, GnoblinWriteStageFn, GnoblinWriteStagePeekFn};
use gnoblin_tree_client::tree_mirror::MirrorValue;

use crate::values::{OwnedValue, RawScope, blank, read_value};

/// What `set_write_stage` installed.
#[derive(Clone, Copy)]
struct WriteStageHook {
    stage: GnoblinWriteStageFn,
    peek: Option<GnoblinWriteStagePeekFn>,
    user: *mut c_void,
}

thread_local! {
    static HOOK: Cell<Option<WriteStageHook>> = const { Cell::new(None) };
}

/// Installs the hook, replacing any other.
pub(crate) fn install(stage: GnoblinWriteStageFn, peek: Option<GnoblinWriteStagePeekFn>, user: *mut c_void) {
    HOOK.with(|hook| hook.set(Some(WriteStageHook { stage, peek, user })));
}

/// Clears the hook `user` installed. A hook another caller installed since is left alone.
pub(crate) fn clear(user: *mut c_void) {
    HOOK.with(|hook| {
        if hook.get().is_some_and(|held| held.user == user) {
            hook.set(None);
        }
    });
}

/// Offers the write of `key` of `node` to the hook: true when the hook kept it and the caller sends nothing.
pub(crate) fn offer(node: u64, key: &str, value: &MirrorValue) -> bool {
    let Some(hook) = HOOK.with(Cell::get) else {
        return false;
    };
    let Some(owned) = OwnedValue::from_mirror(value) else {
        return false;
    };
    let mut scope = RawScope::default();
    let raw = scope.raw(&owned);
    // SAFETY: the hook's owner keeps `user` alive until it clears the hook, `raw` and what it points into live for the call, and the
    // key is borrowed text.
    unsafe { (hook.stage)(hook.user, node, GnoblinStr::borrowed(key), &raw) != 0 }
}

/// The value the hook holds for `key` of `node`, if it holds one.
pub(crate) fn peek(node: u64, key: &str) -> Option<OwnedValue> {
    let hook = HOOK.with(Cell::get)?;
    let peek = hook.peek?;
    let mut out = blank();
    // SAFETY: as for `offer`; `out` is a blank value the callee fills.
    if unsafe { peek(hook.user, node, GnoblinStr::borrowed(key), &mut out) } == 0 {
        return None;
    }
    // SAFETY: the callee keeps what `out` points at valid until its next call, and `read_value` copies it.
    let held = unsafe { read_value(&out, out.kind) }.ok()?;
    OwnedValue::from_mirror(&held)
}
