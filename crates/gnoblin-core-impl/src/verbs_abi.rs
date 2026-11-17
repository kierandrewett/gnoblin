//! The table's `call` and `subscribe`: the ABI face of [`crate::verbs`].
#![allow(unsafe_code)]

use core::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gnoblin_core_abi::{
    self as abi, GnoblinAnswerFn, GnoblinCtx, GnoblinDropFn, GnoblinEventFn, GnoblinPending, GnoblinReg, GnoblinValue,
};
use gnoblin_model::{Fields, Value};
use gnoblin_shell_host::handle::{KIND_PENDING, encode};
use gnoblin_tree_client::tree_mirror::MirrorValue;

use crate::catalog::{self, ClassDef};
use crate::guard::set_code_error;
use crate::registry::{self, Registering, check_ctx};
use crate::session;
use crate::tree::answer_subscription;
use crate::values::{ValueError, read_value};
use crate::verbs::{VerbCall, next_token};

/// The class of the compositor's `gnoblin` namespace that `class` is, or the failure to record: the tree's classes are reached by
/// `write_call` and `on_signal`, not by verbs.
fn verb_class(class: u32, what: &str) -> Result<&'static ClassDef, i32> {
    let Some(def) = catalog::class(class) else {
        set_code_error(abi::E_NO_SUCH_MEMBER, format!("{what}: no class with id {class}"));
        return Err(abi::E_NO_SUCH_MEMBER);
    };
    if def.namespace == "tree" {
        set_code_error(
            abi::E_NO_SUCH_MEMBER,
            format!(
                "{what}: `{}` is a class of the tree, whose methods are write_call's and whose signals are on_signal's; \
                 verbs are the `gnoblin` namespace's",
                def.name
            ),
        );
        return Err(abi::E_NO_SUCH_MEMBER);
    }
    Ok(def)
}

/// A value the shell passed as the compositor's model carries it.
fn model(value: MirrorValue) -> Result<Value, ValueError> {
    Ok(match value {
        MirrorValue::Bool(value) => Value::Bool(value),
        MirrorValue::Int(value) => Value::Int(value),
        MirrorValue::Uint(value) | MirrorValue::Node(value) => Value::Uint(value),
        MirrorValue::Double(value) => Value::Double(value),
        MirrorValue::Text(value) => Value::Text(value),
        MirrorValue::Bytes(value) => Value::Bytes(value),
        MirrorValue::List(numbers) => Value::List(numbers.into_iter().map(Value::Double).collect()),
        MirrorValue::Texts(texts) => Value::List(texts.into_iter().map(Value::Text).collect()),
        MirrorValue::Effects(_) => {
            return Err(ValueError::Unsupported(
                "a list of effects cannot be sent as a verb's argument".to_owned(),
            ));
        }
    })
}

/// `call`.
///
/// # Safety
///
/// `args` must be null or point at `n_args` values valid for the call; `out` null or writable.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn call(
    ctx: GnoblinCtx,
    object: u64,
    class: u32,
    method: u32,
    args: *const GnoblinValue,
    n_args: usize,
    cb: Option<GnoblinAnswerFn>,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
    out: *mut GnoblinPending,
) -> i32 {
    if let Err(code) = check_ctx(ctx, "call", true) {
        return code;
    }
    let def = match verb_class(class, "call") {
        Ok(def) => def,
        Err(code) => return code,
    };
    let Some(method_def) = catalog::method_def(class, method) else {
        set_code_error(
            abi::E_NO_SUCH_MEMBER,
            format!("call: class `{}` has no method with id {method}", def.name),
        );
        return abi::E_NO_SUCH_MEMBER;
    };
    if n_args < method_def.required || n_args > method_def.args.len() {
        set_code_error(
            abi::E_TYPE,
            format!(
                "call: `{}.{}` takes {} arguments{}, not {n_args}",
                def.name,
                method_def.name,
                method_def.required,
                if method_def.required == method_def.args.len() {
                    String::new()
                } else {
                    format!(" to {}", method_def.args.len())
                }
            ),
        );
        return abi::E_TYPE;
    }
    let mut fields = Fields::new();
    if n_args > 0 {
        if args.is_null() {
            set_code_error(abi::E_ERR, "call: arguments with a count and no data");
            return abi::E_ERR;
        }
        for (index, name) in method_def.arg_names.split(' ').take(n_args).enumerate() {
            // SAFETY: the caller guarantees `args` points at `n_args` values valid for the call, and `index < n_args`.
            let read = unsafe { read_value(args.add(index), method_def.args[index]) }.and_then(model);
            match read {
                Ok(value) => {
                    fields.insert(name.to_owned(), value);
                }
                Err(error) => {
                    set_code_error(error.code(), format!("call: `{name}`: {}", error.message()));
                    return error.code();
                }
            }
        }
    }
    let token = next_token();
    let call = VerbCall {
        token,
        object,
        class: def.name.to_owned(),
        method: method_def.name.to_owned(),
        args: fields,
    };
    let sent = session::with(|session| {
        session
            .verbs()
            .and_then(|link| link.call(call).map_err(|text| (abi::E_ERR, text)))
    });
    if let Err((code, text)) = sent {
        set_code_error(code, format!("call: {text}"));
        return code;
    }
    let Some(cb) = cb else {
        // Nobody waits for the answer. `user` was not adopted, so a `drop_user` is not owed either; the caller keeps both.
        if !out.is_null() {
            // SAFETY: the caller guarantees `out` is null or writable.
            unsafe { out.write(0) };
        }
        return abi::E_OK;
    };
    // The call is out, and its answer cannot be delivered before this returns: delivery is on this thread, in `dispatch`.
    let reg = Arc::new(AtomicU64::new(0));
    let subscription = answer_subscription(token, cb, user as usize, &reg, None);
    match registry::subscribe(ctx, "call", subscription, user, drop_user) {
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

/// `subscribe`.
pub(crate) fn subscribe(
    ctx: GnoblinCtx,
    class: u32,
    signal: u32,
    cb: GnoblinEventFn,
    user: *mut c_void,
    drop_user: Option<GnoblinDropFn>,
) -> GnoblinReg {
    if check_ctx(ctx, "subscribe", true).is_err() {
        return 0;
    }
    if verb_class(class, "subscribe").is_err() {
        return 0;
    }
    if let Err(message) = catalog::check_signal(class, signal) {
        set_code_error(abi::E_NO_SUCH_MEMBER, format!("subscribe: {message}"));
        return 0;
    }
    // The connection is opened now, so a shell learns at registration that nothing will ever arrive, not by waiting.
    if let Err((code, text)) = session::with(|session| session.verbs().map(|_| ())) {
        set_code_error(code, format!("subscribe: {text}"));
        return 0;
    }
    registry::register(
        ctx,
        "subscribe",
        Registering::Signal {
            node: 0,
            class,
            signal,
            cb,
        },
        user,
        drop_user,
    )
}
