//! How a script's callback runs.
//!
//! The core calls a callback on the shell thread. Here that means taking the interpreter's lock, calling the function, and
//! containing what it raises: the exception is logged with its traceback (file and line of the script), and the call returns to the
//! core as a failure (`E_PANIC`), which the core counts against the generation and then goes on to deliver the next event. The
//! SDK's trampoline turns an unwinding closure into that return code, so a failed callback unwinds with a payload that no panic
//! hook sees ([`std::panic::resume_unwind`]).

use std::panic::resume_unwind;

use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::generation;

/// The unwind payload of a callback that raised: the exception was reported already.
struct Raised;

/// Takes a callable the script passed. A value that cannot be called is a `TypeError` at the registration, not a failure at the
/// first event.
pub(crate) fn callable(value: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    if value.is_callable() {
        Ok(value.clone().unbind())
    } else {
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "expected a function to call, got {}",
            value.get_type().name()?
        )))
    }
}

/// Calls `callback` with the arguments `call` builds. An exception is logged as `what` raising it and the call fails as a callback
/// of the core does.
pub(crate) fn fire<'a>(
    what: &str,
    callback: &Py<PyAny>,
    call: impl for<'py> FnOnce(Python<'py>, &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> + 'a,
) {
    let failed = Python::attach(|py| match call(py, callback.bind(py)) {
        Ok(_) => false,
        Err(error) => {
            report(py, what, &error);
            true
        }
    });
    if failed {
        resume_unwind(Box::new(Raised));
    }
}

/// The traceback of `error` as Python prints it, ending with the exception's own line.
pub(crate) fn traceback_text(py: Python<'_>, error: &PyErr) -> String {
    let formatted = py.import("traceback").and_then(|module| {
        let value = error.value(py);
        // The three-argument form: the traceback is the error's own, whether or not Python has attached it to the exception yet.
        module
            .call_method1("format_exception", (value.get_type(), value, error.traceback(py)))
            .and_then(|lines| lines.extract::<Vec<String>>())
    });
    match formatted {
        Ok(lines) => lines.concat(),
        Err(_) => error.to_string(),
    }
}

/// Logs an exception a callback raised, with its traceback.
pub(crate) fn report(py: Python<'_>, what: &str, error: &PyErr) {
    // The file a traceback quotes may have been edited since it was read; make the lines it shows the ones on disk.
    if let Ok(linecache) = PyModule::import(py, "linecache") {
        let _ = linecache.call_method0("clearcache");
    }
    generation::log_error(&format!("python: {what} raised\n{}", traceback_text(py, error)));
}
