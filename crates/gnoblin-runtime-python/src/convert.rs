//! Values across the boundary: a property of the tree as a Python object, and a Python object as a value to write.
//!
//! The tree's kinds map to the Python types a reader expects: `bool`, `int`, `float`, `str`, `bytes`, a list of floats or of
//! strings, and `None` for a node reference to nothing. A reference to a node is the package's `Node` of its class (`descriptors`).
//!
//! A script writes with the Python type it has: an `int` into a property the schema keeps as a double is a double, and a whole
//! `float` into an integer property is an integer. Anything else of the wrong kind is the core's `TypeError`.

use gnoblin_core::{NodeId, Value};
use gnoblin_core_abi::{VALUE_DOUBLE, VALUE_INT, VALUE_UINT};
use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyFloat, PyInt, PyList, PyString, PyTuple};

/// The attribute a `Node` of the package carries: its id, as an `int`. A value with it is a reference to that node.
const NODE_ID_ATTRIBUTE: &str = "_gnoblin_node_id";

/// The Python object for `value`.
pub(crate) fn to_python<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyAny>> {
    match value {
        Value::Bool(flag) => flag.into_bound_py_any(py),
        Value::Int(number) => number.into_bound_py_any(py),
        Value::Uint(number) => number.into_bound_py_any(py),
        Value::Double(number) => number.into_bound_py_any(py),
        Value::Text(text) => text.into_bound_py_any(py),
        Value::Node(id) => node(py, *id, None),
        Value::List(numbers) => numbers.into_bound_py_any(py),
        Value::Texts(texts) => texts.into_bound_py_any(py),
        Value::Bytes(bytes) => Ok(PyBytes::new(py, bytes).into_any()),
        _ => Ok(py.None().into_bound(py)),
    }
}

/// The Python object for a node reference: `None` for the nil node, else the package's `Node` of the node's class. `class` is the
/// class name when the caller knows it (a node that has left the tree cannot be asked).
pub(crate) fn node<'py>(py: Python<'py>, id: NodeId, class: Option<&str>) -> PyResult<Bound<'py, PyAny>> {
    if id == NodeId::NIL {
        return Ok(py.None().into_bound(py));
    }
    crate::descriptors::of(py, id.0, class)
}

/// The value a script wants written. `kind` is the `VALUE_*` kind the schema keeps the property as, when it is known.
pub(crate) fn from_python(value: &Bound<'_, PyAny>, kind: Option<u32>) -> PyResult<Value> {
    if value.is_instance_of::<PyBool>() {
        return Ok(Value::Bool(value.extract()?));
    }
    if let Ok(id) = value.getattr(NODE_ID_ATTRIBUTE) {
        return Ok(Value::Node(NodeId(id.extract()?)));
    }
    if value.is_instance_of::<PyInt>() {
        return Ok(match kind {
            Some(VALUE_DOUBLE) => Value::Double(value.extract::<f64>()?),
            Some(VALUE_UINT) => Value::Uint(value.extract()?),
            _ => match value.extract::<i64>() {
                Ok(number) => Value::Int(number),
                Err(_) => Value::Uint(value.extract()?),
            },
        });
    }
    if value.is_instance_of::<PyFloat>() {
        let number: f64 = value.extract()?;
        if matches!(kind, Some(VALUE_INT | VALUE_UINT)) && number.fract() == 0.0 {
            return Ok(if kind == Some(VALUE_UINT) && number >= 0.0 {
                Value::Uint(number as u64)
            } else {
                Value::Int(number as i64)
            });
        }
        return Ok(Value::Double(number));
    }
    if value.is_instance_of::<PyString>() {
        return Ok(Value::Text(value.extract()?));
    }
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(Value::Bytes(bytes.as_bytes().to_vec()));
    }
    if value.is_instance_of::<PyList>() || value.is_instance_of::<PyTuple>() {
        if let Ok(texts) = value.extract::<Vec<String>>()
            && !texts.is_empty()
        {
            return Ok(Value::Texts(texts));
        }
        return Ok(Value::List(value.extract::<Vec<f64>>().map_err(|_| {
            PyTypeError::new_err("a list written to the tree holds numbers only, or strings only")
        })?));
    }
    Err(PyTypeError::new_err(format!(
        "cannot write a {} to the tree",
        value.get_type().name()?
    )))
}
