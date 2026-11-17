//! The built-in module `_gnoblin_core`: the whole of what Python can reach of the core.
//!
//! It is generic on purpose. A class, property, method or signal is named as the schema catalog names it, and the typed classes
//! a script uses (`gnoblin.Window` and the rest) are generated from that catalog into the pure-Python package, so adding a property
//! to the schema changes no line here. What is left is the verbs: read, write and call a node, call the compositor's own verbs and hear its signals, listen, claim input, start a program,
//! keep state across a reload, log and post work to the shell thread.
//!
//! The module is registered with the interpreter before it starts ([`crate::interp`]), so `import _gnoblin_core` finds it in any
//! script and in the package, and nowhere else: outside a shell there is nothing for it to talk to.

use gnoblin_core::LogLevel;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use crate::callback::{callable, fire};
use crate::errors::{add_to, core_error, to_py};
use crate::generation::{self, handle, poster, with_state};
use crate::{events, input, process, tree, verbs};

/// Writes a line to the core's log. `level` is `error`, `warn`, `info`, `debug` or `trace`. Callable from any thread.
#[pyfunction]
fn log(level: &str, message: &str) -> PyResult<()> {
    let level = match level {
        "error" => LogLevel::CoreError,
        "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" => LogLevel::Debug,
        "trace" => LogLevel::Trace,
        other => {
            return Err(PyValueError::new_err(format!(
                "a log level is error, warn, info, debug or trace, not `{other}`"
            )));
        }
    };
    generation::log(level, message);
    Ok(())
}

/// What the core says about the generation: its number, language, how many times the shell was reloaded, the module path.
#[pyfunction]
fn info(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let info = handle()?.info().map_err(|e| core_error(&e))?;
    let dict = PyDict::new(py);
    dict.set_item("generation", info.generation)?;
    dict.set_item("reload_count", info.reload_count)?;
    dict.set_item("isolate", info.isolate)?;
    dict.set_item("module_path", info.module_path)?;
    dict.set_item("state_dir", info.state_dir)?;
    Ok(dict)
}

/// The core's counters: live registrations, callbacks run, panics caught, generations retired, stale calls refused.
#[pyfunction]
fn stats(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    let stats = handle()?.stats().map_err(|e| core_error(&e))?;
    let dict = PyDict::new(py);
    dict.set_item("registrations", stats.registrations)?;
    dict.set_item("callbacks", stats.callbacks)?;
    dict.set_item("panics", stats.panics)?;
    dict.set_item("retired_generations", stats.retired_generations)?;
    dict.set_item("stale_calls", stats.stale_calls)?;
    Ok(dict)
}

/// Asks the host to reload this shell after the current callback returns.
#[pyfunction]
fn request_reload() -> PyResult<()> {
    handle()?.request_reload().map_err(|e| core_error(&e))
}

/// Asks the host to exit the compositor with `code`.
#[pyfunction]
fn request_exit(code: i32) -> PyResult<()> {
    handle()?.request_exit(code).map_err(|e| core_error(&e))
}

/// Runs `callback()` on the shell thread at its next turn. Callable from any thread; dropped unrun if the generation retires first.
#[pyfunction]
fn post(callback: &Bound<'_, PyAny>) -> PyResult<()> {
    let callback = callable(callback)?;
    let poster = poster().ok_or_else(|| to_py(None, "no Gnoblin shell is running"))?;
    poster
        .post(move |_| fire("post", &callback, |_, f| f.call0()))
        .map_err(|e| core_error(&e))
}

/// Keeps `data` under `key` for the next generation of this shell.
#[pyfunction]
#[pyo3(signature = (key, data, version = 0))]
fn keep_state(key: &str, data: &[u8], version: u32) -> PyResult<()> {
    handle()?.keep_state(key, data, version).map_err(|e| core_error(&e))
}

/// What an earlier generation kept under `key`, as `(bytes, version)`; `None` when nothing is kept.
#[pyfunction]
fn kept_state<'py>(py: Python<'py>, key: &str) -> PyResult<Option<(Bound<'py, PyBytes>, u32)>> {
    let kept = handle()?.kept_state(key).map_err(|e| core_error(&e))?;
    Ok(kept.map(|(bytes, version)| (PyBytes::new(py, &bytes), version)))
}

/// Registers the function `Node(id, class_name_or_None)` builds a node object with, for values that are nodes.
#[pyfunction]
fn set_node_factory(factory: &Bound<'_, PyAny>) -> PyResult<()> {
    let factory = callable(factory)?;
    with_state(|state| state.node_factory = Some(factory)).ok_or_else(|| to_py(None, generation::WRONG_THREAD))
}

/// The module.
#[pymodule]
pub(crate) fn _gnoblin_core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    add_to(module)?;
    module.add_function(wrap_pyfunction!(log, module)?)?;
    module.add_function(wrap_pyfunction!(info, module)?)?;
    module.add_function(wrap_pyfunction!(stats, module)?)?;
    module.add_function(wrap_pyfunction!(request_reload, module)?)?;
    module.add_function(wrap_pyfunction!(request_exit, module)?)?;
    module.add_function(wrap_pyfunction!(post, module)?)?;
    module.add_function(wrap_pyfunction!(keep_state, module)?)?;
    module.add_function(wrap_pyfunction!(kept_state, module)?)?;
    module.add_function(wrap_pyfunction!(set_node_factory, module)?)?;
    // The tree.
    module.add_function(wrap_pyfunction!(tree::root, module)?)?;
    module.add_function(wrap_pyfunction!(tree::windows, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_exists, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_class, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_parent, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_children, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_get, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_set, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_call, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_insert, module)?)?;
    module.add_function(wrap_pyfunction!(tree::node_destroy, module)?)?;
    module.add_function(wrap_pyfunction!(tree::create_node, module)?)?;
    module.add_function(wrap_pyfunction!(tree::adopt_node, module)?)?;
    module.add_function(wrap_pyfunction!(tree::flush, module)?)?;
    // Events, timers and hooks.
    module.add_function(wrap_pyfunction!(events::on_node_created, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_node_destroyed, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_class_property_changed, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_property_changed, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_window_request, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_node_event, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_signal, module)?)?;
    module.add_function(wrap_pyfunction!(events::on_shutdown, module)?)?;
    module.add_function(wrap_pyfunction!(events::after, module)?)?;
    module.add_function(wrap_pyfunction!(events::every, module)?)?;
    module.add_function(wrap_pyfunction!(events::unsubscribe, module)?)?;
    // The compositor's `gnoblin` namespace: verbs and signals.
    module.add_function(wrap_pyfunction!(verbs::verb_call, module)?)?;
    module.add_function(wrap_pyfunction!(verbs::verb_subscribe, module)?)?;
    // Input.
    module.add_function(wrap_pyfunction!(input::bind, module)?)?;
    module.add_function(wrap_pyfunction!(input::bind_events, module)?)?;
    module.add_function(wrap_pyfunction!(input::bind_pointer, module)?)?;
    module.add_function(wrap_pyfunction!(input::bind_gesture, module)?)?;
    module.add_function(wrap_pyfunction!(input::claim_pointer_buttons, module)?)?;
    module.add_function(wrap_pyfunction!(input::claim_device, module)?)?;
    module.add_function(wrap_pyfunction!(input::grab_input, module)?)?;
    // Programs.
    module.add_function(wrap_pyfunction!(process::spawn, module)?)?;
    module.add_function(wrap_pyfunction!(process::process_signal, module)?)?;
    Ok(())
}
