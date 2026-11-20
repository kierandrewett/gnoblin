//! `gnoblin._std_base`: the base of the generated `gnoblin.std`, and how a value crosses.
//!
//! Everything typed in `gnoblin.std` is generated from the std table; what is here does not depend on the table. `call` is the one native
//! entry (`_gnoblin_std.call(name, *args)`, generic by table entry), [`dyn_value`] turns a script value into what the native layer takes (a
//! record class into a dict, a handle class into its id, a list into a list of those), [`listener`] and [`wrap`] turn what comes back into
//! the generated classes.
//!
//! Importing the module opens `libgnoblin-std.so.1`; when it is not installed the import raises `ImportError` with the text every runtime
//! gives (`gnoblin-std is not installed (looked for libgnoblin-std.so.1 in ...)`).

use std::sync::Mutex;

use pyo3::basic::CompareOp;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyCFunction, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use pyo3::{IntoPyObjectExt, wrap_pyfunction};

use crate::std_native;

/// The class `Registration` of the generated module, which a binding's `start` wraps what the library answered in. The module's sugar
/// installs it.
static REGISTRATION: Mutex<Option<Py<PyAny>>> = Mutex::new(None);

/// Names the class a built-in binding's `start` returns.
pub(crate) fn set_registration_class(class: Bound<'_, PyAny>) {
    *REGISTRATION.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(class.unbind());
}

/// A value the library names by a number: a collection, a workspace, a registration, a node.
///
/// The number is the one attribute the subclass names in `__slots__`.
#[pyclass(subclass, frozen, name = "Handle", module = "gnoblin._std_base")]
pub(crate) struct StdHandle;

#[pymethods]
impl StdHandle {
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self
    }

    #[pyo3(name = "_id")]
    fn number<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let first = slf.getattr("__slots__")?.get_item(0)?;
        slf.getattr(first.str()?)
    }

    fn _to_dyn<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        Self::number(slf)
    }

    fn __richcmp__(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let same = other.get_type().is(slf.get_type()) && Self::number(slf)?.eq(other.call_method0("_id")?)?;
        match op {
            CompareOp::Eq => same.into_py_any(py),
            CompareOp::Ne => (!same).into_py_any(py),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn __hash__(slf: &Bound<'_, Self>) -> PyResult<isize> {
        PyTuple::new(slf.py(), [slf.get_type().name()?.into_any(), Self::number(slf)?])?.hash()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(format!("<{} {}>", slf.get_type().name()?, Self::number(slf)?))
    }
}

/// The number behind a handle, a node or a plain integer.
#[pyfunction]
fn ident(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if let Ok(handle) = value.cast::<StdHandle>() {
        return Ok(StdHandle::number(handle)?.unbind());
    }
    if let Some(node) = value.getattr_opt("_gnoblin_node_id")?.filter(|node| !node.is_none()) {
        return Ok(py.get_type::<PyInt>().call1((node,))?.unbind());
    }
    Ok(py.get_type::<PyInt>().call1((value,))?.unbind())
}

/// A script value as the native layer takes it: records and handles as the plain values they carry.
#[pyfunction]
#[pyo3(name = "dyn")]
fn dyn_value(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if value.is_none()
        || value.is_instance_of::<PyBool>()
        || value.is_instance_of::<PyInt>()
        || value.is_instance_of::<PyFloat>()
        || value.is_instance_of::<PyString>()
    {
        return Ok(value.clone().unbind());
    }
    if let Some(convert) = value.getattr_opt("_to_dyn")?.filter(|convert| !convert.is_none()) {
        return Ok(convert.call0()?.unbind());
    }
    if let Some(node) = value.getattr_opt("_gnoblin_node_id")?.filter(|node| !node.is_none()) {
        return Ok(py.get_type::<PyInt>().call1((node,))?.unbind());
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let converted = PyDict::new(py);
        for (key, item) in dict.iter() {
            converted.set_item(key, dyn_value(py, &item)?)?;
        }
        return Ok(converted.into_any().unbind());
    }
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        let converted = PyList::empty(py);
        for item in value.try_iter()? {
            converted.append(dyn_value(py, &item?)?)?;
        }
        return Ok(converted.into_any().unbind());
    }
    Ok(value.clone().unbind())
}

/// `cls(raw)` unless the library said there is none (`None`).
#[pyfunction]
fn wrap(py: Python<'_>, class: &Bound<'_, PyAny>, raw: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if raw.is_none() {
        Ok(py.None())
    } else {
        Ok(class.call1((raw,))?.unbind())
    }
}

/// Wraps a script's function so the records the library passes arrive as the generated classes.
#[pyfunction]
fn listener<'py>(
    py: Python<'py>,
    function: &Bound<'py, PyAny>,
    converters: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    if !function.is_callable() {
        return Err(PyTypeError::new_err(format!(
            "expected a function to call, got {}",
            function.get_type().name()?
        )));
    }
    let function = function.clone().unbind();
    let converters: Vec<Py<PyAny>> = converters
        .try_iter()?
        .map(|item| Ok(item?.unbind()))
        .collect::<PyResult<_>>()?;
    Ok(PyCFunction::new_closure(
        py,
        None,
        None,
        move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<Py<PyAny>> {
            let py = args.py();
            // The library's arguments past the converters are not the script's: `zip` stops at the shorter, as the spelling always did.
            let mut converted = Vec::new();
            for (arg, convert) in args.iter().zip(&converters) {
                converted.push(if convert.is_none(py) {
                    arg
                } else {
                    convert.bind(py).call1((arg,))?
                });
            }
            Ok(function.bind(py).call1(PyTuple::new(py, converted)?)?.unbind())
        },
    )?
    .into_any())
}

/// The member of `enumeration` for `value`, or the number itself when the library is newer than this module.
#[pyfunction]
fn kind(py: Python<'_>, enumeration: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    match enumeration.call1((value,)) {
        Ok(member) => Ok(member.unbind()),
        Err(error) if error.is_instance_of::<PyValueError>(py) => Ok(value.clone().unbind()),
        Err(error) => Err(error),
    }
}

/// One typed option as the table carries it: its `STD_VALUE_*` kind, its name and the field that holds the value.
fn option<'py>(
    py: Python<'py>,
    name: &str,
    value: &Bound<'py, PyAny>,
    shape: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyDict>> {
    let kind_number: i64 = shape.get_item(0)?.extract()?;
    let field: String = shape.get_item(1)?.extract()?;
    let value = if kind_number == 1 {
        i32::from(value.is_truthy()?).into_bound_py_any(py)?
    } else if field == "node" {
        ident(py, value)?.into_bound(py)
    } else if field == "number" {
        py.get_type::<PyFloat>().call1((value,))?
    } else {
        value.clone()
    };
    let native = PyDict::new(py);
    native.set_item("kind", kind_number)?;
    native.set_item("name", name)?;
    native.set_item(field, value)?;
    Ok(native)
}

/// What an option or an action answers: `None` for nothing, else a boolean, a number, a node or text.
fn option_value<'py>(py: Python<'py>, native: &Bound<'py, PyAny>, whole: bool) -> PyResult<Bound<'py, PyAny>> {
    let kind_number: i64 = native.get_item("kind")?.extract()?;
    match kind_number {
        0 => Ok(py.None().into_bound(py)),
        1 => Ok(PyBool::new(py, native.get_item("flag")?.is_truthy()?)
            .to_owned()
            .into_any()),
        2 | 3 if whole => py.get_type::<PyInt>().call1((native.get_item("number")?,)),
        2 | 3 => native.get_item("number"),
        4 => native.get_item("node"),
        _ => native.get_item("text"),
    }
}

/// The options a script gave, as the table carries them, leaving out the ones it did not.
fn given_options<'py>(
    py: Python<'py>,
    options: &Bound<'py, PyDict>,
    shapes: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyList>> {
    let given = PyList::empty(py);
    for (name, value) in options.iter() {
        if value.is_none() {
            continue;
        }
        let name: String = name.extract()?;
        given.append(option(py, &name, &value, &shapes.get_item(&name)?)?)?;
    }
    Ok(given)
}

/// A layout the library runs: a kind of `gnoblin.std.LAYOUT_KINDS` with its options and actions, written by the generator.
///
/// Options given before `attach` are kept here; after it they are the workspace's, read and written through the library.
#[pyclass(subclass, name = "BuiltinLayout", module = "gnoblin._std_base")]
pub(crate) struct LayoutBase {
    options: Py<PyDict>,
    #[pyo3(get, set)]
    workspace: Py<PyAny>,
}

#[pymethods]
impl LayoutBase {
    #[classattr]
    const KIND: i64 = 0;

    #[classattr]
    #[pyo3(name = "SHAPES")]
    fn shapes(py: Python<'_>) -> Bound<'_, PyDict> {
        PyDict::new(py)
    }

    #[classattr]
    #[pyo3(name = "ACTION_SHAPES")]
    fn action_shapes(py: Python<'_>) -> Bound<'_, PyDict> {
        PyDict::new(py)
    }

    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(py: Python<'_>, _args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self {
            options: PyDict::new(py).unbind(),
            workspace: py.None(),
        }
    }

    /// The options the subclass was given, by name; the generated `__init__` passes them as one dict.
    fn __init__(slf: &Bound<'_, Self>, options: Bound<'_, PyDict>) {
        slf.borrow_mut().options = options.unbind();
    }

    /// Gives `workspace` this layout; its options and actions act on that workspace from then on.
    fn attach<'py>(slf: &Bound<'py, Self>, workspace: &Bound<'py, PyAny>) -> PyResult<Bound<'py, Self>> {
        let py = slf.py();
        let options = slf.borrow().options.clone_ref(py);
        let given = given_options(py, options.bind(py), &slf.getattr("SHAPES")?)?;
        let ws = ident(py, workspace)?;
        std_native::call(
            py,
            "workspace_set_layout_kind",
            &PyTuple::new(py, [ws.into_bound(py), slf.getattr("KIND")?, given.into_any()])?,
        )?;
        slf.borrow_mut().workspace = workspace.clone().unbind();
        Ok(slf.clone())
    }

    fn _get<'py>(slf: &Bound<'py, Self>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let workspace = slf.borrow().workspace.clone_ref(py).into_bound(py);
        if workspace.is_none() {
            return slf
                .borrow()
                .options
                .bind(py)
                .get_item(name)?
                .map_or_else(|| Ok(py.None().into_bound(py)), Ok);
        }
        let ws = ident(py, &workspace)?;
        let native = std_native::call(
            py,
            "workspace_layout_get",
            &PyTuple::new(py, [ws.into_bound(py), PyString::new(py, name).into_any()])?,
        )?;
        let shape = slf.getattr("SHAPES")?.get_item(name)?;
        option_value(py, &native, shape.get_item(2)?.is_truthy()?)
    }

    fn _set(slf: &Bound<'_, Self>, name: &str, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let py = slf.py();
        let (options, workspace) = {
            let this = slf.borrow();
            (this.options.clone_ref(py), this.workspace.clone_ref(py))
        };
        options.bind(py).set_item(name, value)?;
        let workspace = workspace.into_bound(py);
        if !workspace.is_none() && !value.is_none() {
            let shape = slf.getattr("SHAPES")?.get_item(name)?;
            let ws = ident(py, &workspace)?;
            std_native::call(
                py,
                "workspace_layout_set",
                &PyTuple::new(py, [ws.into_bound(py), option(py, name, value, &shape)?.into_any()])?,
            )?;
        }
        Ok(())
    }

    fn _act<'py>(slf: &Bound<'py, Self>, name: &str, args: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let workspace = slf.borrow().workspace.clone_ref(py).into_bound(py);
        if workspace.is_none() {
            return Err(PyRuntimeError::new_err(format!(
                "{name}: the layout is not attached to a workspace"
            )));
        }
        let shapes = slf.getattr("ACTION_SHAPES")?;
        let given = PyList::empty(py);
        for pair in args.try_iter()? {
            let pair = pair?;
            let (arg, value): (String, Bound<'py, PyAny>) = pair.extract()?;
            if !value.is_none() {
                given.append(option(py, &arg, &value, &shapes.get_item(format!("{name}.{arg}"))?)?)?;
            }
        }
        let ws = ident(py, &workspace)?;
        let native = std_native::call(
            py,
            "workspace_layout_act",
            &PyTuple::new(
                py,
                [ws.into_bound(py), PyString::new(py, name).into_any(), given.into_any()],
            )?,
        )?;
        let whole = match shapes.get_item(format!("{name}->")) {
            Ok(answer) => answer.get_item(2)?.is_truthy()?,
            Err(error) if error.is_instance_of::<pyo3::exceptions::PyKeyError>(py) => false,
            Err(error) => return Err(error),
        };
        option_value(py, &native, whole)
    }
}

/// A pointer binding the library runs: a kind of `BINDING_KINDS` with its options, written by the generator.
#[pyclass(subclass, name = "BuiltinBinding", module = "gnoblin._std_base")]
pub(crate) struct BindingBase {
    options: Py<PyDict>,
}

#[pymethods]
impl BindingBase {
    #[classattr]
    const KIND: i64 = 0;

    #[classattr]
    #[pyo3(name = "SHAPES")]
    fn shapes(py: Python<'_>) -> Bound<'_, PyDict> {
        PyDict::new(py)
    }

    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(py: Python<'_>, _args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self {
            options: PyDict::new(py).unbind(),
        }
    }

    /// The options the subclass was given, by name; the generated `__init__` passes them as one dict.
    fn __init__(slf: &Bound<'_, Self>, options: Bound<'_, PyDict>) {
        slf.borrow_mut().options = options.unbind();
    }

    /// Claims the binding's trigger. `remove()` on the result ends it.
    fn start<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let options = slf.borrow().options.clone_ref(py);
        let given = given_options(py, options.bind(py), &slf.getattr("SHAPES")?)?;
        let raw = std_native::call(
            py,
            "binding_builtin",
            &PyTuple::new(py, [slf.getattr("KIND")?, given.into_any()])?,
        )?;
        let class = REGISTRATION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|class| class.clone_ref(py));
        match class {
            Some(class) => class.bind(py).call1((raw,)),
            None => Err(PyRuntimeError::new_err(
                "gnoblin.std has not named its Registration class",
            )),
        }
    }
}

/// Fills the module `gnoblin._std_base`; fails with the library's own text when `libgnoblin-std.so.1` is not installed.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    std_native::load()?;
    module.add("call", py.import("_gnoblin_std")?.getattr("call")?)?;
    module.add("functions", py.import("_gnoblin_std")?.getattr("functions")?)?;
    module.add_class::<StdHandle>()?;
    module.add_class::<LayoutBase>()?;
    module.add_class::<BindingBase>()?;
    module.add_function(wrap_pyfunction!(ident, module)?)?;
    module.add_function(wrap_pyfunction!(dyn_value, module)?)?;
    module.add_function(wrap_pyfunction!(wrap, module)?)?;
    module.add_function(wrap_pyfunction!(listener, module)?)?;
    module.add_function(wrap_pyfunction!(kind, module)?)?;
    Ok(())
}
