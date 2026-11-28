//! The tree: nodes by id, their properties, methods and place.
//!
//! A node is a number to a script (the package wraps it in `Node` and `Window`); each function here is one call of the core SDK on
//! the handle of the generation `_gnoblin_core` acts on. Reading and writing are the SDK's: a write is queued and leaves with the
//! turn's other writes, and a read sees the tree as the last batch left it.

use gnoblin_core::host::{Host, MemberKind, Node, NodeId};
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::callback::{callable, fire};
use crate::convert::{from_python, to_python};
use crate::errors::core_error;
use crate::generation::handle;

/// The node `id` on the current generation's handle.
fn node_of(id: u64) -> PyResult<Node> {
    Ok(handle()?.node(NodeId(id)))
}

/// The id of the tree's root.
#[pyfunction]
pub(crate) fn root() -> PyResult<u64> {
    Ok(handle()?.root().map_err(|e| core_error(&e))?.id().0)
}

/// The ids of the windows in the tree, in stacking order (the first is the bottom).
#[pyfunction]
pub(crate) fn windows() -> PyResult<Vec<u64>> {
    let windows = handle()?.windows().map_err(|e| core_error(&e))?;
    Ok(windows.iter().map(|window| window.id().0).collect())
}

/// Whether the node is in the tree now.
#[pyfunction]
pub(crate) fn node_exists(id: u64) -> PyResult<bool> {
    Ok(node_of(id)?.is_alive())
}

/// The name of the node's class (`window`).
#[pyfunction]
pub(crate) fn node_class(id: u64) -> PyResult<String> {
    node_of(id)?.class_name().map_err(|e| core_error(&e))
}

/// The id of the node's parent, `None` for the root.
#[pyfunction]
pub(crate) fn node_parent(id: u64) -> PyResult<Option<u64>> {
    Ok(node_of(id)?
        .parent()
        .map_err(|e| core_error(&e))?
        .map(|parent| parent.id().0))
}

/// The ids of the node's children, in order.
#[pyfunction]
pub(crate) fn node_children(id: u64) -> PyResult<Vec<u64>> {
    let children = node_of(id)?.children().map_err(|e| core_error(&e))?;
    Ok(children.iter().map(|child| child.id().0).collect())
}

/// Reads a property by its schema name.
#[pyfunction]
pub(crate) fn node_get<'py>(py: Python<'py>, id: u64, name: &str) -> PyResult<Bound<'py, PyAny>> {
    let value = node_of(id)?.get(name).map_err(|e| core_error(&e))?;
    to_python(py, &value)
}

/// Writes a property; `seconds` above zero asks the compositor to animate the change.
#[pyfunction]
#[pyo3(signature = (id, name, value, seconds = 0.0))]
pub(crate) fn node_set(id: u64, name: &str, value: &Bound<'_, PyAny>, seconds: f64) -> PyResult<()> {
    let g = handle()?;
    let node = g.node(NodeId(id));
    let class = node.class().map_err(|e| core_error(&e))?;
    let kind = g
        .member(class, MemberKind::Property, name)
        .map_err(|e| core_error(&e))?
        .value_kind;
    let value = from_python(value, Some(kind))?;
    node.set_animated(name, value, seconds).map_err(|e| core_error(&e))
}

/// Calls a method of the node. With `on_answer`, it is called once with `(result, error)` when the compositor answers: the
/// answer's bytes, and the failure's text or `None`.
#[pyfunction]
#[pyo3(signature = (id, method, args = None, on_answer = None))]
pub(crate) fn node_call(
    id: u64,
    method: &str,
    args: Option<&Bound<'_, PyTuple>>,
    on_answer: Option<&Bound<'_, PyAny>>,
) -> PyResult<()> {
    let node = node_of(id)?;
    let values = match args {
        Some(args) => args
            .iter()
            .map(|item| from_python(&item, None))
            .collect::<PyResult<Vec<_>>>()?,
        None => Vec::new(),
    };
    match on_answer {
        None => node.call(method, &values).map_err(|e| core_error(&e)),
        Some(callback) => {
            let callback = callable(callback)?;
            node.call_with(method, &values, move |answer| answer_to(&callback, "call", answer))
                .map(drop)
                .map_err(|e| core_error(&e))
        }
    }
}

/// Runs `callback(result_bytes, error_or_None)` for an answer.
fn answer_to(callback: &Py<PyAny>, what: &str, answer: gnoblin_core::host::Answer) {
    fire(what, callback, |py, function| {
        function.call1((pyo3::types::PyBytes::new(py, &answer.result), answer.error.clone()))
    });
}

/// Puts the node under `parent` at `index` (`None` is on top).
#[pyfunction]
#[pyo3(signature = (id, parent, index = None))]
pub(crate) fn node_insert(id: u64, parent: u64, index: Option<usize>) -> PyResult<()> {
    let g = handle()?;
    g.node(NodeId(id))
        .insert_into(&g.node(NodeId(parent)), index)
        .map_err(|e| core_error(&e))
}

/// Destroys the node and everything beneath it.
#[pyfunction]
pub(crate) fn node_destroy(id: u64) -> PyResult<()> {
    node_of(id)?.destroy().map_err(|e| core_error(&e))
}

/// Makes a node of `class` that no one has put in the tree yet; returns its id.
#[pyfunction]
pub(crate) fn create_node(class: &str) -> PyResult<u64> {
    Ok(handle()?.create_node(class).map_err(|e| core_error(&e))?.id().0)
}

/// Finds or makes the node of `class` called `name` that an earlier generation made: returns its id, and whether it was found.
#[pyfunction]
#[pyo3(signature = (class, name, ephemeral = false))]
pub(crate) fn adopt_node(class: &str, name: &str, ephemeral: bool) -> PyResult<(u64, bool)> {
    let g: Host = handle()?;
    let adoption = if ephemeral {
        g.adopt_ephemeral_node(class, name)
    } else {
        g.adopt_node(class, name)
    }
    .map_err(|e| core_error(&e))?;
    Ok((adoption.node.id().0, adoption.adopted))
}

/// Calls `on_done(frame, error)` when the compositor has applied everything written so far: the frame number, or the failure's text.
#[pyfunction]
pub(crate) fn flush(on_done: &Bound<'_, PyAny>) -> PyResult<()> {
    let callback = callable(on_done)?;
    handle()?
        .flush(move |outcome| {
            let (frame, error) = match outcome {
                Ok(frame) => (Some(frame), None),
                Err(error) => (None, Some(error.to_string())),
            };
            fire("flush", &callback, |_, function| function.call1((frame, error.clone())));
        })
        .map(drop)
        .map_err(|e| core_error(&e))
}
