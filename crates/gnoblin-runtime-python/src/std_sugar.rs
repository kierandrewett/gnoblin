//! `gnoblin._std_sugar`: the spellings over the generated `gnoblin.std`: `len(collection)`, `client.window`, `clients()`.
//!
//! `std.py` is the table, typed. What a Python reader expects of a collection or a client that the table has no name for is here, and
//! nothing else: no behaviour of the desktop lives in this file. The generated module calls [`install`] with its own namespace once it is
//! defined.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyFloat, PyList, PyTuple};
use pyo3::wrap_pyfunction;

use crate::descriptors;
use crate::methods::NativeMethod;
use crate::std_base;

/// The error a floor of float arithmetic is allowed: the right edge of 619.2 + 404.8 is 1023.9999999999999.
const EDGE_EPSILON: f64 = 1e-6;

/// A class of the generated module, by name.
fn class<'py>(namespace: &Bound<'py, PyDict>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    namespace
        .get_item(name)?
        .ok_or_else(|| PyTypeError::new_err(format!("the generated gnoblin.std has no `{name}`")))
}

/// Adds `function` to `class` as a method that binds its instance.
fn add_method(
    py: Python<'_>,
    class: &Bound<'_, PyAny>,
    name: &str,
    function: Bound<'_, pyo3::types::PyCFunction>,
) -> PyResult<()> {
    class.setattr(name, Bound::new(py, NativeMethod::new(function.into_any()))?)
}

/// The members as the class of their kind: clients, outputs or workspaces.
#[pyfunction]
fn members<'py>(py: Python<'py>, collection: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let namespace = std_namespace(py)?;
    let kind = collection.call_method0("kind")?;
    let kinds = class(&namespace, "CollectionKind")?;
    let wrapper = if kind.eq(kinds.getattr("CLIENTS")?)? {
        class(&namespace, "Client")?
    } else if kind.eq(kinds.getattr("OUTPUTS")?)? {
        class(&namespace, "Output")?
    } else {
        class(&namespace, "Workspace")?
    };
    let wrapped = PyList::empty(py);
    for member in collection.call_method0("members")?.try_iter()? {
        wrapped.append(wrapper.call1((member?,))?)?;
    }
    wrapped.call_method0("__iter__")
}

/// The namespace of the generated module, which [`install`] was called with.
static NAMESPACE: PyOnceLock<Py<PyDict>> = PyOnceLock::new();

fn std_namespace(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    NAMESPACE
        .get(py)
        .map(|namespace| namespace.bind(py).clone())
        .ok_or_else(|| PyTypeError::new_err("gnoblin.std has not installed its spellings yet"))
}

#[pyfunction]
fn collection_len(collection: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    Ok(collection.call_method0("len")?.unbind())
}

#[pyfunction]
fn collection_contains(collection: &Bound<'_, PyAny>, member: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    Ok(collection.call_method1("contains", (member,))?.unbind())
}

/// The member at `index`, or `None`.
#[pyfunction]
fn collection_at(py: Python<'_>, collection: &Bound<'_, PyAny>, index: isize) -> PyResult<Py<PyAny>> {
    let found = collection.call_method0("members")?;
    let count = found.len()? as isize;
    if -count <= index && index < count {
        Ok(found.get_item(index)?.unbind())
    } else {
        Ok(py.None())
    }
}

/// Calls `callback` for `event` of the collection; `replay` tells it of every member that is there already.
#[pyfunction]
#[pyo3(signature = (collection, event, callback, replay = false))]
fn collection_on(
    collection: &Bound<'_, PyAny>,
    event: &Bound<'_, PyAny>,
    callback: &Bound<'_, PyAny>,
    replay: bool,
) -> PyResult<Py<PyAny>> {
    Ok(collection
        .call_method1("listen", (event, i32::from(replay), callback))?
        .unbind())
}

/// The window of the tree this client is.
#[pyfunction]
fn client_window<'py>(py: Python<'py>, client: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    descriptors::of(py, client.getattr("node")?.extract()?, None)
}

/// A tiled placement of `node` at whole pixels.
///
/// Each of the four edges is floored on its own, and the size is the distance between two floored edges, so a master of 607.2 and a stack
/// of 404.8 that starts at 619.2 are 607 and 404 wide, and the stack still ends at 1023 in an area that ends at 1024. This is the rule of
/// the Rust SDK's `Placement::tile` and of the TypeScript library's tiling layouts.
#[pyfunction]
fn placement_tile<'py>(
    py: Python<'py>,
    node: &Bound<'py, PyAny>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> PyResult<Bound<'py, PyAny>> {
    let namespace = std_namespace(py)?;
    let left = (x + EDGE_EPSILON).floor();
    let top = (y + EDGE_EPSILON).floor();
    let bounds = class(&namespace, "Rect")?.call1((
        left,
        top,
        (x + width + EDGE_EPSILON).floor() - left,
        (y + height + EDGE_EPSILON).floor() - top,
    ))?;
    let flags = class(&namespace, "PlacementFlags")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item(
        "flags",
        flags
            .getattr("BOUNDS")?
            .call_method1("__or__", (flags.getattr("TILED")?,))?,
    )?;
    kwargs.set_item("node", node)?;
    kwargs.set_item("bounds", bounds)?;
    class(&namespace, "Placement")?.call((), Some(&kwargs))
}

/// The workspace's area less the outer gap on every side: where a tiling layout works.
#[pyfunction]
fn layout_inset_area<'py>(py: Python<'py>, context: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    let namespace = std_namespace(py)?;
    let gap: f64 = context.getattr("gap_outer")?.extract()?;
    let area = context.getattr("area")?;
    let (x, y, width, height): (f64, f64, f64, f64) = (
        area.getattr("x")?.extract()?,
        area.getattr("y")?.extract()?,
        area.getattr("width")?.extract()?,
        area.getattr("height")?.extract()?,
    );
    class(&namespace, "Rect")?.call1((x + gap, y + gap, width - 2.0 * gap, height - 2.0 * gap))
}

/// Makes a workspace, gives it the layout (an object with `compute`, or `None` for floating) and adds it to the list.
#[pyfunction]
#[pyo3(signature = (cls, id, *, name = "", layout = None, gaps = None))]
fn workspace_create<'py>(
    py: Python<'py>,
    cls: &Bound<'py, PyAny>,
    id: &str,
    name: &str,
    layout: Option<&Bound<'py, PyAny>>,
    gaps: Option<&Bound<'py, PyTuple>>,
) -> PyResult<Bound<'py, PyAny>> {
    let namespace = std_namespace(py)?;
    let (inner, outer): (Bound<'py, PyAny>, Bound<'py, PyAny>) = match gaps {
        Some(gaps) => (gaps.get_item(0)?, gaps.get_item(1)?),
        None => (PyFloat::new(py, 0.0).into_any(), PyFloat::new(py, 0.0).into_any()),
    };
    let spec = PyDict::new(py);
    spec.set_item("id", id)?;
    spec.set_item("name", name)?;
    spec.set_item("gap_inner", inner)?;
    spec.set_item("gap_outer", outer)?;
    match layout.filter(|layout| !layout.is_none()) {
        None => spec.set_item("layout", py.None())?,
        Some(layout) => {
            let layout_spec = PyDict::new(py);
            layout_spec.set_item("kind", class(&namespace, "LayoutKind")?.getattr("FOREIGN")?)?;
            layout_spec.set_item("vtable", layout)?;
            spec.set_item("layout", class(&namespace, "LayoutSpec")?.call((), Some(&layout_spec))?)?;
        }
    }
    let workspace = cls.call_method1("new", (class(&namespace, "WorkspaceSpec")?.call((), Some(&spec))?,))?;
    workspace.call_method0("add")?;
    Ok(workspace)
}

/// Gives the workspace a built-in layout (`MasterStackLayout(ratio=0.6)`) and returns it, bound to this workspace.
#[pyfunction]
fn workspace_use<'py>(workspace: &Bound<'py, PyAny>, layout: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    layout.call_method1("attach", (workspace,))
}

/// Every client of the compositor, in the order they appeared.
#[pyfunction]
fn clients(py: Python<'_>) -> PyResult<Py<PyAny>> {
    root_of(py, "CLIENTS")
}

/// Every connected output.
#[pyfunction]
fn outputs(py: Python<'_>) -> PyResult<Py<PyAny>> {
    root_of(py, "OUTPUTS")
}

/// The workspaces the shell added, in order.
#[pyfunction]
fn workspaces(py: Python<'_>) -> PyResult<Py<PyAny>> {
    root_of(py, "WORKSPACES")
}

fn root_of(py: Python<'_>, kind: &str) -> PyResult<Py<PyAny>> {
    let namespace = std_namespace(py)?;
    let kinds = class(&namespace, "CollectionKind")?;
    Ok(class(&namespace, "Collection")?
        .call_method1("root", (kinds.getattr(kind)?,))?
        .unbind())
}

/// Adds the spellings to the classes of `gnoblin.std`, and the functions that name its roots.
#[pyfunction]
fn install(py: Python<'_>, namespace: &Bound<'_, PyDict>) -> PyResult<()> {
    NAMESPACE.get_or_init(py, || namespace.clone().unbind());
    let collection = class(namespace, "Collection")?;
    let builtins = py.import("builtins")?;
    add_method(py, &collection, "__len__", wrap_pyfunction!(collection_len, py)?)?;
    add_method(py, &collection, "__iter__", wrap_pyfunction!(members, py)?)?;
    add_method(
        py,
        &collection,
        "__contains__",
        wrap_pyfunction!(collection_contains, py)?,
    )?;
    add_method(py, &collection, "items", wrap_pyfunction!(members, py)?)?;
    add_method(py, &collection, "at", wrap_pyfunction!(collection_at, py)?)?;
    add_method(py, &collection, "on", wrap_pyfunction!(collection_on, py)?)?;

    class(namespace, "Client")?.setattr(
        "window",
        builtins
            .getattr("property")?
            .call1((wrap_pyfunction!(client_window, py)?,))?,
    )?;

    class(namespace, "Placement")?.setattr(
        "tile",
        builtins
            .getattr("staticmethod")?
            .call1((wrap_pyfunction!(placement_tile, py)?,))?,
    )?;
    add_method(
        py,
        &class(namespace, "LayoutContext")?,
        "inset_area",
        wrap_pyfunction!(layout_inset_area, py)?,
    )?;

    let workspace = class(namespace, "Workspace")?;
    workspace.setattr(
        "create",
        builtins
            .getattr("classmethod")?
            .call1((wrap_pyfunction!(workspace_create, py)?,))?,
    )?;
    add_method(py, &workspace, "use", wrap_pyfunction!(workspace_use, py)?)?;

    std_base::set_registration_class(class(namespace, "Registration")?);

    namespace.set_item("clients", wrap_pyfunction!(clients, py)?)?;
    namespace.set_item("outputs", wrap_pyfunction!(outputs, py)?)?;
    namespace.set_item("workspaces", wrap_pyfunction!(workspaces, py)?)?;
    Ok(())
}

/// Fills the module `gnoblin._std_sugar`.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(install, module)?)?;
    Ok(())
}
