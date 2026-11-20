//! A native function that binds like a method.
//!
//! A built-in function in a class body is not a descriptor, so `instance.name` would give it back without the instance. [`NativeMethod`]
//! wraps one so `instance.name(...)` passes the instance first, as a function written in Python would, and `Class.name(instance, ...)`
//! still works. The runtime uses it where it adds methods to a class it does not own the source of: the handler it hands the `logging`
//! module, and the spellings of `gnoblin.std` (`len(collection)`).

use pyo3::prelude::*;

/// A function that becomes a bound method when it is read from an instance.
#[pyclass(frozen, name = "native_method", module = "gnoblin")]
pub(crate) struct NativeMethod {
    function: Py<PyAny>,
}

impl NativeMethod {
    /// Wraps `function`, which takes the instance as its first argument.
    pub(crate) fn new(function: Bound<'_, PyAny>) -> Self {
        Self {
            function: function.unbind(),
        }
    }
}

#[pymethods]
impl NativeMethod {
    fn __get__<'py>(
        &self,
        py: Python<'py>,
        instance: Option<&Bound<'py, PyAny>>,
        owner: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = owner;
        match instance {
            None => Ok(self.function.bind(py).clone()),
            Some(instance) => py
                .import("types")?
                .getattr("MethodType")?
                .call1((self.function.bind(py), instance)),
        }
    }
}
