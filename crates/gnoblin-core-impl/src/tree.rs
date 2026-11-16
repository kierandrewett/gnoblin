//! The tree functions of the ABI: attach, read, write, call and flush.
//!
//! Each function here is the shell-thread operation behind one table entry. It checks the context, reaches the thread's
//! [`session`](crate::session) for the tree, and turns what the tree client says into an ABI code and a last-error text. Reads see the
//! tick's own writes; writes are queued in the tick and leave as one transaction at the end of the dispatch turn (or at `flush`),
//! under the refusal and resend rules of `gnoblin_tree_client::gate`.
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinAnswer, GnoblinAnswerFn, GnoblinCtx, GnoblinDropFn, GnoblinPending, GnoblinStr, GnoblinValue,
};
use gnoblin_shell_host::generation::CallbackOutcome;
use gnoblin_shell_host::handle::{KIND_PENDING, decode, encode};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxAnimation};

use crate::catalog;
use crate::events::ShellEvent;
use crate::guard::set_code_error;
use crate::registry::{self, Subscription, check_ctx};
use crate::session::{self, AttachMode};
use crate::values::{OwnedValue, lend_value, read_args, read_value};

/// Records a session failure and returns its code. A text that already names the function is kept as it is.
fn fail(what: &str, (code, text): (i32, String)) -> i32 {
    if text.starts_with(what) {
        set_code_error(code, text);
    } else {
        set_code_error(code, format!("{what}: {text}"));
    }
    code
}

/// `tree_attach`.
pub(crate) fn attach(ctx: GnoblinCtx, mode: u32) -> i32 {
    if let Err(code) = check_ctx(ctx, "tree_attach", true) {
        return code;
    }
    let mode = match mode {
        abi::ATTACH_HOLDER => AttachMode::Holder,
        abi::ATTACH_OBSERVER => AttachMode::Observer,
        other => {
            set_code_error(abi::E_ERR, format!("tree_attach: {other} is not an attach mode"));
            return abi::E_ERR;
        }
    };
    match session::with(|session| session.attach(mode)) {
        Ok(()) => abi::E_OK,
        Err(error) => fail("tree_attach", error),
    }
}

/// The class id of `node`, as the catalog knows it.
fn class_of(tree: &gnoblin_tree_client::session::TreeSession, node: u64) -> Result<u32, (i32, String)> {
    let name = tree
        .class_of(node)
        .ok_or_else(|| (abi::E_NO_SUCH_NODE, format!("no node {node}")))?;
    match catalog::tree_class_id(&name) {
        0 => Err((
            abi::E_NO_SUCH_MEMBER,
            format!("node {node} is of class `{name}`, which is not in the catalog"),
        )),
        id => Ok(id),
    }
}

/// `tree_root`.
pub(crate) fn root(ctx: GnoblinCtx) -> Result<u64, i32> {
    check_ctx(ctx, "tree_root", true)?;
    session::with(|session| session.tree_ref().map(|tree| tree.root())).map_err(|error| fail("tree_root", error))
}

/// `node_exists`: 1 or 0.
pub(crate) fn node_exists(ctx: GnoblinCtx, node: u64) -> i32 {
    if let Err(code) = check_ctx(ctx, "node_exists", true) {
        return code;
    }
    match session::with(|session| session.tree_ref().map(|tree| tree.exists(node))) {
        Ok(exists) => i32::from(exists),
        Err(error) => fail("node_exists", error),
    }
}

/// `node_class`: the class id, or zero with the last error set.
pub(crate) fn node_class(ctx: GnoblinCtx, node: u64) -> u32 {
    if check_ctx(ctx, "node_class", true).is_err() {
        return 0;
    }
    match session::with(|session| session.tree_ref().and_then(|tree| class_of(tree, node))) {
        Ok(class) => class,
        Err(error) => {
            fail("node_class", error);
            0
        }
    }
}

/// `node_parent`.
pub(crate) fn node_parent(ctx: GnoblinCtx, node: u64) -> Result<u64, i32> {
    check_ctx(ctx, "node_parent", true)?;
    session::with(|session| {
        session.tree_ref().and_then(|tree| {
            tree.parent_of(node)
                .ok_or_else(|| (abi::E_NO_SUCH_NODE, format!("no node {node}")))
        })
    })
    .map_err(|error| fail("node_parent", error))
}

/// `node_children`.
pub(crate) fn node_children(ctx: GnoblinCtx, node: u64) -> Result<Vec<u64>, i32> {
    check_ctx(ctx, "node_children", true)?;
    session::with(|session| {
        session.tree_ref().and_then(|tree| {
            tree.children_of(node)
                .map(<[u64]>::to_vec)
                .ok_or_else(|| (abi::E_NO_SUCH_NODE, format!("no node {node}")))
        })
    })
    .map_err(|error| fail("node_children", error))
}

/// `node_get`: the property's value through this tick's writes, or its default when nothing wrote it.
pub(crate) fn node_get(ctx: GnoblinCtx, node: u64, prop: u32) -> Result<GnoblinValue, i32> {
    check_ctx(ctx, "node_get", true)?;
    let value = session::with(|session| {
        let tree = session.tree_ref()?;
        let class = class_of(tree, node)?;
        let def = catalog::property_def(class, prop).ok_or_else(|| {
            (
                abi::E_NO_SUCH_MEMBER,
                format!("class {class} has no property with id {prop}"),
            )
        })?;
        if def.flags & abi::MEMBER_SECRET != 0 {
            return Err((abi::E_REFUSED, format!("`{}` is secret and is never read", def.name)));
        }
        match tree.get(node, def.name) {
            Some(held) => OwnedValue::from_mirror(held).ok_or_else(|| {
                (
                    abi::E_UNSUPPORTED,
                    format!("`{}` holds effects, which the ABI does not carry yet", def.name),
                )
            }),
            None => OwnedValue::default_of(def.value_kind, def.default).ok_or_else(|| {
                (
                    abi::E_UNSUPPORTED,
                    format!("`{}` is of a kind the ABI does not carry yet", def.name),
                )
            }),
        }
    })
    .map_err(|error| fail("node_get", error))?;
    Ok(lend_value(value))
}

/// `class_name`.
pub(crate) fn class_name(ctx: GnoblinCtx, class: u32) -> Result<GnoblinStr, i32> {
    check_ctx(ctx, "class_name", true)?;
    let Some(label) = catalog::class_label(class) else {
        set_code_error(abi::E_NO_SUCH_MEMBER, format!("class_name: no class with id {class}"));
        return Err(abi::E_NO_SUCH_MEMBER);
    };
    Ok(crate::values::lend_name(&label))
}

/// `write_create`.
pub(crate) fn create(ctx: GnoblinCtx, class: u32) -> Result<u64, i32> {
    check_ctx(ctx, "write_create", true)?;
    let name = match catalog::class(class) {
        Some(def) if def.namespace == "tree" => def.name,
        Some(def) => {
            set_code_error(
                abi::E_NO_SUCH_MEMBER,
                format!(
                    "write_create: `{}.{}` is not a class of the tree",
                    def.namespace, def.name
                ),
            );
            return Err(abi::E_NO_SUCH_MEMBER);
        }
        None => {
            set_code_error(abi::E_NO_SUCH_MEMBER, format!("write_create: no class with id {class}"));
            return Err(abi::E_NO_SUCH_MEMBER);
        }
    };
    session::with(|session| {
        session
            .tree()?
            .create(name)
            .map_err(|error| (abi::E_ERR, error.to_string()))
    })
    .map_err(|error| fail("write_create", error))
}

/// `write_destroy`.
pub(crate) fn destroy(ctx: GnoblinCtx, node: u64) -> i32 {
    if let Err(code) = check_ctx(ctx, "write_destroy", true) {
        return code;
    }
    let result = session::with(|session| {
        session
            .tree()?
            .destroy(node)
            .map(|_| ())
            .map_err(|error| (abi::E_NO_SUCH_NODE, error.to_string()))
    });
    match result {
        Ok(()) => abi::E_OK,
        Err(error) => fail("write_destroy", error),
    }
}

/// `write_insert`.
pub(crate) fn insert(ctx: GnoblinCtx, parent: u64, node: u64, index: i64) -> i32 {
    if let Err(code) = check_ctx(ctx, "write_insert", true) {
        return code;
    }
    let result = session::with(|session| {
        session
            .tree()?
            .insert(node, parent, index)
            .map_err(|error| (abi::E_NO_SUCH_NODE, error.to_string()))
    });
    match result {
        Ok(()) => abi::E_OK,
        Err(error) => fail("write_insert", error),
    }
}

/// The curve a timed write plays: linear. The compositor needs one and the ABI carries none yet.
const LINEAR: [f64; 2] = [0.0, 1.0];

/// `write_set`.
///
/// # Safety
///
/// `value` must be null or point at a `GnoblinValue` valid for the call.
pub(crate) unsafe fn set(ctx: GnoblinCtx, node: u64, prop: u32, value: *const GnoblinValue, seconds: f64) -> i32 {
    if let Err(code) = check_ctx(ctx, "write_set", true) {
        return code;
    }
    if !seconds.is_finite() || seconds < 0.0 {
        set_code_error(abi::E_ERR, format!("write_set: {seconds} is not a duration"));
        return abi::E_ERR;
    }
    let result = session::with(|session| {
        let tree = session.tree()?;
        let class = class_of(tree, node)?;
        let def = catalog::property_def(class, prop).ok_or_else(|| {
            (
                abi::E_NO_SUCH_MEMBER,
                format!("class {class} has no property with id {prop}"),
            )
        })?;
        if def.flags & abi::MEMBER_READONLY != 0 {
            return Err((abi::E_REFUSED, format!("read-only: `{}` cannot be set", def.name)));
        }
        if seconds > 0.0 && def.flags & abi::MEMBER_ANIMATED == 0 {
            return Err((
                abi::E_REFUSED,
                format!("not-animated: `{}` cannot be animated", def.name),
            ));
        }
        // SAFETY: the caller guarantees `value` is null or valid for the call.
        let value: MirrorValue =
            unsafe { read_value(value, def.value_kind) }.map_err(|error| (error.code(), error.message().to_owned()))?;
        let animation = if seconds > 0.0 {
            TxAnimation {
                seconds: Some(seconds),
                samples: Some(LINEAR.to_vec()),
                ..TxAnimation::default()
            }
        } else {
            TxAnimation::default()
        };
        tree.set(node, def.name, value, animation)
            .map_err(|error| (abi::E_NO_SUCH_NODE, error.to_string()))
    });
    match result {
        Ok(()) => abi::E_OK,
        Err(error) => fail("write_set", error),
    }
}

/// Calls the shell's answer callback and, being one-shot, removes its own registration afterwards.
fn answer_subscription(
    token: u64,
    cb: GnoblinAnswerFn,
    user: usize,
    reg: &Arc<AtomicU64>,
    cleanup: Option<gnoblin_shell_host::generation::Release>,
) -> Subscription {
    let reg = Arc::clone(reg);
    Subscription {
        matches: Box::new(
            move |event: &ShellEvent| matches!(event, ShellEvent::Answer(answer) if answer.token == token),
        ),
        call: Arc::new(move |event: &ShellEvent| {
            let ShellEvent::Answer(answer) = event else {
                return CallbackOutcome::Handled;
            };
            let raw = GnoblinAnswer {
                size: size_of::<GnoblinAnswer>() as u32,
                code: if answer.error.is_empty() {
                    abi::E_OK
                } else {
                    abi::E_REFUSED
                },
                result: GnoblinStr {
                    ptr: if answer.result.is_empty() {
                        core::ptr::null()
                    } else {
                        answer.result.as_ptr()
                    },
                    len: answer.result.len(),
                },
                error: GnoblinStr::borrowed(&answer.error),
            };
            // SAFETY: `cb` and `user` came from one `write_call` or `flush`; `raw` and what it points at are live for the call.
            let rc = unsafe { cb(user as *mut c_void, &raw const raw) };
            // The answer comes once: the registration goes, and the shell's `drop_user` with it, after this callback returns.
            registry::remove_id(reg.load(Ordering::Acquire));
            registry::outcome_of(rc)
        }),
        cleanup,
    }
}

/// `write_call`.
///
/// # Safety
///
/// `args` must be null or point at `n_args` values valid for the call; `out` null or writable.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn call(
    ctx: GnoblinCtx,
    node: u64,
    method: u32,
    args: *const GnoblinValue,
    n_args: usize,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "write_call", true) {
        return code;
    }
    let queued = session::with(|session| {
        let tree = session.tree()?;
        let class = class_of(tree, node)?;
        let def = catalog::method_def(class, method).ok_or_else(|| {
            (
                abi::E_NO_SUCH_MEMBER,
                format!("class {class} has no method with id {method}"),
            )
        })?;
        // SAFETY: the caller guarantees `args` is null or points at `n_args` values valid for the call.
        let wire = unsafe { read_args(args, n_args, def.args, def.name) }
            .map_err(|error| (error.code(), error.message().to_owned()))?;
        tree.call(node, def.name, wire.args, wire.text)
            .map_err(|error| (abi::E_NO_SUCH_NODE, error.to_string()))
    });
    let token = match queued {
        Ok(token) => token,
        Err(error) => return fail("write_call", error),
    };
    let Some(cb) = cb else {
        // Nobody waits for the answer. `user` was not adopted, so a `drop_user` is not owed either; the caller keeps both.
        if !out.is_null() {
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { out.write(0) };
        }
        return abi::E_OK;
    };
    let reg = Arc::new(AtomicU64::new(0));
    let subscription = answer_subscription(token, cb, user as usize, &reg, None);
    match registry::subscribe(ctx, "write_call", subscription, user, drop_user) {
        Ok(id) => {
            reg.store(id, Ordering::Release);
            if !out.is_null() {
                // SAFETY: the caller guarantees `out` is null or writable.
                unsafe { out.write(encode(KIND_PENDING, id)) };
            }
            abi::E_OK
        }
        Err(code) => code,
    }
}

/// `flush`: sends what the tick wrote now, and (with a callback) says when everything sent has been answered.
///
/// # Safety
///
/// `out` must be null or writable.
pub(crate) unsafe fn flush(
    ctx: GnoblinCtx,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "flush", true) {
        return code;
    }
    if !session::with(|session| session.attached()) {
        set_code_error(
            abi::E_ERR,
            "flush: the shell has not attached to the tree: call tree_attach first",
        );
        return abi::E_ERR;
    }
    if let Err(text) = session::with(session::Session::flush) {
        set_code_error(abi::E_ERR, format!("flush: the tree link failed: {text}"));
        return abi::E_ERR;
    }
    let Some(cb) = cb else {
        if !out.is_null() {
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { out.write(0) };
        }
        return abi::E_OK;
    };
    let token = session::with(session::Session::wait_for_flush);
    let reg = Arc::new(AtomicU64::new(0));
    let cleanup: gnoblin_shell_host::generation::Release =
        Box::new(move || session::with(|session| session.forget_flush(token)));
    let subscription = answer_subscription(token, cb, user as usize, &reg, Some(cleanup));
    match registry::subscribe(ctx, "flush", subscription, user, drop_user) {
        Ok(id) => {
            reg.store(id, Ordering::Release);
            if !out.is_null() {
                // SAFETY: the caller guarantees `out` is null or writable.
                unsafe { out.write(encode(KIND_PENDING, id)) };
            }
            abi::E_OK
        }
        Err(code) => {
            session::with(|session| session.forget_flush(token));
            code
        }
    }
}

/// `cancel`: a pending call or flush will never call back.
pub(crate) fn cancel(pending: GnoblinPending) -> i32 {
    let Some(id) = decode(KIND_PENDING, pending) else {
        set_code_error(abi::E_STALE, format!("cancel: {pending:#x} is not a pending handle"));
        return abi::E_STALE;
    };
    match registry::remove_pending(id) {
        Ok(()) => abi::E_OK,
        Err(code) => code,
    }
}
