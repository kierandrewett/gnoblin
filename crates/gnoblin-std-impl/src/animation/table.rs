//! The animation functions of the std table: easing, transitions and their controls.
//!
//! A transition is staged between `transition_begin` and `transition_commit`: the shell's closure runs in between (outside any borrow
//! of the library, so it may call any table function), and its writes, and the library's own layout and style passes, are held. Commit
//! runs those passes against the staged state, ends staging and sends what was held as timed writes.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinDropFn, GnoblinNode, GnoblinStr};
use gnoblin_std_abi as sabi;
use gnoblin_std_abi::{
    GnoblinStdEventFn, GnoblinStdProp, GnoblinStdReg, GnoblinStdTransition, GnoblinStdTransitionInfo,
    GnoblinStdTransitionOptions,
};

use super::easing::Easing;
use super::engine::Options;
use super::staging::{self, Prop};
use crate::error::{Fail, Res};
use crate::state::{self, RegData};
// The animation functions need no desktop: they open the library's model alone (`state::lookup_quiet`).
use crate::table::{
    call, call_reg, fill_out, lent, mutate_quiet as mutate, put, quiet_of as std_of, read_ext, read_quiet as read, utf8,
};
use crate::turn::schedule;

thread_local! {
    /// The text a property read lent, valid until the next call on this thread.
    static LENT_PROP: RefCell<String> = const { RefCell::new(String::new()) };
}

/// A property value the caller passed.
///
/// # Safety
///
/// `raw` must be null or point at a `GnoblinStdProp` whose `text` is readable.
pub(crate) unsafe fn prop_in(raw: *const GnoblinStdProp) -> Res<Prop> {
    // SAFETY: forwarded from the caller; every field is an integer, a float or a pointer.
    let raw = unsafe { read_ext(raw, "a property value") }?;
    Ok(match raw.kind {
        sabi::STD_PROP_NUMBER => Prop::Number(raw.a),
        sabi::STD_PROP_BOOL => Prop::Bool(raw.a != 0.0),
        sabi::STD_PROP_TRIPLE => Prop::Triple([raw.a, raw.b, raw.c]),
        sabi::STD_PROP_NODE => Prop::Node(raw.node),
        sabi::STD_PROP_TEXT => {
            // SAFETY: the caller guarantees the text is readable for the call.
            Prop::Text(unsafe { utf8(raw.text.ptr, raw.text.len) }?.to_owned())
        }
        other => return Err(Fail::type_error(format!("{other} is not a kind of property value"))),
    })
}

/// A property value the library hands back; text is lent until the next call on this thread.
pub(crate) fn prop_struct(value: Option<&Prop>) -> GnoblinStdProp {
    let mut out = GnoblinStdProp {
        size: size_of::<GnoblinStdProp>() as u32,
        kind: sabi::STD_PROP_UNSET,
        a: 0.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        text: GnoblinStr::EMPTY,
        node: 0,
    };
    match value {
        None => {}
        Some(Prop::Number(number)) => {
            out.kind = sabi::STD_PROP_NUMBER;
            out.a = *number;
        }
        Some(Prop::Bool(flag)) => {
            out.kind = sabi::STD_PROP_BOOL;
            out.a = f64::from(u8::from(*flag));
        }
        Some(Prop::Triple([a, b, c])) => {
            out.kind = sabi::STD_PROP_TRIPLE;
            (out.a, out.b, out.c) = (*a, *b, *c);
        }
        Some(Prop::Node(node)) => {
            out.kind = sabi::STD_PROP_NODE;
            out.node = *node;
        }
        Some(Prop::Text(text)) => {
            out.kind = sabi::STD_PROP_TEXT;
            LENT_PROP.with(|lent_text| {
                *lent_text.borrow_mut() = text.clone();
                out.text = lent(&lent_text.borrow());
            });
        }
    }
    out
}

/// The options the caller passed, read: the checks are made where the options are used, which names the call in the error.
///
/// # Safety
///
/// `options` must be null or point at a `GnoblinStdTransitionOptions`.
unsafe fn options_in(options: *const GnoblinStdTransitionOptions) -> Res<GnoblinStdTransitionOptions> {
    // SAFETY: forwarded from the caller.
    unsafe { read_ext(options, "transition options") }
}

pub(crate) unsafe extern "C" fn easing_sample(_ctx: GnoblinCtx, easing: u32, t: f64, out: *mut f64) -> i32 {
    call("easing_sample", || {
        let easing = Easing::from_abi(easing).ok_or_else(|| Fail::type_error(format!("{easing} is not an easing")))?;
        // SAFETY: the caller guarantees `out` is null or a writable double.
        unsafe { put(out, easing.sample(t)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_begin(ctx: GnoblinCtx, options: *const GnoblinStdTransitionOptions) -> i32 {
    call("transition_begin", || {
        // SAFETY: the caller guarantees `options` is null or a readable struct.
        let options = unsafe { options_in(options) }?;
        mutate(ctx, |st| st.transition_begin(&options))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_set(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
    value: *const GnoblinStdProp,
) -> i32 {
    call("transition_set", || {
        // SAFETY: the caller guarantees `key` is `key_len` readable bytes and `value` a readable struct.
        let (key, value) = unsafe { (utf8(key, key_len)?, prop_in(value)?) };
        mutate(ctx, |st| st.transition_set(node, key, value))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_release(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
) -> i32 {
    call("transition_release", || {
        // SAFETY: the caller guarantees `key` is `key_len` readable bytes.
        let key = unsafe { utf8(key, key_len) }?;
        let std = std_of(ctx)?;
        std.st.borrow_mut().transition_release(node, key);
        // A transition that lost its last property is over: the next turn tells its listeners.
        schedule(&std);
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_get(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
    out: *mut GnoblinStdProp,
) -> i32 {
    call("transition_get", || {
        // SAFETY: the caller guarantees `key` is `key_len` readable bytes.
        let key = unsafe { utf8(key, key_len) }?;
        let value = read(ctx, |st| Ok(st.transition_get(node, key)))?;
        let raw = prop_struct(value.as_ref());
        // SAFETY: the caller guarantees `out` is a writable struct whose first field is its size.
        unsafe { fill_out(out, &raw, "transition_get") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_commit(ctx: GnoblinCtx, out: *mut GnoblinStdTransition) -> i32 {
    call("transition_commit", || {
        let std = std_of(ctx)?;
        if !staging::active() {
            return Err(Fail::stale(
                "no transition is being staged: call `transition_begin` first",
            ));
        }
        // The layout and style passes run against the staged state, so their writes are staged with the callback's.
        crate::turn::settle(&std);
        let entries = staging::end();
        let result = {
            let mut st = std.st.borrow_mut();
            match st.anim_take_pending() {
                Some(options) => st.transition_commit(entries, &options),
                None => Err(Fail::stale("no transition is being staged")),
            }
        };
        schedule(&std);
        // SAFETY: the caller guarantees `out` is null or a writable handle.
        unsafe { put(out, result?) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_abort(ctx: GnoblinCtx) -> i32 {
    call("transition_abort", || {
        let std = std_of(ctx)?;
        std.st.borrow_mut().transition_abort();
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_animate(
    ctx: GnoblinCtx,
    node: GnoblinNode,
    key: *const u8,
    key_len: usize,
    value: *const GnoblinStdProp,
    options: *const GnoblinStdTransitionOptions,
    out: *mut GnoblinStdTransition,
) -> i32 {
    call("transition_animate", || {
        // SAFETY: the caller guarantees the pointers are what the table documents.
        let (key, value, options) = unsafe { (utf8(key, key_len)?, prop_in(value)?, options_in(options)?) };
        let options = Options::check(&options, "gnoblin.transition")?;
        let made = mutate(ctx, |st| st.transition_animate(node, key, value, &options))?;
        // SAFETY: the caller guarantees `out` is null or a writable handle.
        unsafe { put(out, made.unwrap_or(0)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_control(
    ctx: GnoblinCtx,
    transition: GnoblinStdTransition,
    op: u32,
    value: f64,
) -> i32 {
    call("transition_control", || {
        mutate(ctx, |st| st.transition_control(transition, op, value))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_info(
    ctx: GnoblinCtx,
    transition: GnoblinStdTransition,
    out: *mut GnoblinStdTransitionInfo,
) -> i32 {
    call("transition_info", || {
        let info = read(ctx, |st| st.transition_info(transition))?;
        // SAFETY: the caller guarantees `out` is a writable struct whose first field is its size.
        unsafe { fill_out(out, &info, "transition_info") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn transition_finished(
    ctx: GnoblinCtx,
    transition: GnoblinStdTransition,
    cb: GnoblinStdEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinStdReg {
    call_reg("transition_finished", || {
        let id = state::next_id();
        mutate(ctx, |st| {
            let state = st
                .transition_state(transition)
                .ok_or_else(|| Fail::stale(format!("transition {transition} does not exist or ended long ago")))?;
            // Every check has passed: from here the library owns `user`.
            st.regs.insert(
                id,
                Rc::new(RegData::new(
                    transition,
                    gnoblin_std_abi::STD_EVENT_TRANSITION_FINISHED,
                    cb,
                    user,
                    drop_user,
                )),
            );
            if state != gnoblin_std_abi::STD_TRANSITION_RUNNING {
                st.notify_finished(transition, id, state == gnoblin_std_abi::STD_TRANSITION_CANCELLED);
            }
            Ok(())
        })?;
        Ok(id)
    })
}
