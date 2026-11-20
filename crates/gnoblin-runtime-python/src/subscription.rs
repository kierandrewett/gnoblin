//! `gnoblin.events`: the handles a registration returns, and the values an event carries.
//!
//! A [`SubscriptionHandle`] is a registration the core holds for the running generation: an event, a chord, a timer. It lasts until a reload
//! replaces the generation, or until `unsubscribe`. Dropping the object does not remove it.
//!
//! `gnoblin.off(function)` removes the registrations made for a function, so each registration is also noted under its function, weakly:
//! a function that is collected takes its notes with it, and a callable that cannot be weakly referenced cannot be found by `off`.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyList;

use crate::errors::core_error;
use crate::values;

/// A registration the core holds for the running generation.
#[pyclass(name = "Subscription", module = "gnoblin.events")]
pub(crate) struct SubscriptionHandle {
    token: u64,
    active: bool,
}

impl SubscriptionHandle {
    /// The handle for the registration the core numbered `token`.
    pub(crate) const fn new(token: u64) -> Self {
        Self { token, active: true }
    }
}

#[pymethods]
impl SubscriptionHandle {
    #[new]
    fn py_new(token: u64) -> Self {
        Self::new(token)
    }

    /// Whether `unsubscribe` has not been called.
    #[getter]
    pub(crate) fn active(&self) -> bool {
        self.active
    }

    /// Removes the registration. Removing it twice, or after its generation ended, raises `StaleError`.
    pub(crate) fn unsubscribe(&mut self) -> PyResult<()> {
        self.active = false;
        crate::generation::take_subscription(self.token)?
            .unsubscribe()
            .map_err(|error| core_error(&error))
    }

    fn __repr__(&self) -> String {
        format!(
            "<SubscriptionHandle {}{}>",
            self.token,
            if self.active { "" } else { " removed" }
        )
    }
}

/// Function to the subscriptions made for it, weakly keyed.
static BY_FUNCTION: PyOnceLock<Py<PyAny>> = PyOnceLock::new();

fn by_function(py: Python<'_>) -> PyResult<&Bound<'_, PyAny>> {
    let map = BY_FUNCTION.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        Ok(py.import("weakref")?.getattr("WeakKeyDictionary")?.call0()?.unbind())
    })?;
    Ok(map.bind(py))
}

/// Notes that `subscription` was made for `function`, so `gnoblin.off(function)` finds it.
pub(crate) fn remember(
    py: Python<'_>,
    function: &Bound<'_, PyAny>,
    subscription: &Bound<'_, SubscriptionHandle>,
) -> PyResult<()> {
    let notes = match by_function(py)?.call_method1("setdefault", (function, PyList::empty(py))) {
        Ok(notes) => notes,
        // A callable that cannot be weakly referenced cannot be found by `off`; the `SubscriptionHandle` is still the way to remove it.
        Err(error) if error.is_instance_of::<PyTypeError>(py) => return Ok(()),
        Err(error) => return Err(error),
    };
    notes.call_method1("append", (subscription,))?;
    Ok(())
}

/// The subscriptions made for `function`, removed from the record.
pub(crate) fn forget<'py>(
    py: Python<'py>,
    function: &Bound<'py, PyAny>,
) -> PyResult<Vec<Bound<'py, SubscriptionHandle>>> {
    let notes = match by_function(py)?.call_method1("pop", (function, PyList::empty(py))) {
        Ok(notes) => notes,
        Err(error) if error.is_instance_of::<PyTypeError>(py) => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    notes
        .try_iter()?
        .map(|item| Ok(item?.cast_into::<SubscriptionHandle>()?))
        .collect()
}

/// Fills the module `gnoblin.events`.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<SubscriptionHandle>()?;
    module.add_class::<values::WindowRequest>()?;
    module.add_class::<values::NodeEvent>()?;
    module.add_class::<values::ChordEvent>()?;
    module.add_class::<values::PointerDrag>()?;
    module.add_class::<values::GestureEvent>()?;
    module.add_class::<values::PointerButton>()?;
    module.add_class::<values::ProcessExit>()?;
    module.add_class::<values::ProcessHandle>()?;
    Ok(())
}
