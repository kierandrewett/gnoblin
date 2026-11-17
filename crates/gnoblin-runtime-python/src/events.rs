//! Events, timers and shutdown hooks: the registrations a script makes for its generation.
//!
//! The layer is generic: a registration names a class (and a property or signal) as the catalog does, and the callback is the
//! script's function, called on the shell thread with the node as the package's typed object. The named subscriptions of the
//! package (`on_client_created`, `on_title_changed`, ...) are aliases the package writes over these. Each function returns the
//! number that removes the registration (`unsubscribe`). A registration belongs to the generation that was current when the script
//! made it and is removed with it, so a reload leaves exactly the registrations the new script made.

use gnoblin_core::{Gnoblin, NodeId, Subscription, Value};
use pyo3::prelude::*;

use crate::callback::{callable, fire};
use crate::convert::{node, to_python};
use crate::errors::core_error;
use crate::generation::{current_ctx, handle, keep_subscription, take_subscription};

/// Makes a registration with the current generation's handle and keeps it under a number.
fn register(make: impl FnOnce(&Gnoblin) -> gnoblin_core::Result<Subscription>) -> PyResult<u64> {
    let g = handle()?;
    let subscription = make(&g).map_err(|e| core_error(&e))?;
    Ok(keep_subscription(current_ctx(), subscription))
}

/// Calls `callback(node)` for each node of `class` that comes into the tree.
#[pyfunction]
pub(crate) fn on_node_created(class: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    let name = class.to_owned();
    register(|g| {
        g.on_node_created(class, move |created| {
            fire("on_node_created", &callback, |py, f| {
                f.call1((node(py, created.id(), Some(&name))?,))
            });
        })
    })
}

/// Calls `callback(node)` for each node of `class` that leaves the tree. The node is gone from reads; its id and class are left.
#[pyfunction]
pub(crate) fn on_node_destroyed(class: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    let name = class.to_owned();
    register(|g| {
        g.on_node_destroyed(class, move |gone| {
            fire("on_node_destroyed", &callback, |py, f| {
                f.call1((node(py, gone.id(), Some(&name))?,))
            });
        })
    })
}

/// Calls `callback(node, value)` when the property `name` of any node of `class` changes, whoever changed it.
#[pyfunction]
pub(crate) fn on_class_property_changed(class: &str, name: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    let class_name = class.to_owned();
    register(|g| {
        g.on_class_property_changed(class, name, move |changed, value| {
            fire("on_class_property_changed", &callback, |py, f| {
                f.call1((node(py, changed.id(), Some(&class_name))?, to_python(py, value)?))
            });
        })
    })
}

/// Calls `callback(node, value)` when the property `name` of the node `id` changes.
#[pyfunction]
pub(crate) fn on_property_changed(id: u64, name: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.on_property_changed(&g.node(NodeId(id)), name, move |changed, value| {
            fire("on_property_changed", &callback, |py, f| {
                f.call1((node(py, changed.id(), None)?, to_python(py, value)?))
            });
        })
    })
}

/// Calls `callback(window, request, x, y)` when a window's client makes a request that policy decides (`maximize`, `close`, ...).
#[pyfunction]
pub(crate) fn on_window_request(callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.on_window_request(move |window, request| {
            fire("on_window_request", &callback, |py, f| {
                f.call1((
                    node(py, window.id(), Some("window"))?,
                    request.name.as_str(),
                    request.x,
                    request.y,
                ))
            });
        })
    })
}

/// Calls `callback(kind, x, y, button, key, text, dx, dy, pressed)` for each input event sent to the node `id`.
#[pyfunction]
pub(crate) fn on_node_event(id: u64, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.on_node_event(&g.node(NodeId(id)), move |event| {
            fire("on_node_event", &callback, |_, f| {
                f.call1((
                    event.kind.as_str(),
                    event.x,
                    event.y,
                    event.button,
                    event.key.as_str(),
                    event.text.as_str(),
                    event.dx,
                    event.dy,
                    event.pressed,
                ))
            });
        })
    })
}

/// Calls `callback(node, values)` for each `signal` of `class`: the signal's fields in schema order.
#[pyfunction]
pub(crate) fn on_signal(class: &str, signal: &str, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    let name = class.to_owned();
    register(|g| {
        g.on_signal_named(class, signal, move |event| {
            fire("on_signal", &callback, |py, f| {
                let values = event
                    .values()
                    .iter()
                    .map(|value| match Value::from_shell(value) {
                        Some(value) => to_python(py, &value),
                        None => Ok(py.None().into_bound(py)),
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                f.call1((node(py, NodeId(event.node().0), Some(&name))?, values))
            });
        })
    })
}

/// Calls `callback()` once when the generation ends, before its registrations are removed.
#[pyfunction]
pub(crate) fn on_shutdown(callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.on_shutdown(move || {
            fire("on_shutdown", &callback, |_, f| f.call0());
        })
    })
}

/// Calls `callback()` once, `ms` milliseconds from now.
#[pyfunction]
pub(crate) fn after(ms: u32, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.after(ms, move || {
            fire("after", &callback, |_, f| f.call0());
        })
    })
}

/// Calls `callback()` every `ms` milliseconds.
#[pyfunction]
pub(crate) fn every(ms: u32, callback: &Bound<'_, PyAny>) -> PyResult<u64> {
    let callback = callable(callback)?;
    register(|g| {
        g.every(ms, move || {
            fire("every", &callback, |_, f| f.call0());
        })
    })
}

/// Removes the registration behind `token`.
#[pyfunction]
pub(crate) fn unsubscribe(token: u64) -> PyResult<()> {
    take_subscription(token)?.unsubscribe().map_err(|e| core_error(&e))
}
