//! `__gnoblinStd`: every function of `libgnoblin-std`'s table, by name, for a script.
//!
//! It is generic on purpose, as `__gnoblinHost` is not: `call("collection_len", handle)` converts the script's values to the call
//! layer's [`Dyn`] (`gnoblin-std-dyn`, whose per-function code is generated from the table), calls the installed library for the
//! generation that is starting or serving, and converts what comes back. The typed classes a shell uses (`gnoblin-std/native`) are
//! generated from the same table into the generated `gnoblin-std/generated/` modules, so a function added to the table changes no line here.
//!
//! A function or an object a script gives the library (a listener, a layout) stays in the script's own context, in a table of
//! references (`__gnoblinStdSupport`), and the library holds only its number. The library calls it back from the core's dispatch, which
//! the runtime runs between frames, through the shell's own entry into the running generation, under the listener budget. A
//! callback of a generation that has been replaced answers an error that says so.
//!
//! The library is opened the first time a script asks (`load`), never when the runtime starts.
#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_js::rquickjs::{self, Array, Ctx, Exception, Function, Object, Value};
use gnoblin_js::{JsNative, ScriptIds};
use gnoblin_std_dyn::{Callable, Dyn, StdError, StdLib};

/// The script half: the table of references and the helpers that need the script's own reflection.
const SUPPORT: &str = r#"
(() => {
  if (globalThis.__gnoblinStdSupport) return;
  const refs = new Map();
  let next = 1;
  const plain = (value) => {
    const proto = Object.getPrototypeOf(value);
    return proto === Object.prototype || proto === null;
  };
  const methods = (target) => {
    const names = new Set();
    for (let at = target; at && at !== Object.prototype; at = Object.getPrototypeOf(at)) {
      for (const key of Object.getOwnPropertyNames(at)) {
        if (key !== "constructor" && typeof target[key] === "function") names.add(key);
      }
    }
    return [...names];
  };
  Object.defineProperty(globalThis, "__gnoblinStdSupport", {
    value: {
      plain,
      methods,
      keep(value) { const id = next++; refs.set(id, value); return id; },
      drop(id) { refs.delete(id); },
      invoke(id, method, args) {
        const target = refs.get(id);
        if (target === undefined) throw new Error("the callback was released");
        return method == null ? target(...args) : target[method](...args);
      },
    },
  });
})();
"#;

thread_local! {
    /// The library, once opened.
    static LIBRARY: RefCell<Option<Rc<StdLib>>> = const { RefCell::new(None) };
    /// The generation of the core the script that is evaluating (or serving) belongs to.
    static CONTEXT: Cell<GnoblinCtx> = const { Cell::new(0) };
    /// An address inside the core, which says where the core is.
    static ANCHOR: Cell<usize> = const { Cell::new(0) };
    /// Whether a script has used the library: the runtime then runs the core's dispatch every turn.
    static USED: Cell<bool> = const { Cell::new(false) };
    /// The references the library released while a script was running: they are let go at the next safe moment.
    static RELEASED: RefCell<Vec<(u64, u64)>> = const { RefCell::new(Vec::new()) };
    /// How the library enters the running generation, for the length of one dispatch.
    static ENTER: Cell<Option<*const dyn Enter>> = const { Cell::new(None) };
}

/// Gives the runtime the library it would otherwise open by name, for a host that holds it itself (`embed`).
pub(crate) fn install_library(library: StdLib) {
    LIBRARY.with(|slot| *slot.borrow_mut() = Some(Rc::new(library)));
}

/// Names the core generation the scripts that run now belong to.
pub(crate) fn set_context(ctx: GnoblinCtx) {
    CONTEXT.with(|cell| cell.set(ctx));
}

/// The core generation scripts belong to.
pub(crate) fn context() -> GnoblinCtx {
    CONTEXT.with(Cell::get)
}

/// Remembers an address inside the core.
pub(crate) fn set_anchor(address: *const std::ffi::c_void) {
    ANCHOR.with(|cell| cell.set(address as usize));
}

/// Whether a script has used the library.
pub(crate) fn used() -> bool {
    USED.with(Cell::get)
}

/// Notes that a script has asked the core for something that is answered through its dispatch (a verb, a signal), so the runtime
/// runs the dispatch every turn.
pub(crate) fn mark_used() {
    USED.with(|cell| cell.set(true));
}

/// Lets go of a reference of the table of references (`__gnoblinStdSupport`) at the next safe moment: `generation` is the
/// token of the context that holds it.
pub(crate) fn release_later(generation: u64, id: u64) {
    RELEASED.with(|list| list.borrow_mut().push((generation, id)));
}

/// How to run script in the running generation, as seen by a callback.
pub(crate) trait Enter {
    /// Runs `body` in it, under the listener budget.
    fn enter(&self, body: &mut dyn for<'js> FnMut(&Ctx<'js>) -> rquickjs::Result<()>) -> Result<(), String>;
}

/// The way into the script from inside one of its own calls into the library: the call's context.
struct CallEnter<'a, 'js>(&'a Ctx<'js>);

impl Enter for CallEnter<'_, '_> {
    fn enter(&self, body: &mut dyn for<'js> FnMut(&Ctx<'js>) -> rquickjs::Result<()>) -> Result<(), String> {
        body(self.0).map_err(|error| {
            // A script's failure is the callback's: the context holds no exception that would reach the caller of the call.
            let _ = self.0.catch();
            error.to_string()
        })
    }
}

/// Runs `body` with `enter` as the way a callback gets into the running generation.
pub(crate) fn with_enter<R>(enter: &dyn Enter, body: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ENTER.with(|cell| cell.set(None));
        }
    }
    // SAFETY: the pointer is read only while `body` runs, by `entered`, and `Reset` clears it before `enter` can end.
    let erased: *const (dyn Enter + 'static) = unsafe { std::mem::transmute(enter as *const dyn Enter) };
    ENTER.with(|cell| cell.set(Some(erased)));
    let _reset = Reset;
    body()
}

/// Runs `f` with the way into the running generation: the turn's own (`with_enter`) when the dispatch that calls back is the
/// runtime's, and the shell instance's when it is the host's (the host drains the core's queue after the runtime's turn).
pub(crate) fn entered<R>(f: impl FnOnce(&dyn Enter) -> Result<R, String>) -> Result<R, String> {
    if let Some(pointer) = ENTER.with(Cell::get) {
        // SAFETY: set by `with_enter` for the length of the dispatch that is running this callback.
        return f(unsafe { &*pointer });
    }
    crate::instance::with_enter(f)
        .unwrap_or_else(|| Err("a std callback ran while no JavaScript shell was running on this thread".to_owned()))
}

fn library() -> Result<Rc<StdLib>, String> {
    if let Some(library) = LIBRARY.with(|slot| slot.borrow().clone()) {
        return Ok(library);
    }
    let anchor = ANCHOR.with(Cell::get) as *const std::ffi::c_void;
    let library = Rc::new(StdLib::load(anchor)?);
    LIBRARY.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&library)));
    Ok(library)
}

/// A script's function or object, called back by the library.
struct JsCallable {
    id: u64,
    /// The context whose table of references holds the callback.
    token: u64,
    methods: HashSet<String>,
    ids: ScriptIds,
}

impl JsCallable {
    fn run(&self, method: Option<&str>, args: Vec<Dyn>) -> Result<Dyn, String> {
        entered(|enter| {
            let mut answer = Dyn::Nil;
            let mut replaced = false;
            enter.enter(&mut |ctx| {
                let support: Object<'_> = ctx.globals().get("__gnoblinStdSupport")?;
                if token_of(&support)? != self.token {
                    replaced = true;
                    return Ok(());
                }
                let list = Array::new(ctx.clone())?;
                for (index, arg) in args.iter().enumerate() {
                    list.set(index, from_dyn(ctx, &self.ids, arg)?)?;
                }
                let invoke: Function<'_> = support.get("invoke")?;
                let value: Value<'_> = invoke.call((self.id as f64, method, list))?;
                answer = to_dyn(ctx, &self.ids, &support, value)?;
                Ok(())
            })?;
            if replaced {
                return Err("the callback belongs to a generation that was replaced".to_owned());
            }
            Ok(answer)
        })
    }
}

impl Callable for JsCallable {
    fn invoke(&self, args: Vec<Dyn>) -> Result<Dyn, String> {
        self.run(None, args)
    }

    fn invoke_method(&self, name: &str, args: Vec<Dyn>) -> Result<Dyn, String> {
        self.run(Some(name), args)
    }

    fn has_method(&self, name: &str) -> bool {
        self.methods.contains(name)
    }
}

impl Drop for JsCallable {
    fn drop(&mut self) {
        RELEASED.with(|list| list.borrow_mut().push((self.token, self.id)));
    }
}

/// Lets go of what the library released, for the generation that runs now.
pub(crate) fn release<'js>(support: &Object<'js>) -> rquickjs::Result<()> {
    let generation = token_of(support)?;
    let ours: Vec<u64> = RELEASED.with(|list| {
        let mut list = list.borrow_mut();
        let (ours, others): (Vec<_>, Vec<_>) = list.drain(..).partition(|(owner, _)| *owner == generation);
        // What belongs to an older context is let go with that context.
        *list = others.into_iter().filter(|(owner, _)| *owner > generation).collect();
        ours.into_iter().map(|(_, id)| id).collect()
    });
    if ours.is_empty() {
        return Ok(());
    }
    let drop: Function<'js> = support.get("drop")?;
    for id in ours {
        drop.call::<_, ()>((id as f64,))?;
    }
    Ok(())
}

/// The number that names this context's table of references.
pub(crate) fn token_of<'js>(support: &Object<'js>) -> rquickjs::Result<u64> {
    Ok(support.get::<_, f64>("token")? as u64)
}

/// The call layer's value for a script value.
fn to_dyn<'js>(ctx: &Ctx<'js>, ids: &ScriptIds, support: &Object<'js>, value: Value<'js>) -> rquickjs::Result<Dyn> {
    if value.is_undefined() || value.is_null() {
        return Ok(Dyn::Nil);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(Dyn::Bool(flag));
    }
    if let Some(number) = value.as_number() {
        if number.is_finite() && number >= ScriptIds::HANDLE_BASE {
            return ids
                .from_script(number)
                .map(Dyn::Uint)
                .ok_or_else(|| Exception::throw_type(ctx, "a number that names no node"));
        }
        return Ok(Dyn::Float(number));
    }
    if let Some(text) = value.as_string() {
        return Ok(Dyn::Text(text.to_string()?));
    }
    if let Some(array) = value.as_array() {
        let mut items = Vec::with_capacity(array.len());
        for item in array.iter::<Value<'js>>() {
            items.push(to_dyn(ctx, ids, support, item?)?);
        }
        return Ok(Dyn::List(items));
    }
    let is_function = value.is_function();
    let Some(object) = value.as_object() else {
        return Err(Exception::throw_type(ctx, "a value the std library cannot take"));
    };
    let plain: bool = if is_function {
        false
    } else {
        support.get::<_, Function<'js>>("plain")?.call((object.clone(),))?
    };
    if plain {
        let mut entries = Vec::new();
        for key in object.keys::<String>() {
            let key = key?;
            let item: Value<'js> = object.get(key.as_str())?;
            entries.push((key, to_dyn(ctx, ids, support, item)?));
        }
        return Ok(Dyn::Map(entries));
    }
    let methods: Vec<String> = if is_function {
        Vec::new()
    } else {
        support.get::<_, Function<'js>>("methods")?.call((object.clone(),))?
    };
    let id: f64 = support.get::<_, Function<'js>>("keep")?.call((object.clone(),))?;
    Ok(Dyn::Object(Rc::new(JsCallable {
        id: id as u64,
        token: token_of(support)?,
        methods: methods.into_iter().collect(),
        ids: ids.clone(),
    })))
}

/// The script value for a value the library gave back.
fn from_dyn<'js>(ctx: &Ctx<'js>, ids: &ScriptIds, value: &Dyn) -> rquickjs::Result<Value<'js>> {
    Ok(match value {
        Dyn::Nil | Dyn::Object(_) => Value::new_null(ctx.clone()),
        Dyn::Bool(flag) => Value::new_bool(ctx.clone(), *flag),
        Dyn::Int(number) => Value::new_number(ctx.clone(), *number as f64),
        Dyn::Uint(number) => Value::new_number(ctx.clone(), ids.to_script(*number)),
        Dyn::Float(number) => Value::new_number(ctx.clone(), *number),
        Dyn::Text(text) => rquickjs::String::from_str(ctx.clone(), text)?.into_value(),
        Dyn::List(items) => {
            let array = Array::new(ctx.clone())?;
            for (index, item) in items.iter().enumerate() {
                array.set(index, from_dyn(ctx, ids, item)?)?;
            }
            array.into_value()
        }
        Dyn::Map(entries) => {
            let object = Object::new(ctx.clone())?;
            for (key, item) in entries {
                object.set(key.as_str(), from_dyn(ctx, ids, item)?)?;
            }
            object.into_value()
        }
    })
}

/// The error a failed call throws: an `Error` with the library's text and its ABI code in `code`.
fn throw<'js>(ctx: &Ctx<'js>, error: &StdError) -> rquickjs::Error {
    let thrown = Exception::from_message(ctx.clone(), &error.message).and_then(|exception| {
        exception.as_object().set("code", f64::from(error.code))?;
        Ok(exception)
    });
    match thrown {
        Ok(exception) => ctx.throw(exception.into_value()),
        Err(error) => error,
    }
}

thread_local! {
    static TOKENS: Cell<u64> = const { Cell::new(0) };
}

/// The native for each generation: `globalThis.__gnoblinStd`.
pub(crate) fn native() -> JsNative {
    JsNative::new(install)
}

fn install<'js>(ctx: &Ctx<'js>, ids: &ScriptIds) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(SUPPORT)?;
    let support: Object<'js> = ctx.globals().get("__gnoblinStdSupport")?;
    let token = TOKENS.with(|cell| {
        cell.set(cell.get() + 1);
        cell.get()
    });
    support.set("token", token as f64)?;
    let std = Object::new(ctx.clone())?;

    // The functions hold no script value: a closure that kept one would be a root the engine cannot see through, and the runtime
    // could not free its heap. They read what they need from the global scope each time.
    std.set(
        "load",
        Function::new(ctx.clone(), |ctx: Ctx<'js>| -> rquickjs::Result<()> {
            library()
                .map(drop)
                .map_err(|message| Exception::throw_message(&ctx, &message))
        })?,
    )?;

    let call_ids = ids.clone();
    std.set(
        "call",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, name: String, args: Array<'js>| -> rquickjs::Result<Value<'js>> {
                let library = library().map_err(|message| Exception::throw_message(&ctx, &message))?;
                let support: Object<'js> = ctx.globals().get("__gnoblinStdSupport")?;
                release(&support)?;
                let mut values = Vec::with_capacity(args.len());
                for arg in args.iter::<Value<'js>>() {
                    values.push(to_dyn(&ctx, &call_ids, &support, arg?)?);
                }
                USED.with(|cell| cell.set(true));
                // A call the script makes may ask the script back before it returns (a transition's commit runs the layout pass
                // against the staged state, and a script's layout is a callback): the way in is the call's own context, which
                // is already inside the script. A callback of a dispatch has its own way in, which stays.
                let answered = if ENTER.with(Cell::get).is_some() {
                    library.call(context(), &name, &values)
                } else {
                    with_enter(&CallEnter(&ctx), || library.call(context(), &name, &values))
                };
                let answer = answered.map_err(|error| throw(&ctx, &error))?;
                from_dyn(&ctx, &call_ids, &answer)
            },
        )?,
    )?;

    std.set(
        "functions",
        Function::new(ctx.clone(), || -> Vec<String> {
            StdLib::function_names().map(str::to_owned).collect()
        })?,
    )?;
    ctx.globals().set("__gnoblinStd", std)?;
    Ok(())
}
