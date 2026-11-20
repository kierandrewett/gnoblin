//! The built-in module `_gnoblin_objects`: the object protocol of the libraries (`gnoblin-object`), for a script.
//!
//! Both libraries serve their classes through one table of generic calls (`create`, `get`, `set`, `call`, `listen`, `implement`, ...), and
//! this module is a script's door to it, generic on purpose: it knows no class. The classes a shell uses (`gnoblin.objects.std.EntryModel`,
//! `FocusPolicy`) are generated from the catalog into the package and call these functions with the ids the catalog gave, so a class added to
//! a catalog changes no line here.
//!
//! [`ObjectNamespace`] is a library's table for one namespace and [`ObjectHandle`] a script's reference to one object of it. A reference ends when the
//! script's last one goes: the release is queued and made on the shell thread at the next call, because a handle may be dropped by any
//! thread of the script and the library may be called from the shell thread only.
//!
//! Values cross as Python's own. A script gives a `dict` for a record (its values, in the catalog's order), an `enum.Enum` member for an
//! enum value, a [`ObjectHandle`] for an object, `bytes` for bytes, a list or tuple for a list and a callable for a closure. A record comes back as
//! a `tuple` of its fields, an enum value as its `int`, an object as a [`ObjectHandle`] and a list as a `list`; the generated class that asked
//! for the value knows which it is from the member's type. A closure is called from the core's dispatch, which the runtime runs every
//! turn, with the member's id and a list of arguments.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use gnoblin_core::ErrorKind;
use gnoblin_core_abi::{self as abi, GnoblinObjectApi, GnoblinStr, MEMBER_METHOD, MEMBER_PROPERTY, MEMBER_SIGNAL};
use gnoblin_object::client::Api;
use gnoblin_object::{Callable, ClassId, Closure, Fail, MemberId, Obj, Res, Timing, Val};
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use crate::callback::report;
use crate::errors::to_py;
use crate::generation::{self, WRONG_THREAD, current_ctx, with_state};
use crate::std_native::library;

/// The namespaces the core serves; every other is the std library's.
const CORE_NAMESPACES: &[&str] = &["gnoblin.model"];

/// References the script dropped, to be released on the shell thread.
static PENDING: Mutex<Vec<(Api, Obj)>> = Mutex::new(Vec::new());

/// Releases what the script let go of. Only the shell thread calls it.
fn drain() {
    let pending = std::mem::take(&mut *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    for (api, obj) in pending {
        // A generation that ended took its objects with it: the refusal is not the script's to hear.
        let _ = api.release(obj);
    }
}

/// The exception for a failed call: the kinds of the core's, with the library's own text.
fn failure(fail: &Fail) -> PyErr {
    to_py(Some(ErrorKind::from_code(fail.code)), &fail.message)
}

/// The shell thread's generation, after the references the script dropped were released. Any other thread is refused.
fn enter() -> PyResult<abi::GnoblinCtx> {
    let ctx = current_ctx();
    if ctx == 0 {
        return Err(to_py(None, WRONG_THREAD));
    }
    drain();
    Ok(ctx)
}

/// A library's table for one namespace.
#[pyclass(frozen, name = "Namespace", module = "_gnoblin_objects")]
struct ObjectNamespace {
    name: String,
    raw: &'static GnoblinObjectApi,
    last_error: fn() -> String,
    /// The name of each class, read from the catalog the first time one is asked for.
    classes: Mutex<Option<HashMap<ClassId, String>>>,
}

impl ObjectNamespace {
    /// The protocol of this namespace for the generation the script runs in.
    fn api(&self) -> PyResult<Api> {
        Ok(Api::new(self.raw, enter()?, self.last_error))
    }

    fn class_name(&self, api: Api, class: ClassId) -> Res<String> {
        let mut classes = self.classes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if classes.is_none() {
            *classes = Some(
                api.catalog()?
                    .into_iter()
                    .filter(|item| item.entry == abi::CATALOG_CLASS)
                    .map(|item| (item.class, item.name))
                    .collect(),
            );
        }
        classes
            .as_ref()
            .and_then(|names| names.get(&class).cloned())
            .ok_or_else(|| Fail::err(format!("`{}` has no class {class}", self.name)))
    }
}

/// What a conversion needs: the namespace a value belongs to, and the generation's protocol, to make a handle of an object that arrives.
struct ConvertCx {
    namespace: Py<ObjectNamespace>,
    api: Api,
}

impl ConvertCx {
    /// Another conversion context for the same namespace and generation.
    fn dup(&self, py: Python<'_>) -> ConvertCx {
        ConvertCx {
            namespace: self.namespace.clone_ref(py),
            api: self.api,
        }
    }
}

/// A script's reference to one object of a library.
#[pyclass(frozen, name = "Handle", module = "_gnoblin_objects")]
struct ObjectHandle {
    cx: ConvertCx,
    class: ClassId,
    obj: Obj,
}

impl Drop for ObjectHandle {
    fn drop(&mut self) {
        PENDING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((self.cx.api, self.obj));
    }
}

/// The script's `value` as a [`Val`].
fn to_val(value: &Bound<'_, PyAny>, cx: &ConvertCx) -> PyResult<Val> {
    static ENUM: PyOnceLock<Py<PyAny>> = PyOnceLock::new();
    let py = value.py();
    if value.is_none() {
        return Ok(Val::None);
    }
    if let Ok(handle) = value.cast::<ObjectHandle>() {
        let handle = handle.get();
        return Ok(Val::Object {
            class: handle.class,
            obj: handle.obj,
        });
    }
    let enum_type = ENUM.get_or_try_init(py, || py.import("enum")?.getattr("Enum").map(Bound::unbind))?;
    if value.is_instance(enum_type.bind(py))? {
        let id: u32 = value
            .getattr("value")?
            .extract()
            .map_err(|_| PyTypeError::new_err("only an enum whose values are integers can cross to the library"))?;
        return Ok(Val::Enum { class: 0, value: id });
    }
    if value.is_instance_of::<PyBool>() {
        return Ok(Val::Bool(value.extract()?));
    }
    if value.is_instance_of::<PyInt>() {
        return Ok(match value.extract::<i64>() {
            Ok(number) => Val::Int(number),
            Err(_) => Val::Uint(value.extract()?),
        });
    }
    if value.is_instance_of::<PyFloat>() {
        return Ok(Val::Double(value.extract()?));
    }
    if value.is_instance_of::<PyString>() {
        return Ok(Val::Text(value.extract()?));
    }
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(Val::Bytes(bytes.as_bytes().to_vec()));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let fields = dict
            .values()
            .iter()
            .map(|field| to_val(&field, cx))
            .collect::<PyResult<Vec<_>>>()?;
        return Ok(Val::Record { class: 0, fields });
    }
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        let mut items = Vec::new();
        for item in value.try_iter()? {
            items.push(to_val(&item?, cx)?);
        }
        return Ok(Val::List(items));
    }
    if value.is_callable() {
        let closure: Closure = std::rc::Rc::new(PyClosure {
            function: value.clone().unbind(),
            cx: cx.dup(py),
        });
        return Ok(Val::Closure(closure));
    }
    Err(PyTypeError::new_err(format!(
        "a {} cannot cross to the library: records, enum values and library objects are made by the generated classes",
        value.get_type().name()?
    )))
}

/// A [`Val`] the library sent, as the script's value. An object in it becomes a [`ObjectHandle`] that holds the reference the library handed out.
fn from_val<'py>(py: Python<'py>, value: &Val, cx: &ConvertCx) -> PyResult<Bound<'py, PyAny>> {
    match value {
        Val::None | Val::Closure(_) => Ok(py.None().into_bound(py)),
        Val::Bool(flag) => flag.into_bound_py_any(py),
        Val::Int(number) => number.into_bound_py_any(py),
        Val::Uint(number) | Val::Node(number) => number.into_bound_py_any(py),
        Val::Double(number) => number.into_bound_py_any(py),
        Val::Text(text) | Val::Secret(text) => text.into_bound_py_any(py),
        Val::Bytes(bytes) => Ok(PyBytes::new(py, bytes).into_any()),
        Val::List(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(from_val(py, item, cx)?)?;
            }
            Ok(list.into_any())
        }
        Val::Record { fields, .. } => {
            let items = fields
                .iter()
                .map(|field| from_val(py, field, cx))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, items)?.into_any())
        }
        Val::Enum { value, .. } => value.into_bound_py_any(py),
        Val::Object { class, obj } => {
            let class = if *class == 0 {
                cx.api.class_of(*obj).map_err(|fail| failure(&fail))?
            } else {
                *class
            };
            Py::new(
                py,
                ObjectHandle {
                    cx: cx.dup(py),
                    class,
                    obj: *obj,
                },
            )
            .map(|handle| handle.into_bound(py).into_any())
        }
    }
}

/// What the library calls when a script gave it a function or an object that implements an interface.
struct PyClosure {
    function: Py<PyAny>,
    cx: ConvertCx,
}

impl Callable for PyClosure {
    fn call(&self, member: MemberId, args: &[Val]) -> Res<Val> {
        Python::attach(|py| {
            let run = || -> PyResult<Val> {
                let list = PyList::empty(py);
                for arg in args {
                    list.append(from_val(py, arg, &self.cx)?)?;
                }
                let answer = self.function.bind(py).call1((member, list))?;
                to_val(&answer, &self.cx)
            };
            run().map_err(|error| {
                report(py, "an object callback", &error);
                Fail::new(abi::E_PANIC, error.to_string())
            })
        })
    }
}

fn inits_of(pairs: &Bound<'_, PyAny>, cx: &ConvertCx) -> PyResult<Vec<(MemberId, Val)>> {
    let mut inits = Vec::new();
    for pair in pairs.try_iter()? {
        let (member, value): (MemberId, Bound<'_, PyAny>) = pair?.extract()?;
        inits.push((member, to_val(&value, cx)?));
    }
    Ok(inits)
}

fn args_of(args: &Bound<'_, PyAny>, cx: &ConvertCx) -> PyResult<Vec<Val>> {
    let mut values = Vec::new();
    for arg in args.try_iter()? {
        values.push(to_val(&arg?, cx)?);
    }
    Ok(values)
}

#[pymethods]
impl ObjectNamespace {
    /// `[class, *members]`: the id of the class `class_name` and of each of `names` (`p:text`, `m:insert`, `s:changed`).
    fn ids(&self, class_name: &str, names: Vec<String>) -> PyResult<Vec<u32>> {
        let api = self.api()?;
        let lookup = || -> Res<Vec<u32>> {
            let class = api.class(class_name)?;
            let mut found = vec![class];
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
                found.push(api.member(class, kind, member)?);
            }
            Ok(found)
        };
        lookup().map_err(|fail| failure(&fail))
    }

    /// Makes an object of the class `class` with the properties `inits` (`(member, value)` pairs) set.
    fn create(slf: &Bound<'_, Self>, class: ClassId, inits: &Bound<'_, PyAny>) -> PyResult<ObjectHandle> {
        let cx = ConvertCx {
            namespace: slf.clone().unbind(),
            api: slf.get().api()?,
        };
        let inits = inits_of(inits, &cx)?;
        let obj = cx.api.create(class, &inits).map_err(|fail| failure(&fail))?;
        Ok(ObjectHandle { cx, class, obj })
    }

    /// Calls a function of the class `class`.
    fn call_static<'py>(
        slf: &Bound<'py, Self>,
        class: ClassId,
        member: MemberId,
        args: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let cx = ConvertCx {
            namespace: slf.clone().unbind(),
            api: slf.get().api()?,
        };
        let args = args_of(args, &cx)?;
        let value = cx
            .api
            .call_static(class, member, &args)
            .map_err(|fail| failure(&fail))?;
        from_val(slf.py(), &value, &cx)
    }

    /// Makes the object of the interface `class` whose methods `members` run `function(member, args)`.
    fn implement(
        slf: &Bound<'_, Self>,
        class: ClassId,
        members: Vec<MemberId>,
        function: &Bound<'_, PyAny>,
    ) -> PyResult<ObjectHandle> {
        if !function.is_callable() {
            return Err(PyTypeError::new_err("an implementation needs a function to call"));
        }
        let cx = ConvertCx {
            namespace: slf.clone().unbind(),
            api: slf.get().api()?,
        };
        let closure = PyClosure {
            function: function.clone().unbind(),
            cx: cx.dup(slf.py()),
        };
        let obj = cx
            .api
            .implement(class, &members, move |member, args| closure.call(member, args))
            .map_err(|fail| failure(&fail))?;
        Ok(ObjectHandle { cx, class, obj })
    }

    /// Ends a listener `handle.listen` returned.
    fn unlisten(&self, registration: u64) -> PyResult<()> {
        self.api()?.unlisten(registration).map_err(|fail| failure(&fail))
    }
}

#[pymethods]
impl ObjectHandle {
    /// The name of the object's class in the catalog.
    #[getter]
    fn class_name(&self) -> PyResult<String> {
        enter()?;
        self.cx
            .namespace
            .get()
            .class_name(self.cx.api, self.class)
            .map_err(|fail| failure(&fail))
    }

    /// The value of the property `member`.
    fn get<'py>(&self, py: Python<'py>, member: MemberId) -> PyResult<Bound<'py, PyAny>> {
        enter()?;
        let value = self.cx.api.get(self.obj, member).map_err(|fail| failure(&fail))?;
        from_val(py, &value, &self.cx)
    }

    /// Writes the property `member`.
    fn set(&self, member: MemberId, value: &Bound<'_, PyAny>) -> PyResult<()> {
        enter()?;
        let value = to_val(value, &self.cx)?;
        self.cx.api.set(self.obj, member, &value).map_err(|fail| failure(&fail))
    }

    /// Writes the property `member` over a time: `timing` is `(seconds, delay, tween, samples)`.
    fn set_timed(&self, member: MemberId, value: &Bound<'_, PyAny>, timing: (f64, f64, u64, Vec<f64>)) -> PyResult<()> {
        enter()?;
        let value = to_val(value, &self.cx)?;
        let timing = Timing {
            seconds: timing.0,
            delay: timing.1,
            tween: timing.2,
            samples: timing.3,
        };
        self.cx
            .api
            .set_timed(self.obj, member, &value, &timing)
            .map_err(|fail| failure(&fail))
    }

    /// Calls the method `member`.
    fn call<'py>(&self, py: Python<'py>, member: MemberId, args: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        enter()?;
        let args = args_of(args, &self.cx)?;
        let value = self
            .cx
            .api
            .call(self.obj, member, &args)
            .map_err(|fail| failure(&fail))?;
        from_val(py, &value, &self.cx)
    }

    /// Calls `function(member, args)` from the library's turn each time the signal or property `member` is told, and returns the number
    /// that ends the listener.
    fn listen(&self, member: MemberId, function: &Bound<'_, PyAny>) -> PyResult<u64> {
        enter()?;
        if !function.is_callable() {
            return Err(PyTypeError::new_err("a listener needs a function to call"));
        }
        let closure = PyClosure {
            function: function.clone().unbind(),
            cx: self.cx.dup(function.py()),
        };
        self.cx
            .api
            .listen(self.obj, member, move |member, args| closure.call(member, args))
            .map_err(|fail| failure(&fail))
    }

    /// Ends the object and its children, whoever else holds it.
    fn destroy(&self) -> PyResult<()> {
        enter()?;
        self.cx.api.destroy(self.obj).map_err(|fail| failure(&fail))
    }

    fn __repr__(&self) -> String {
        format!("<ObjectHandle class {} object {:#x}>", self.class, self.obj)
    }
}

/// A library's table for a namespace, and the function that reads the text of its last failure.
type Served = (&'static GnoblinObjectApi, fn() -> String);

/// The table of `namespace`, which opens the library that serves it.
fn raw_for(namespace: &str) -> PyResult<Served> {
    let ctx = enter()?;
    if CORE_NAMESPACES.contains(&namespace) {
        let api = with_state(|state| state.api).ok_or_else(|| to_py(None, generation::WRONG_THREAD))?;
        let mut table: *const GnoblinObjectApi = core::ptr::null();
        // SAFETY: `namespace` is borrowed for the call and `table` is a live local the core writes.
        let code = unsafe { (api.object_api)(ctx, GnoblinStr::borrowed(namespace), &raw mut table) };
        if code != abi::E_OK {
            return Err(to_py(
                Some(ErrorKind::from_code(code)),
                &gnoblin_core::private::last_error(),
            ));
        }
        // SAFETY: on success the core wrote a pointer to its static table, which is never unmapped.
        let raw = unsafe { table.as_ref() }.ok_or_else(|| to_py(None, "the core gave no object table"))?;
        return Ok((raw, gnoblin_core::private::last_error));
    }
    let library = library()?;
    let raw = library
        .object_api(ctx, namespace)
        .map_err(|error| to_py(Some(ErrorKind::from_code(error.code)), &error.message))?;
    Ok((raw, gnoblin_std_dyn::last_error_text))
}

/// Opens the namespace `namespace` (`gnoblin.std`), which asks the library that serves it. Raises `ImportError` when the std library is
/// not installed.
#[pyfunction]
fn open(namespace: &str) -> PyResult<ObjectNamespace> {
    let (raw, last_error) = raw_for(namespace)?;
    Ok(ObjectNamespace {
        name: namespace.to_owned(),
        raw,
        last_error,
        classes: Mutex::new(None),
    })
}

/// The module.
#[pymodule]
pub(crate) fn _gnoblin_objects(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_class::<ObjectNamespace>()?;
    module.add_class::<ObjectHandle>()?;
    Ok(())
}
