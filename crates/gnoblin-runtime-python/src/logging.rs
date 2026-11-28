//! What the runtime does once, when the interpreter starts: routes Python's own reporting into the core's log.
//!
//! An exception nobody caught, a warning, a `logging` record and a thread's traceback all end in the compositor's log with the
//! component `python`, so a shell is debugged where every other part of the session is. What a script prints to its standard streams is
//! left alone: those are the process's own.

use gnoblin_core::host::LogLevel;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PySlice, PyTuple};
use pyo3::wrap_pyfunction;

use crate::generation;
use crate::methods::NativeMethod;

/// The lines `traceback` prints for an exception, as one text.
fn formatted(py: Python<'_>, kind: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>, trace: &Bound<'_, PyAny>) -> String {
    py.import("traceback")
        .and_then(|module| module.call_method1("format_exception", (kind, value, trace)))
        .and_then(|lines| lines.extract::<Vec<String>>())
        .map_or_else(|_| value.to_string(), |lines| lines.concat())
}

/// `sys.excepthook`: an exception nobody caught.
#[pyfunction]
fn excepthook(py: Python<'_>, kind: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>, trace: &Bound<'_, PyAny>) {
    generation::log(
        LogLevel::CoreError,
        &format!("python: uncaught exception\n{}", formatted(py, kind, value, trace)),
    );
}

/// `threading.excepthook`: an exception that ended a thread of the script.
#[pyfunction]
fn thread_hook(py: Python<'_>, args: &Bound<'_, PyAny>) -> PyResult<()> {
    let name = args
        .getattr("thread")
        .and_then(|thread| thread.getattr("name"))
        .and_then(|name| name.extract::<String>())
        .unwrap_or_else(|_| "?".to_owned());
    let text = formatted(
        py,
        &args.getattr("exc_type")?,
        &args.getattr("exc_value")?,
        &args.getattr("exc_traceback")?,
    );
    generation::log(LogLevel::CoreError, &format!("python: thread {name} raised\n{text}"));
    Ok(())
}

/// `warnings.showwarning`: a warning.
#[pyfunction]
#[pyo3(signature = (message, category, filename, lineno, file = None, line = None))]
fn showwarning(
    py: Python<'_>,
    message: &Bound<'_, PyAny>,
    category: &Bound<'_, PyAny>,
    filename: &Bound<'_, PyAny>,
    lineno: &Bound<'_, PyAny>,
    file: Option<&Bound<'_, PyAny>>,
    line: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let _ = file;
    let text: String = py
        .import("warnings")?
        .call_method1("formatwarning", (message, category, filename, lineno, line))?
        .extract()?;
    generation::log(LogLevel::Warn, text.trim_end());
    Ok(())
}

/// The core's level for a `logging` level number: the decade it is in (`CRITICAL` is `error`), `info` for anything below `DEBUG`.
const fn level_of(number: i64) -> LogLevel {
    match number - number % 10 {
        40 | 50 => LogLevel::CoreError,
        30 => LogLevel::Warn,
        10 => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}

/// `CoreHandler.emit`: writes a record to the core's log.
#[pyfunction]
fn emit(handler: &Bound<'_, PyAny>, record: &Bound<'_, PyAny>) -> PyResult<()> {
    let outcome = (|| -> PyResult<()> {
        let level = level_of(record.getattr("levelno")?.extract()?);
        let name: String = record.getattr("name")?.extract()?;
        let text: String = handler.call_method1("format", (record,))?.extract()?;
        generation::log(level, &format!("{name}: {text}"));
        Ok(())
    })();
    // Logging must never raise into the shell: the handler reports its own failure the way every `logging` handler does.
    if outcome.is_err() {
        handler.call_method1("handleError", (record,))?;
    }
    Ok(())
}

/// Routes the interpreter's reporting to the core's log. Called once, by the runtime, before any shell runs.
pub(crate) fn install(py: Python<'_>) -> PyResult<()> {
    py.import("sys")?
        .setattr("excepthook", wrap_pyfunction!(excepthook, py)?)?;
    py.import("threading")?
        .setattr("excepthook", wrap_pyfunction!(thread_hook, py)?)?;
    py.import("warnings")?
        .setattr("showwarning", wrap_pyfunction!(showwarning, py)?)?;
    let logging = py.import("logging")?;
    let methods = PyDict::new(py);
    methods.set_item(
        "emit",
        Bound::new(py, NativeMethod::new(wrap_pyfunction!(emit, py)?.into_any()))?,
    )?;
    methods.set_item("__doc__", "A `logging` handler that writes to the core's log.")?;
    let handler = py
        .get_type::<pyo3::types::PyType>()
        .call1(("CoreHandler", PyTuple::new(py, [logging.getattr("Handler")?])?, methods))?
        .call0()?;
    handler.call_method1(
        "setFormatter",
        (logging.getattr("Formatter")?.call1(("%(message)s",))?,),
    )?;
    let root = logging.call_method0("getLogger")?;
    root.getattr("handlers")?.set_item(PySlice::full(py), [handler])?;
    root.call_method1("setLevel", (logging.getattr("INFO")?,))?;
    Ok(())
}
