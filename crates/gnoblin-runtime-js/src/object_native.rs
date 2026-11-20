//! `__gnoblinObjects`: the object protocol of the libraries (`gnoblin-object`), for a script.
//!
//! Both libraries serve their classes through one table of generic calls (`create`, `get`, `set`, `call`, `listen`, `implement`, ...), and
//! this module is a script's door to it, generic on purpose: it knows no class. The classes a shell uses (`EntryModel`, `FocusPolicy`)
//! are generated from the catalog into `gnoblin-std/generated/objects` and `gnoblin-core/generated/objects` and call these functions with
//! the ids the catalog gave, so a class added to a catalog changes no line here.
//!
//! A library object is held by a *slot*: a small number the script keeps, which names an entry of a table the native functions share (one
//! table for each evaluated generation). The entry holds the library's handle, which is a 64-bit number a script cannot hold exactly. A
//! script's reference ends with `release`, which the generated base queues from a `FinalizationRegistry`.
//!
//! Values cross as the script's own, with three marks on the way in (the generated base writes them): `{ $t: 1, $v: fields }` is a
//! record, `{ $t: 2, $v: id }` an enum value, `{ $t: 3, $v: slot }` an object and `{ $t: 4, $v: numbers }` bytes. On the way out a record
//! is an array of its fields, an enum value its id, an object a new slot and bytes a `Uint8Array`, and the generated class that asked for
//! the value knows which it is from the type of the member. A function is a closure the library may call from its turn, with the
//! member's id and an array of arguments; it stays in the script's table of references (`__gnoblinStdSupport`), as every callback does
//! (`std_native`), and the library holds only its number.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core_abi::{self as abi, MEMBER_METHOD, MEMBER_PROPERTY, MEMBER_SIGNAL};
use gnoblin_js::rquickjs::{self, Array, Ctx, Exception, Function, Object, Value};
use gnoblin_js::{JsNative, ScriptIds};
use gnoblin_object::client::Api;
use gnoblin_object::{Callable, ClassId, Closure, Fail, MemberId, Obj, Res, Timing, Val};

use crate::core_native::{ScriptCallback, keep};
use crate::std_native::{context, entered, library, mark_used, token_of, within_call};
use crate::table::{CoreTable, remembered};

/// The namespaces the core serves; every other is the std library's.
const CORE_NAMESPACES: &[&str] = &["gnoblin.model"];

/// One opened namespace of a generation.
struct OpenedNamespace {
    name: String,
    api: Api,
    /// The name of each class, read from the catalog the first time one is asked for.
    classes: RefCell<Option<HashMap<ClassId, String>>>,
}

/// What a slot stands for: a reference to a library object.
#[derive(Clone, Copy)]
struct ObjectSlot {
    namespace: usize,
    class: ClassId,
    obj: Obj,
}

/// The state the native functions of one generation share.
#[derive(Default)]
struct ObjectTable {
    namespaces: RefCell<Vec<OpenedNamespace>>,
    slots: RefCell<Vec<Option<ObjectSlot>>>,
    free: RefCell<Vec<usize>>,
}

impl ObjectTable {
    fn api(&self, namespace: usize) -> Res<Api> {
        self.namespaces
            .borrow()
            .get(namespace)
            .map(|entry| entry.api)
            .ok_or_else(|| Fail::err("the namespace was not opened"))
    }

    /// A slot for a reference to `obj`, which the caller owns and the slot now holds.
    fn adopt(&self, namespace: usize, class: ClassId, obj: Obj) -> f64 {
        let slot = ObjectSlot { namespace, class, obj };
        let mut slots = self.slots.borrow_mut();
        let index = match self.free.borrow_mut().pop() {
            Some(index) => {
                slots[index] = Some(slot);
                index
            }
            None => {
                slots.push(Some(slot));
                slots.len() - 1
            }
        };
        (index + 1) as f64
    }

    fn slot(&self, number: f64) -> Res<ObjectSlot> {
        let index = number as usize;
        index
            .checked_sub(1)
            .and_then(|index| self.slots.borrow().get(index).copied().flatten())
            .ok_or_else(|| Fail::stale("the object was released or ended"))
    }

    /// Takes the slot out of the table: the reference is the caller's to release.
    fn take(&self, number: f64) -> Option<ObjectSlot> {
        let index = (number as usize).checked_sub(1)?;
        let taken = self.slots.borrow_mut().get_mut(index)?.take();
        if taken.is_some() {
            self.free.borrow_mut().push(index);
        }
        taken
    }

    /// The name of the class `class` of `namespace`.
    fn class_name(&self, namespace: usize, class: ClassId) -> Res<String> {
        let namespaces = self.namespaces.borrow();
        let entry = namespaces
            .get(namespace)
            .ok_or_else(|| Fail::err("the namespace was not opened"))?;
        if entry.classes.borrow().is_none() {
            let names = entry
                .api
                .catalog()?
                .into_iter()
                .filter(|item| item.entry == abi::CATALOG_CLASS)
                .map(|item| (item.class, item.name))
                .collect();
            *entry.classes.borrow_mut() = Some(names);
        }
        entry
            .classes
            .borrow()
            .as_ref()
            .and_then(|names| names.get(&class).cloned())
            .ok_or_else(|| Fail::err(format!("`{}` has no class {class}", entry.name)))
    }
}

/// What a script's function is to the library: a closure that runs it from the core's dispatch, in the generation that made it.
struct JsClosure {
    callback: ScriptCallback,
    table: Rc<ObjectTable>,
    ids: ScriptIds,
    namespace: usize,
}

/// How a call of a script function ended.
enum CallEnded {
    /// The function belongs to a generation that was replaced: it was not run.
    Replaced,
    Answered(Val),
    /// It raised, with the exception's text.
    Raised(String),
}

impl Callable for JsClosure {
    fn call(&self, member: MemberId, args: &[Val]) -> Res<Val> {
        let ended = entered(|enter| {
            let mut ended = CallEnded::Replaced;
            enter.enter(&mut |ctx| {
                let support: Object<'_> = ctx.globals().get("__gnoblinStdSupport")?;
                if token_of(&support)? != self.callback.token {
                    return Ok(());
                }
                let run = || -> rquickjs::Result<Val> {
                    let list = Array::new(ctx.clone())?;
                    for (index, arg) in args.iter().enumerate() {
                        list.set(index, from_val(ctx, &self.table, &self.ids, self.namespace, arg)?)?;
                    }
                    let call_args = Array::new(ctx.clone())?;
                    call_args.set(0, f64::from(member))?;
                    call_args.set(1, list)?;
                    let invoke: Function<'_> = support.get("invoke")?;
                    let answer: Value<'_> = invoke.call((self.callback.id as f64, Option::<&str>::None, call_args))?;
                    to_val(ctx, &self.table, &self.ids, self.namespace, answer)
                };
                ended = match run() {
                    Ok(answer) => CallEnded::Answered(answer),
                    Err(rquickjs::Error::Exception) => CallEnded::Raised(describe(ctx)),
                    Err(other) => return Err(other),
                };
                Ok(())
            })?;
            Ok(ended)
        })
        .map_err(|message| Fail::new(abi::E_PANIC, message))?;
        match ended {
            CallEnded::Answered(answer) => Ok(answer),
            CallEnded::Replaced => Err(Fail::new(
                abi::E_RETIRED,
                "the function belongs to a generation that was replaced",
            )),
            CallEnded::Raised(message) => {
                if let Some(core) = remembered() {
                    core.log(
                        context(),
                        abi::LOG_WARN,
                        &format!("js: an object callback raised: {message}"),
                    );
                }
                Err(Fail::new(abi::E_PANIC, message))
            }
        }
    }
}

/// The text of the exception the context holds, cleared.
fn describe(ctx: &Ctx<'_>) -> String {
    let caught = ctx.catch();
    if let Some(object) = caught.as_object().cloned()
        && let Some(exception) = Exception::from_object(object)
    {
        let message = exception.message().unwrap_or_default();
        return match exception.stack() {
            Some(stack) if !stack.is_empty() => format!("{message}\n{stack}"),
            _ => message,
        };
    }
    caught
        .as_string()
        .and_then(|text| text.to_string().ok())
        .unwrap_or_else(|| "a script exception".to_owned())
}

/// The script's `value` as a [`Val`] for a call of the namespace `namespace`: a function in it is a closure the library calls back with
/// values of that namespace.
fn to_val<'js>(
    ctx: &Ctx<'js>,
    table: &Rc<ObjectTable>,
    ids: &ScriptIds,
    namespace: usize,
    value: Value<'js>,
) -> rquickjs::Result<Val> {
    to_val_in(ctx, table, ids, namespace, value, 0)
}

fn to_val_in<'js>(
    ctx: &Ctx<'js>,
    table: &Rc<ObjectTable>,
    ids: &ScriptIds,
    namespace: usize,
    value: Value<'js>,
    depth: u32,
) -> rquickjs::Result<Val> {
    if depth > 32 {
        return Err(Exception::throw_range(
            ctx,
            "a value nested more than 32 deep cannot cross to the library",
        ));
    }
    if value.is_undefined() || value.is_null() {
        return Ok(Val::None);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(Val::Bool(flag));
    }
    if let Some(number) = value.as_number() {
        if number.is_finite() && number >= ScriptIds::HANDLE_BASE {
            return ids
                .from_script(number)
                .map(Val::Node)
                .ok_or_else(|| Exception::throw_type(ctx, "a number that names no node"));
        }
        return Ok(if number.fract() == 0.0 && number.abs() < 9.0e15 {
            Val::Int(number as i64)
        } else {
            Val::Double(number)
        });
    }
    if let Some(text) = value.as_string() {
        return Ok(Val::Text(text.to_string()?));
    }
    if let Some(array) = value.as_array() {
        let mut items = Vec::with_capacity(array.len());
        for item in array.iter::<Value<'js>>() {
            items.push(to_val_in(ctx, table, ids, namespace, item?, depth + 1)?);
        }
        return Ok(Val::List(items));
    }
    if value.is_function() {
        let callback = keep(ctx, value)?;
        mark_used();
        let closure: Closure = Rc::new(JsClosure {
            callback,
            table: Rc::clone(table),
            ids: ids.clone(),
            namespace,
        });
        return Ok(Val::Closure(closure));
    }
    let Some(object) = value.as_object() else {
        return Err(Exception::throw_type(ctx, "a value the object protocol cannot take"));
    };
    let mark: Option<f64> = object.get("$t")?;
    let payload: Value<'js> = object.get("$v")?;
    match mark {
        Some(1.0) => {
            let Some(fields) = payload.as_array() else {
                return Err(Exception::throw_type(ctx, "a record's fields are an array"));
            };
            let mut items = Vec::with_capacity(fields.len());
            for item in fields.iter::<Value<'js>>() {
                items.push(to_val_in(ctx, table, ids, namespace, item?, depth + 1)?);
            }
            Ok(Val::Record {
                class: 0,
                fields: items,
            })
        }
        Some(2.0) => {
            let id = payload.as_number().unwrap_or(0.0);
            Ok(Val::Enum {
                class: 0,
                value: id as u32,
            })
        }
        Some(3.0) => {
            let slot = table
                .slot(payload.as_number().unwrap_or(0.0))
                .map_err(|fail| throw(ctx, &fail))?;
            Ok(Val::Object {
                class: slot.class,
                obj: slot.obj,
            })
        }
        Some(4.0) => {
            let Some(numbers) = payload.as_array() else {
                return Err(Exception::throw_type(ctx, "bytes are an array of numbers"));
            };
            let mut bytes = Vec::with_capacity(numbers.len());
            for item in numbers.iter::<f64>() {
                bytes.push(item? as u8);
            }
            Ok(Val::Bytes(bytes))
        }
        _ => Err(Exception::throw_type(
            ctx,
            "an object the library cannot take: records, enum values and library objects are made by the generated classes",
        )),
    }
}

/// A [`Val`] the library sent, as the script's value. An object in it becomes a slot in `table` that holds the reference the library
/// handed out.
fn from_val<'js>(
    ctx: &Ctx<'js>,
    table: &Rc<ObjectTable>,
    ids: &ScriptIds,
    namespace: usize,
    value: &Val,
) -> rquickjs::Result<Value<'js>> {
    Ok(match value {
        Val::None | Val::Closure(_) => Value::new_undefined(ctx.clone()),
        Val::Bool(flag) => Value::new_bool(ctx.clone(), *flag),
        Val::Int(number) => Value::new_number(ctx.clone(), *number as f64),
        Val::Uint(number) => Value::new_number(ctx.clone(), *number as f64),
        Val::Double(number) => Value::new_number(ctx.clone(), *number),
        Val::Text(text) | Val::Secret(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
        Val::Node(id) => Value::new_number(ctx.clone(), ids.to_script(*id)),
        Val::Bytes(bytes) => {
            let numbers = Array::new(ctx.clone())?;
            for (index, byte) in bytes.iter().enumerate() {
                numbers.set(index, f64::from(*byte))?;
            }
            let make: Function<'js> = ctx.eval("(numbers) => Uint8Array.from(numbers)")?;
            make.call((numbers,))?
        }
        Val::List(items) | Val::Record { fields: items, .. } => {
            let array = Array::new(ctx.clone())?;
            for (index, item) in items.iter().enumerate() {
                array.set(index, from_val(ctx, table, ids, namespace, item)?)?;
            }
            array.into_value()
        }
        Val::Enum { value, .. } => Value::new_number(ctx.clone(), f64::from(*value)),
        Val::Object { class, obj } => {
            let class = if *class == 0 {
                table
                    .api(namespace)
                    .and_then(|api| api.class_of(*obj))
                    .map_err(|fail| throw(ctx, &fail))?
            } else {
                *class
            };
            Value::new_number(ctx.clone(), table.adopt(namespace, class, *obj))
        }
    })
}

/// The error a failed call throws: an `Error` with the library's text and its ABI code in `code`, a `TypeError` when the arguments were
/// of the wrong type.
fn throw<'js>(ctx: &Ctx<'js>, fail: &Fail) -> rquickjs::Error {
    if fail.code == abi::E_TYPE {
        return Exception::throw_type(ctx, &fail.message);
    }
    let thrown = Exception::from_message(ctx.clone(), &fail.message).and_then(|exception| {
        exception.as_object().set("code", f64::from(fail.code))?;
        Ok(exception)
    });
    match thrown {
        Ok(exception) => ctx.throw(exception.into_value()),
        Err(error) => error,
    }
}

/// A library's table for a namespace, and the function that reads the text of its last failure.
type Served = (&'static abi::GnoblinObjectApi, fn() -> String);

thread_local! {
    /// The namespaces a host that holds a library of its own serves (`embed`): a test that runs a script against a catalog of its own.
    static SERVED: RefCell<HashMap<String, Served>> = RefCell::new(HashMap::new());
}

/// Serves `namespace` from `table` on this thread, ahead of the core and the std library.
pub(crate) fn serve(namespace: &str, table: &'static abi::GnoblinObjectApi, last_error: fn() -> String) {
    SERVED.with(|served| served.borrow_mut().insert(namespace.to_owned(), (table, last_error)));
}

/// The table of `namespace`, which opens the library that serves it.
fn api_for(namespace: &str) -> Result<Api, String> {
    let generation = context();
    if let Some((table, last_error)) = SERVED.with(|served| served.borrow().get(namespace).copied()) {
        return Ok(Api::new(table, generation, last_error));
    }
    if CORE_NAMESPACES.contains(&namespace) {
        let core: CoreTable = remembered().ok_or("no core is running this script")?;
        let raw = core.object_api(generation, namespace)?;
        return Ok(Api::new(raw, generation, crate::table::last_error));
    }
    let library = library()?;
    let raw = library
        .object_api(generation, namespace)
        .map_err(|error| error.message)?;
    Ok(Api::new(raw, generation, gnoblin_std_dyn::last_error_text))
}

/// The pairs `[member, value]` of an array a script gave, as the inits of a call.
fn inits_of<'js>(
    ctx: &Ctx<'js>,
    table: &Rc<ObjectTable>,
    ids: &ScriptIds,
    namespace: usize,
    pairs: &Array<'js>,
) -> rquickjs::Result<Vec<(MemberId, Val)>> {
    let mut inits = Vec::with_capacity(pairs.len());
    for pair in pairs.iter::<Array<'js>>() {
        let pair = pair?;
        let member: f64 = pair.get(0)?;
        let value: Value<'js> = pair.get(1)?;
        inits.push((member as MemberId, to_val(ctx, table, ids, namespace, value)?));
    }
    Ok(inits)
}

fn args_of<'js>(
    ctx: &Ctx<'js>,
    table: &Rc<ObjectTable>,
    ids: &ScriptIds,
    namespace: usize,
    args: &Array<'js>,
) -> rquickjs::Result<Vec<Val>> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args.iter::<Value<'js>>() {
        values.push(to_val(ctx, table, ids, namespace, arg?)?);
    }
    Ok(values)
}

/// The native for each generation: `globalThis.__gnoblinObjects`.
pub(crate) fn native() -> JsNative {
    JsNative::new(install)
}

fn install<'js>(ctx: &Ctx<'js>, ids: &ScriptIds) -> rquickjs::Result<()> {
    let table = Rc::new(ObjectTable::default());
    let native = Object::new(ctx.clone())?;

    // The functions hold no script value: a closure that kept one would be a root the engine cannot see through, and the runtime could
    // not free its heap. They read what they need from the global scope each time.
    let state = Rc::clone(&table);
    native.set(
        "open",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String| -> rquickjs::Result<f64> {
                if let Some(found) = state.namespaces.borrow().iter().position(|entry| entry.name == name) {
                    return Ok(found as f64);
                }
                let api = api_for(&name).map_err(|message| Exception::throw_message(&ctx, &message))?;
                let mut namespaces = state.namespaces.borrow_mut();
                namespaces.push(OpenedNamespace {
                    name,
                    api,
                    classes: RefCell::new(None),
                });
                Ok((namespaces.len() - 1) as f64)
            },
        )?,
    )?;

    let state = Rc::clone(&table);
    native.set(
        "ids",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  namespace: f64,
                  class_name: String,
                  names: Vec<String>|
                  -> rquickjs::Result<Vec<f64>> {
                let lookup = || -> Res<Vec<f64>> {
                    let api = state.api(namespace as usize)?;
                    let class = api.class(&class_name)?;
                    let mut found = vec![f64::from(class)];
                    for name in &names {
                        let (kind, member) = name
                            .split_once(':')
                            .ok_or_else(|| Fail::type_error(format!("`{name}` is not `kind:name`")))?;
                        let kind = match kind {
                            "p" => MEMBER_PROPERTY,
                            "m" => MEMBER_METHOD,
                            "s" => MEMBER_SIGNAL,
                            other => return Err(Fail::type_error(format!("`{other}` is not a member kind"))),
                        };
                        found.push(f64::from(api.member(class, kind, member)?));
                    }
                    Ok(found)
                };
                lookup().map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "create",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, namespace: f64, class: f64, inits: Array<'js>| -> rquickjs::Result<f64> {
                let namespace = namespace as usize;
                let inits = inits_of(&ctx, &state, &script, namespace, &inits)?;
                let api = state.api(namespace).map_err(|fail| throw(&ctx, &fail))?;
                let obj = api
                    .create(class as ClassId, &inits)
                    .map_err(|fail| throw(&ctx, &fail))?;
                Ok(state.adopt(namespace, class as ClassId, obj))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "get",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64, member: f64| -> rquickjs::Result<Value<'js>> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                let value = api
                    .get(slot.obj, member as MemberId)
                    .map_err(|fail| throw(&ctx, &fail))?;
                from_val(&ctx, &state, &script, slot.namespace, &value)
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "set",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64, member: f64, value: Value<'js>| -> rquickjs::Result<()> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let value = to_val(&ctx, &state, &script, slot.namespace, value)?;
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                api.set(slot.obj, member as MemberId, &value)
                    .map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "update",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64, inits: Array<'js>| -> rquickjs::Result<()> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let inits = inits_of(&ctx, &state, &script, slot.namespace, &inits)?;
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                api.update(slot.obj, &inits).map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "setTimed",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  slot: f64,
                  member: f64,
                  value: Value<'js>,
                  timing: Object<'js>|
                  -> rquickjs::Result<()> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let value = to_val(&ctx, &state, &script, slot.namespace, value)?;
                let timing = Timing {
                    seconds: timing.get("seconds")?,
                    delay: timing.get("delay")?,
                    tween: timing.get::<_, f64>("tween")? as u64,
                    samples: timing.get("samples")?,
                };
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                api.set_timed(slot.obj, member as MemberId, &value, &timing)
                    .map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "call",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64, member: f64, args: Array<'js>| -> rquickjs::Result<Value<'js>> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let args = args_of(&ctx, &state, &script, slot.namespace, &args)?;
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                let value = within_call(&ctx, || api.call(slot.obj, member as MemberId, &args))
                    .map_err(|fail| throw(&ctx, &fail))?;
                from_val(&ctx, &state, &script, slot.namespace, &value)
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "callStatic",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  namespace: f64,
                  class: f64,
                  member: f64,
                  args: Array<'js>|
                  -> rquickjs::Result<Value<'js>> {
                let namespace = namespace as usize;
                let args = args_of(&ctx, &state, &script, namespace, &args)?;
                let api = state.api(namespace).map_err(|fail| throw(&ctx, &fail))?;
                let value = within_call(&ctx, || api.call_static(class as ClassId, member as MemberId, &args))
                    .map_err(|fail| throw(&ctx, &fail))?;
                from_val(&ctx, &state, &script, namespace, &value)
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "listen",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64, member: f64, handler: Value<'js>| -> rquickjs::Result<f64> {
                if !handler.is_function() {
                    return Err(Exception::throw_type(&ctx, "a listener needs a function to call"));
                }
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
                let closure = JsClosure {
                    callback: keep(&ctx, handler)?,
                    table: Rc::clone(&state),
                    ids: script.clone(),
                    namespace: slot.namespace,
                };
                mark_used();
                let registration = api
                    .listen(slot.obj, member as MemberId, move |member, args| {
                        closure.call(member, args)
                    })
                    .map_err(|fail| throw(&ctx, &fail))?;
                Ok(registration as f64)
            },
        )?,
    )?;

    let state = Rc::clone(&table);
    native.set(
        "unlisten",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, namespace: f64, registration: f64| -> rquickjs::Result<()> {
                let api = state.api(namespace as usize).map_err(|fail| throw(&ctx, &fail))?;
                api.unlisten(registration as u64).map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    let (state, script) = (Rc::clone(&table), ids.clone());
    native.set(
        "implement",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  namespace: f64,
                  class: f64,
                  members: Vec<f64>,
                  handler: Value<'js>|
                  -> rquickjs::Result<f64> {
                if !handler.is_function() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "an implementation needs a function to call",
                    ));
                }
                let namespace = namespace as usize;
                let api = state.api(namespace).map_err(|fail| throw(&ctx, &fail))?;
                let closure = JsClosure {
                    callback: keep(&ctx, handler)?,
                    table: Rc::clone(&state),
                    ids: script.clone(),
                    namespace,
                };
                mark_used();
                let members: Vec<MemberId> = members.into_iter().map(|member| member as MemberId).collect();
                let obj = api
                    .implement(class as ClassId, &members, move |member, args| {
                        closure.call(member, args)
                    })
                    .map_err(|fail| throw(&ctx, &fail))?;
                Ok(state.adopt(namespace, class as ClassId, obj))
            },
        )?,
    )?;

    let state = Rc::clone(&table);
    native.set(
        "destroy",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, slot: f64| -> rquickjs::Result<()> {
            let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
            let api = state.api(slot.namespace).map_err(|fail| throw(&ctx, &fail))?;
            api.destroy(slot.obj).map_err(|fail| throw(&ctx, &fail))
        })?,
    )?;

    // A script's reference ends: the library's object does not (see `release` of the protocol). What a retired generation holds is
    // gone with it, so a refusal here is not the script's to hear.
    let state = Rc::clone(&table);
    native.set(
        "release",
        Function::new(ctx.clone(), move |slot: f64| {
            if let Some(slot) = state.take(slot)
                && let Ok(api) = state.api(slot.namespace)
            {
                let _ = api.release(slot.obj);
            }
        })?,
    )?;

    let state = Rc::clone(&table);
    native.set(
        "classOf",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, slot: f64| -> rquickjs::Result<String> {
                let slot = state.slot(slot).map_err(|fail| throw(&ctx, &fail))?;
                state
                    .class_name(slot.namespace, slot.class)
                    .map_err(|fail| throw(&ctx, &fail))
            },
        )?,
    )?;

    ctx.globals().set("__gnoblinObjects", native)?;
    Ok(())
}
