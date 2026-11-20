//! The built-in module `_gnoblin_std`: every function of `libgnoblin-std`'s table, by name.
//!
//! It is generic on purpose, as `_gnoblin_core` is. `call("collection_len", handle)` converts the arguments to the call layer's
//! [`Dyn`] (`gnoblin-std-dyn`, whose per-function code is generated from the table), calls the installed library for the generation
//! `_gnoblin_core` acts on, and converts what comes back. The typed classes a script uses (`gnoblin.std.Workspace` and the rest) are
//! generated from the same table into the generated `gnoblin/std.py`, so a function added to the table changes no line here.
//!
//! The library is opened the first time a script asks (`load`), never when the runtime starts: a shell that does not use std runs on
//! a machine that does not have it.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_std_dyn::{Callable, Dyn, StdError, StdLib};
use pyo3::exceptions::{PyImportError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use crate::callback::report;
use crate::errors::to_py;
use crate::generation::{self, current_ctx, with_state};

thread_local! {
    /// The library, once opened: it belongs to the shell thread like every handle of a generation.
    static LIBRARY: RefCell<Option<Rc<StdLib>>> = const { RefCell::new(None) };
}

/// The attribute a `Node` of the package carries: its id, as an `int`.
const NODE_ID_ATTRIBUTE: &str = "_gnoblin_node_id";

/// The library, opened now if it was not.
fn library() -> PyResult<Rc<StdLib>> {
    if let Some(library) = LIBRARY.with(|slot| slot.borrow().clone()) {
        return Ok(library);
    }
    // Any address inside the core says where the core is: the table's own functions live there.
    let anchor = with_state(|state| state.api.log as *const std::ffi::c_void)
        .ok_or_else(|| to_py(None, generation::WRONG_THREAD))?;
    let library = Rc::new(StdLib::load(anchor).map_err(PyImportError::new_err)?);
    LIBRARY.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&library)));
    Ok(library)
}

/// A script's function or object, called back by the library.
struct PyCallable(Py<PyAny>);

impl PyCallable {
    fn run(&self, what: &str, args: Vec<Dyn>, method: Option<&str>) -> Result<Dyn, String> {
        Python::attach(|py| {
            let outcome = (|| -> PyResult<Dyn> {
                let mut values: Vec<Bound<'_, PyAny>> = Vec::with_capacity(args.len());
                for arg in &args {
                    values.push(from_dyn(py, arg)?);
                }
                let target = self.0.bind(py);
                let function = match method {
                    Some(name) => target.getattr(name)?,
                    None => target.clone(),
                };
                let answer = function.call1(PyTuple::new(py, values)?)?;
                to_dyn(&answer)
            })();
            outcome.map_err(|error| {
                report(py, what, &error);
                error.to_string()
            })
        })
    }
}

impl Callable for PyCallable {
    fn invoke(&self, args: Vec<Dyn>) -> Result<Dyn, String> {
        self.run("std callback", args, None)
    }

    fn invoke_method(&self, name: &str, args: Vec<Dyn>) -> Result<Dyn, String> {
        self.run(&format!("std callback `{name}`"), args, Some(name))
    }

    fn has_method(&self, name: &str) -> bool {
        Python::attach(|py| self.0.bind(py).getattr(name).is_ok_and(|member| member.is_callable()))
    }
}

/// The call layer's value for a Python object.
fn to_dyn(value: &Bound<'_, PyAny>) -> PyResult<Dyn> {
    if value.is_none() {
        return Ok(Dyn::Nil);
    }
    if value.is_instance_of::<PyBool>() {
        return Ok(Dyn::Bool(value.extract()?));
    }
    if let Ok(id) = value.getattr(NODE_ID_ATTRIBUTE) {
        return Ok(Dyn::Uint(id.extract()?));
    }
    if value.is_instance_of::<PyInt>() {
        return Ok(match value.extract::<i64>() {
            Ok(number) => Dyn::Int(number),
            Err(_) => Dyn::Uint(value.extract()?),
        });
    }
    if value.is_instance_of::<PyFloat>() {
        return Ok(Dyn::Float(value.extract()?));
    }
    if value.is_instance_of::<PyString>() {
        return Ok(Dyn::Text(value.extract()?));
    }
    if let Ok(list) = value.cast::<PyList>() {
        return list
            .iter()
            .map(|item| to_dyn(&item))
            .collect::<PyResult<_>>()
            .map(Dyn::List);
    }
    if let Ok(tuple) = value.cast::<PyTuple>() {
        return tuple
            .iter()
            .map(|item| to_dyn(&item))
            .collect::<PyResult<_>>()
            .map(Dyn::List);
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let mut entries = Vec::with_capacity(dict.len());
        for (key, item) in dict.iter() {
            let key: String = key
                .extract()
                .map_err(|_| PyTypeError::new_err("a record's field names are strings"))?;
            entries.push((key, to_dyn(&item)?));
        }
        return Ok(Dyn::Map(entries));
    }
    // A function, or an object whose methods the library calls.
    Ok(Dyn::Object(Rc::new(PyCallable(value.clone().unbind()))))
}

/// The Python object for a value the library gave back.
fn from_dyn<'py>(py: Python<'py>, value: &Dyn) -> PyResult<Bound<'py, PyAny>> {
    use pyo3::IntoPyObjectExt;
    match value {
        Dyn::Nil => Ok(py.None().into_bound(py)),
        Dyn::Bool(flag) => flag.into_bound_py_any(py),
        Dyn::Int(number) => number.into_bound_py_any(py),
        Dyn::Uint(number) => number.into_bound_py_any(py),
        Dyn::Float(number) => number.into_bound_py_any(py),
        Dyn::Text(text) => text.into_bound_py_any(py),
        Dyn::List(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(from_dyn(py, item)?)?;
            }
            Ok(list.into_any())
        }
        Dyn::Map(entries) => {
            let dict = PyDict::new(py);
            for (key, item) in entries {
                dict.set_item(key, from_dyn(py, item)?)?;
            }
            Ok(dict.into_any())
        }
        Dyn::Object(_) => Ok(py.None().into_bound(py)),
    }
}

/// The exception for a failed call: the kinds of the core's, with the library's own text.
fn failure(error: &StdError) -> PyErr {
    to_py(Some(gnoblin_core::ErrorKind::from_code(error.code)), &error.message)
}

/// Opens `libgnoblin-std.so.1`, or raises `ImportError` naming the file and where it was looked for. The package's `gnoblin.std`
/// calls it at import, so a shell that imports std without the library fails at the import.
#[pyfunction]
fn load() -> PyResult<()> {
    library().map(drop)
}

/// Calls the table function `name` with `args`, for the generation `_gnoblin_core` acts on.
#[pyfunction]
#[pyo3(signature = (name, *args))]
fn call<'py>(py: Python<'py>, name: &str, args: &Bound<'py, PyTuple>) -> PyResult<Bound<'py, PyAny>> {
    let library = library()?;
    let values = args.iter().map(|arg| to_dyn(&arg)).collect::<PyResult<Vec<_>>>()?;
    let ctx = current_ctx();
    if ctx == 0 {
        return Err(to_py(None, "no Gnoblin shell generation is running"));
    }
    // The library may call a listener back from inside nothing of this call (it never does), but a script object is Python code:
    // hold no borrow of the interpreter's state while it runs.
    let answer = library.call(ctx, name, &values).map_err(|error| failure(&error))?;
    from_dyn(py, &answer)
}

/// The names of the table functions this build knows, in table order.
#[pyfunction]
fn functions() -> Vec<&'static str> {
    StdLib::function_names().collect()
}

/// The module.
#[pymodule]
pub(crate) fn _gnoblin_std(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(load, module)?)?;
    module.add_function(wrap_pyfunction!(call, module)?)?;
    module.add_function(wrap_pyfunction!(functions, module)?)?;
    Ok(())
}
