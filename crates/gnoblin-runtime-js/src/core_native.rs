//! `__gnoblinCore`: the core's `call` and `subscribe`, by name, for a script (`design/modular-framework.md` section 5.9).
//!
//! The compositor's `gnoblin` namespace (`gnoblin.session`, `gnoblin.idle`, the outputs) is not part of the tree a script mirrors:
//! its verbs are calls on the owner socket and its signals are observations of it, and the core serves both through two table
//! functions. This module is the script's door to them and it is generic on purpose, as `__gnoblinStd` is: `call("gnoblin.session",
//! "logout", [], onAnswer, 0)` resolves the class and the method in the catalog, converts the script's values to the kinds the
//! catalog says the parameters have, and hands the call to the core. The typed spelling (`verbs.session.logout()`,
//! `verbs.session.onBell(fn)`) is generated from the same catalog into the generated `gnoblin-core/generated/verbs` module, so a verb added to
//! the schema changes no line here.
//!
//! The answer and the signals come back through the core's dispatch, which the runtime runs between frames: the core calls the
//! callback on the shell thread, and the callback enters the running generation under the listener budget, as a callback of the std
//! library does ([`crate::std_native`]). A script function the core holds stays in the script's table of references and the core holds
//! only its number; a function of a generation that was replaced is not run.
#![allow(unsafe_code)]

use core::ffi::c_void;

use gnoblin_core_abi::{
    self as abi, GnoblinAnswer, GnoblinCtx, GnoblinEvent, GnoblinMember, GnoblinStr, GnoblinValue, MEMBER_METHOD,
    MEMBER_SIGNAL,
};
use gnoblin_js::rquickjs::{self, Array, Ctx, Exception, Function, Object, Value};
use gnoblin_js::{JsNative, ScriptIds};

use crate::std_native::{context, entered, mark_used, release, release_later, token_of};
use crate::table::{CoreTable, remembered};

/// A script function in the table of references of the generation that made it. Dropping it lets the reference go at the next safe
/// moment.
struct ScriptCallback {
    id: u64,
    token: u64,
}

impl Drop for ScriptCallback {
    fn drop(&mut self) {
        release_later(self.token, self.id);
    }
}

/// What the core is given for a call: the script's answer function.
struct AnswerSlot {
    callback: ScriptCallback,
}

/// What the core is given for a subscription: the script's signal function.
struct SignalSlot {
    callback: ScriptCallback,
}

/// A value on its way to or from the script, owned: no script reference is held across the core's calls.
#[derive(Clone, Debug, PartialEq)]
enum Plain {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Numbers(Vec<f64>),
    Texts(Vec<String>),
}

unsafe extern "C" fn drop_slot<T>(user: *mut c_void) {
    // SAFETY: `user` is the `Box<T>` leaked by `call` or `subscribe`; the core hands it back once.
    drop(unsafe { Box::from_raw(user.cast::<T>()) });
}

/// The text of a string the core lent for a call.
fn text_of(raw: &GnoblinStr) -> String {
    if raw.ptr.is_null() || raw.len == 0 {
        return String::new();
    }
    // SAFETY: the core lends `len` bytes at `ptr` for the call.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) }).into_owned()
}

/// The script-side value of a value the core built for an event. Kinds a script has no spelling of are `null`.
fn plain_of(raw: &GnoblinValue) -> Plain {
    match raw.kind {
        abi::VALUE_BOOL => Plain::Bool(raw.bits != 0),
        abi::VALUE_INT => Plain::Number(raw.bits as i64 as f64),
        abi::VALUE_UINT | abi::VALUE_NODE => Plain::Number(raw.bits as f64),
        abi::VALUE_DOUBLE => Plain::Number(f64::from_bits(raw.bits)),
        abi::VALUE_TEXT => Plain::Text(text_of(&GnoblinStr {
            ptr: raw.ptr.cast(),
            len: raw.len,
        })),
        abi::VALUE_LIST if !raw.ptr.is_null() => {
            // SAFETY: the core builds a list value as `len` values at `ptr`, valid for the callback.
            let items = unsafe { std::slice::from_raw_parts(raw.ptr.cast::<GnoblinValue>(), raw.len) };
            Plain::Numbers(items.iter().map(|item| f64::from_bits(item.bits)).collect())
        }
        abi::VALUE_TEXTS if !raw.ptr.is_null() => {
            // SAFETY: the core builds a texts value as `len` strings at `ptr`, valid for the callback.
            let items = unsafe { std::slice::from_raw_parts(raw.ptr.cast::<GnoblinStr>(), raw.len) };
            Plain::Texts(items.iter().map(text_of).collect())
        }
        _ => Plain::Null,
    }
}

fn to_script<'js>(ctx: &Ctx<'js>, value: &Plain) -> rquickjs::Result<Value<'js>> {
    Ok(match value {
        Plain::Null => Value::new_null(ctx.clone()),
        Plain::Bool(flag) => Value::new_bool(ctx.clone(), *flag),
        Plain::Number(number) => Value::new_number(ctx.clone(), *number),
        Plain::Text(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
        Plain::Numbers(numbers) => {
            let array = Array::new(ctx.clone())?;
            for (index, number) in numbers.iter().enumerate() {
                array.set(index, *number)?;
            }
            array.into_value()
        }
        Plain::Texts(texts) => {
            let array = Array::new(ctx.clone())?;
            for (index, text) in texts.iter().enumerate() {
                array.set(index, text.as_str())?;
            }
            array.into_value()
        }
    })
}

/// Runs the script function `callback` with `args`, in the generation that is serving, under the listener budget. `Ok(false)` when the
/// function belongs to a generation that was replaced (it is not run).
fn run(callback: &ScriptCallback, args: &[Plain]) -> Result<bool, String> {
    entered(|enter| {
        let mut ran = true;
        enter.enter(&mut |ctx| {
            let support: Object<'_> = ctx.globals().get("__gnoblinStdSupport")?;
            if token_of(&support)? != callback.token {
                ran = false;
                return Ok(());
            }
            let list = Array::new(ctx.clone())?;
            for (index, arg) in args.iter().enumerate() {
                list.set(index, to_script(ctx, arg)?)?;
            }
            let invoke: Function<'_> = support.get("invoke")?;
            invoke.call::<_, Value<'_>>((callback.id as f64, Option::<&str>::None, list))?;
            Ok(())
        })?;
        Ok(ran)
    })
}

/// What a callback of the core returns for the way `run` ended: a script that raised is a failed callback, which the core counts.
fn settle(what: &str, outcome: Result<bool, String>) -> i32 {
    match outcome {
        Ok(_) => abi::E_OK,
        Err(message) => {
            if let Some(core) = remembered() {
                core.log(context(), abi::LOG_WARN, &format!("js: {what} raised: {message}"));
            }
            abi::E_ERR
        }
    }
}

unsafe extern "C" fn answer_callback(user: *mut c_void, answer: *const GnoblinAnswer) -> i32 {
    // SAFETY: `user` is the `AnswerSlot` `call` leaked; the core hands it back on the shell thread and frees it only through `drop_slot`.
    let slot = unsafe { &*user.cast::<AnswerSlot>() };
    // SAFETY: the core passes a live answer, valid until this callback returns.
    let Some(answer) = (unsafe { answer.as_ref() }) else {
        return abi::E_ERR;
    };
    let result = text_of(&answer.result);
    let error = text_of(&answer.error);
    let error = if error.is_empty() && answer.code == abi::E_OK {
        Plain::Null
    } else {
        Plain::Text(error)
    };
    settle(
        "a verb's answer function",
        run(&slot.callback, &[Plain::Text(result), error]),
    )
}

unsafe extern "C" fn signal_callback(user: *mut c_void, event: *const GnoblinEvent) -> i32 {
    // SAFETY: `user` is the `SignalSlot` `subscribe` leaked; the core hands it back on the shell thread and frees it only through `drop_slot`.
    let slot = unsafe { &*user.cast::<SignalSlot>() };
    // SAFETY: the core passes a live event, valid until this callback returns.
    let Some(event) = (unsafe { event.as_ref() }) else {
        return abi::E_ERR;
    };
    let values: Vec<Plain> = if event.values.is_null() || event.n_values == 0 {
        Vec::new()
    } else {
        // SAFETY: the core lends `n_values` values at `values` for the callback.
        unsafe { std::slice::from_raw_parts(event.values, event.n_values) }
            .iter()
            .map(plain_of)
            .collect()
    };
    settle("a signal function", run(&slot.callback, &values))
}

fn throw_type<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, message)
}

/// The parameter names of a member, in order.
fn names_of(member: &GnoblinMember) -> Vec<String> {
    text_of(&member.arg_names)
        .split(' ')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The script's value for parameter `name` of kind `kind` (a `VALUE_*`), checked: a number for an integer parameter must be whole
/// and in range, so a wrong call is a `TypeError` that names the parameter and not a refusal from the core.
fn arg_from_script<'js>(ctx: &Ctx<'js>, name: &str, kind: u32, value: &Value<'js>) -> rquickjs::Result<Plain> {
    let wrong = |wanted: &str| throw_type(ctx, &format!("`{name}` is {wanted}"));
    match kind {
        abi::VALUE_BOOL => value.as_bool().map(Plain::Bool).ok_or_else(|| wrong("a boolean")),
        abi::VALUE_INT => match value.as_number() {
            Some(number) if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_992.0 => {
                Ok(Plain::Number(number))
            }
            _ => Err(wrong("a whole number")),
        },
        abi::VALUE_UINT | abi::VALUE_NODE => match value.as_number() {
            Some(number) if number.fract() == 0.0 && (0.0..=9_007_199_254_740_992.0).contains(&number) => {
                Ok(Plain::Number(number))
            }
            _ => Err(wrong("a whole number that is not negative")),
        },
        abi::VALUE_DOUBLE => value.as_number().map(Plain::Number).ok_or_else(|| wrong("a number")),
        abi::VALUE_TEXT => match value.as_string() {
            Some(text) => Ok(Plain::Text(text.to_string()?)),
            None => Err(wrong("a string")),
        },
        abi::VALUE_LIST => {
            let array = value.as_array().ok_or_else(|| wrong("an array of numbers"))?;
            let mut numbers = Vec::with_capacity(array.len());
            for item in array.iter::<Value<'js>>() {
                numbers.push(item?.as_number().ok_or_else(|| wrong("an array of numbers"))?);
            }
            Ok(Plain::Numbers(numbers))
        }
        abi::VALUE_TEXTS => {
            let array = value.as_array().ok_or_else(|| wrong("an array of strings"))?;
            let mut texts = Vec::with_capacity(array.len());
            for item in array.iter::<Value<'js>>() {
                let item = item?;
                let text = item.as_string().ok_or_else(|| wrong("an array of strings"))?;
                texts.push(text.to_string()?);
            }
            Ok(Plain::Texts(texts))
        }
        _ => Err(wrong("of a kind a script cannot pass")),
    }
}

/// The ABI values for `args` of the kinds `kinds`, borrowing from `args`, and `f` called with them.
fn with_raws<R>(args: &[Plain], kinds: &[u32], f: impl FnOnce(&[GnoblinValue]) -> R) -> R {
    // The arrays a list points into live here until `f` returns; the inner buffers do not move when the outer vectors grow.
    let numbers: Vec<Vec<GnoblinValue>> = args
        .iter()
        .map(|arg| match arg {
            Plain::Numbers(items) => items
                .iter()
                .map(|number| scalar(abi::VALUE_DOUBLE, number.to_bits()))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let strings: Vec<Vec<GnoblinStr>> = args
        .iter()
        .map(|arg| match arg {
            Plain::Texts(items) => items.iter().map(|text| GnoblinStr::borrowed(text)).collect(),
            _ => Vec::new(),
        })
        .collect();
    let raws: Vec<GnoblinValue> = args
        .iter()
        .enumerate()
        .map(|(index, arg)| {
            let kind = kinds.get(index).copied().unwrap_or(abi::VALUE_TEXT);
            match arg {
                Plain::Bool(flag) => scalar(abi::VALUE_BOOL, u64::from(*flag)),
                Plain::Number(number) => match kind {
                    abi::VALUE_INT => scalar(abi::VALUE_INT, *number as i64 as u64),
                    abi::VALUE_UINT => scalar(abi::VALUE_UINT, *number as u64),
                    abi::VALUE_NODE => scalar(abi::VALUE_NODE, *number as u64),
                    _ => scalar(abi::VALUE_DOUBLE, number.to_bits()),
                },
                Plain::Text(text) => GnoblinValue {
                    ptr: text.as_ptr().cast(),
                    len: text.len(),
                    ..scalar(abi::VALUE_TEXT, 0)
                },
                Plain::Numbers(items) => GnoblinValue {
                    ptr: numbers[index].as_ptr().cast(),
                    len: items.len(),
                    ..scalar(abi::VALUE_LIST, 0)
                },
                Plain::Texts(items) => GnoblinValue {
                    ptr: strings[index].as_ptr().cast(),
                    len: items.len(),
                    ..scalar(abi::VALUE_TEXTS, 0)
                },
                Plain::Null => scalar(abi::VALUE_TEXT, 0),
            }
        })
        .collect();
    f(&raws)
}

fn scalar(kind: u32, bits: u64) -> GnoblinValue {
    GnoblinValue {
        size: size_of::<GnoblinValue>() as u32,
        kind,
        flags: 0,
        reserved: 0,
        bits,
        ptr: core::ptr::null(),
        len: 0,
    }
}

/// The table of the running core and the generation scripts belong to, or a `TypeError`-free `Error` that says there is none.
fn core_table<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<(CoreTable, GnoblinCtx)> {
    remembered()
        .map(|table| (table, context()))
        .ok_or_else(|| Exception::throw_message(ctx, "no core is running this script"))
}

/// Keeps the script function `value` in the generation's table of references.
fn keep<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<ScriptCallback> {
    let support: Object<'js> = ctx.globals().get("__gnoblinStdSupport")?;
    release(&support)?;
    let id: f64 = support.get::<_, Function<'js>>("keep")?.call((value,))?;
    Ok(ScriptCallback {
        id: id as u64,
        token: token_of(&support)?,
    })
}

fn failure<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    Exception::throw_message(ctx, message)
}

/// `call(className, method, args, onAnswer, object)`.
fn call<'js>(
    ctx: Ctx<'js>,
    class_name: String,
    method: String,
    args: Array<'js>,
    on_answer: Value<'js>,
    object: f64,
) -> rquickjs::Result<()> {
    let (core, generation) = core_table(&ctx)?;
    let class = core
        .class_id(generation, &class_name)
        .ok_or_else(|| failure(&ctx, &format!("no class `{class_name}` in the catalog")))?;
    let member = core
        .member(generation, class, MEMBER_METHOD, &method)
        .map_err(|message| failure(&ctx, &message))?;
    let names = names_of(&member);
    // SAFETY: the core lends `arg_count` kinds at `arg_kinds` for the life of the process.
    let kinds: &[u32] = if member.arg_kinds.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(member.arg_kinds, member.arg_count as usize) }
    };
    if args.len() > kinds.len() {
        return Err(throw_type(
            &ctx,
            &format!(
                "`{class_name}.{method}` takes {} argument(s), not {}",
                kinds.len(),
                args.len()
            ),
        ));
    }
    let mut plain = Vec::with_capacity(args.len());
    for (index, arg) in args.iter::<Value<'js>>().enumerate() {
        let name = names.get(index).map_or("an argument", String::as_str);
        plain.push(arg_from_script(&ctx, name, kinds[index], &arg?)?);
    }
    let object = if object.is_finite() && object >= 0.0 {
        object as u64
    } else {
        0
    };
    let slot = if on_answer.is_function() {
        let callback = keep(&ctx, on_answer)?;
        Some(Box::into_raw(Box::new(AnswerSlot { callback })))
    } else {
        None
    };
    let (cb, user, drop_user): (Option<abi::GnoblinAnswerFn>, *mut c_void, Option<abi::GnoblinDropFn>) = match slot {
        Some(slot) => (Some(answer_callback), slot.cast(), Some(drop_slot::<AnswerSlot>)),
        None => (None, core::ptr::null_mut(), None),
    };
    mark_used();
    // SAFETY: `user` and `drop_user` are the slot's (or neither): the core adopts the slot when the call succeeds and then frees it
    // through `drop_slot` exactly once.
    let outcome = with_raws(&plain, kinds, |raws| unsafe {
        core.call(generation, object, class, member.id, raws, cb, user, drop_user)
    });
    if let Err(message) = outcome {
        if let Some(slot) = slot {
            // SAFETY: the core refused the call, so it has not adopted the slot and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
        }
        return Err(failure(&ctx, &message));
    }
    Ok(())
}

/// `subscribe(className, signal, callback)`: the number that removes the registration.
fn subscribe<'js>(ctx: Ctx<'js>, class_name: String, signal: String, callback: Value<'js>) -> rquickjs::Result<f64> {
    if !callback.is_function() {
        return Err(throw_type(&ctx, "a signal needs a function to call"));
    }
    let (core, generation) = core_table(&ctx)?;
    let class = core
        .class_id(generation, &class_name)
        .ok_or_else(|| failure(&ctx, &format!("no class `{class_name}` in the catalog")))?;
    let member = core
        .member(generation, class, MEMBER_SIGNAL, &signal)
        .map_err(|message| failure(&ctx, &message))?;
    let callback = keep(&ctx, callback)?;
    let slot = Box::into_raw(Box::new(SignalSlot { callback }));
    mark_used();
    // SAFETY: `slot` is a leaked box the core hands back to `signal_callback` and `drop_slot::<SignalSlot>`.
    let outcome = unsafe {
        core.subscribe(
            generation,
            class,
            member.id,
            signal_callback,
            slot.cast(),
            Some(drop_slot::<SignalSlot>),
        )
    };
    match outcome {
        Ok(registration) => Ok(registration as f64),
        Err(message) => {
            // SAFETY: the core refused the registration, so it has not adopted the slot and nothing else owns the box.
            drop(unsafe { Box::from_raw(slot) });
            Err(failure(&ctx, &message))
        }
    }
}

/// `unsubscribe(registration)`.
fn unsubscribe<'js>(ctx: Ctx<'js>, registration: f64) -> rquickjs::Result<()> {
    let (core, _) = core_table(&ctx)?;
    match core.remove(registration as u64) {
        abi::E_OK => Ok(()),
        code => Err(failure(
            &ctx,
            &format!(
                "the registration is not there to remove (it was removed already, or its generation ended): code {code}"
            ),
        )),
    }
}

/// The native for each generation: `globalThis.__gnoblinCore`.
pub(crate) fn native() -> JsNative {
    JsNative::new(install)
}

fn install<'js>(ctx: &Ctx<'js>, _ids: &ScriptIds) -> rquickjs::Result<()> {
    let core = Object::new(ctx.clone())?;
    // The functions hold no script value: a closure that kept one would be a root the engine cannot see through, and the runtime could
    // not free its heap. They read what they need from the global scope each time.
    core.set(
        "call",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, class: String, method: String, args: Array<'js>, on_answer: Value<'js>, object: f64| {
                call(ctx, class, method, args, on_answer, object)
            },
        )?,
    )?;
    core.set(
        "subscribe",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'js>, class: String, signal: String, callback: Value<'js>| {
                subscribe(ctx, class, signal, callback)
            },
        )?,
    )?;
    core.set(
        "unsubscribe",
        Function::new(ctx.clone(), |ctx: Ctx<'js>, registration: f64| {
            unsubscribe(ctx, registration)
        })?,
    )?;
    ctx.globals().set("__gnoblinCore", core)?;
    Ok(())
}
