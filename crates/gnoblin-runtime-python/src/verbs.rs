//! The compositor's own verbs and signals (`gnoblin.session`, `gnoblin.idle`, the outputs): the two generic functions the
//! package's generated `verbs` module is written over.
//!
//! Like the rest of the module this layer names a class, a method and a signal as the catalog does, and moves values by position.
//! The typed spelling (`verbs.session.logout()`, `verbs.session.on_bell(fn)`) is generated from the schema into
//! the generated `gnoblin/_generated/verbs.py`, so a method added to the schema adds no line here.

use gnoblin_core::host::Value;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyTuple};

use crate::callback::{callable, fire};
use crate::convert::{from_python, to_python};
use crate::errors::core_error;
use crate::events::register;
use crate::generation::handle;

/// Calls `method` of `class` (`"gnoblin.session"`) on the compositor. `object` is the compositor's identity of the object, or `0`
/// for the class's one object. With `on_answer`, it is called once with `(result, error)` when the compositor answers: the
/// operation's value as JSON bytes (empty for a bare done), and the compositor's error code or `None`.
#[pyfunction]
#[pyo3(signature = (class, method, args = None, on_answer = None, object = 0))]
pub(crate) fn verb_call(
    class: &str,
    method: &str,
    args: Option<&Bound<'_, PyTuple>>,
    on_answer: Option<&Bound<'_, PyAny>>,
    object: u64,
) -> PyResult<()> {
    let g = handle()?;
    let values = match args {
        Some(args) => args
            .iter()
            .map(|item| from_python(&item, None))
            .collect::<PyResult<Vec<Value>>>()?,
        None => Vec::new(),
    };
    match on_answer {
        None if object == 0 => g.call_verb(class, method, &values).map_err(|e| core_error(&e)),
        None => g
            .call_verb_on(object, class, method, &values, |_| {})
            .map(drop)
            .map_err(|e| core_error(&e)),
        Some(callback) => {
            let callback = callable(callback)?;
            g.call_verb_on(object, class, method, &values, move |answer| {
                fire("verb_call", &callback, |py, f| {
                    f.call1((PyBytes::new(py, &answer.result), answer.error.clone()))
                });
            })
            .map(drop)
            .map_err(|e| core_error(&e))
        }
    }
}

/// Calls `callback(*values)` for every `signal` of `class` the compositor raises, with the payload in the schema's field order.
/// Returns the number that removes the registration (`unsubscribe`).
#[pyfunction]
pub(crate) fn verb_subscribe(class: &str, signal: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.subscribe(class, signal, move |event| {
            fire("verb_subscribe", &callback, |py, f| {
                let values = event
                    .values()
                    .iter()
                    .map(|value| match Value::from_shell(value) {
                        Some(value) => to_python(py, &value),
                        None => Ok(py.None().into_bound(py)),
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                f.call1(PyTuple::new(py, values)?)
            });
        })
    })
}
