//! `gnoblin._base`: what a generated class of the tree is made of.
//!
//! The classes themselves (`Window`, `Seat`, ... in `gnoblin._generated.classes`) are generated from the schema catalog; nothing about
//! one of them is written by hand. What is here is what they are built from:
//!
//! - [`ScriptNode`], a handle on one node of the compositor's tree, by id. It holds the id and nothing else: the compositor owns the node,
//!   so reading a window that has closed raises `NodeGoneError`, and a handle kept across a reload keeps working because the node does.
//! - [`PropertyDescriptor`], [`MethodDescriptor`] and [`SignalDescriptor`], descriptors that read, call and listen **by name** through the core, so a property added to
//!   the schema works the day it is generated.
//!
//! A subclass of `Node` that names `__class_name__` in its own body is registered under that name in `CLASSES`, which is how a node
//! the core reports by class becomes the typed object for it.

use gnoblin_core::host::NodeId;
use pyo3::IntoPyObjectExt;
use pyo3::basic::CompareOp;
use pyo3::exceptions::{PyAttributeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyCFunction, PyDict, PyTuple, PyType};

use crate::errors::NodeGoneError;
use crate::subscription::{SubscriptionHandle, remember};
use crate::{events, tree};

/// Every generated class by its schema name (`"window"`), filled as the classes are defined.
static CLASSES: PyOnceLock<Py<PyDict>> = PyOnceLock::new();

/// The registry of the classes: `gnoblin.host.CLASSES`.
pub(crate) fn classes(py: Python<'_>) -> &Bound<'_, PyDict> {
    CLASSES.get_or_init(py, || PyDict::new(py).unbind()).bind(py)
}

/// The text Python gives `repr(text)`.
fn repr_of(py: Python<'_>, text: &str) -> PyResult<String> {
    Ok(text.into_bound_py_any(py)?.repr()?.to_string())
}

/// The id of a node object: a [`ScriptNode`], or anything that names one in `_id`.
fn id_of(node: &Bound<'_, PyAny>) -> PyResult<u64> {
    match node.cast::<ScriptNode>() {
        Ok(node) => Ok(node.get().id.0),
        Err(_) => node.getattr("_id")?.extract(),
    }
}

/// A node of the compositor's tree, by id. The typed subclasses add the schema's properties, methods and signals.
#[pyclass(subclass, frozen, name = "Node", module = "gnoblin._base")]
pub(crate) struct ScriptNode {
    id: NodeId,
}

/// The typed handle for node `id`: the class registered for its schema name, or [`ScriptNode`] for one the generated classes do not know (or
/// one that has left the tree and cannot be asked). `class` is the schema name when the caller knows it.
///
/// # Errors
///
/// The core refused the question for another reason than the node being gone, or the class could not be built.
pub(crate) fn of<'py>(py: Python<'py>, id: u64, class: Option<&str>) -> PyResult<Bound<'py, PyAny>> {
    let name = match class {
        Some(name) => Some(name.to_owned()),
        None => match tree::node_class(id) {
            Ok(name) => Some(name),
            Err(error) if error.is_instance_of::<NodeGoneError>(py) => None,
            Err(error) => return Err(error),
        },
    };
    let typed = match name {
        Some(name) => classes(py).get_item(name)?,
        None => None,
    };
    match typed {
        Some(class) => class.call1((id,)),
        None => py.get_type::<ScriptNode>().call1((id,)),
    }
}

#[pymethods]
impl ScriptNode {
    #[new]
    fn new(id: u64) -> Self {
        Self { id: NodeId(id) }
    }

    /// The schema name a generated class sets; a plain `Node` is `node`.
    #[classattr]
    fn __class_name__() -> &'static str {
        "node"
    }

    #[classmethod]
    #[pyo3(signature = (**kwargs))]
    fn __init_subclass__(cls: &Bound<'_, PyType>, kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        if let Some(kwargs) = kwargs.filter(|kwargs| !kwargs.is_empty()) {
            return Err(PyTypeError::new_err(format!(
                "{}.__init_subclass__() takes no keyword arguments ({})",
                cls.name()?,
                kwargs.keys()
            )));
        }
        let own = cls.getattr("__dict__")?.call_method1("get", ("__class_name__",))?;
        if !own.is_none() {
            classes(cls.py()).set_item(own, cls)?;
        }
        Ok(())
    }

    /// Makes the typed handle for node `id`, whose class is `class_name` when known.
    #[staticmethod]
    #[pyo3(name = "_of", signature = (id, class_name = None))]
    fn py_of(py: Python<'_>, id: u64, class_name: Option<&str>) -> PyResult<Py<PyAny>> {
        Ok(of(py, id, class_name)?.unbind())
    }

    #[getter(_id)]
    fn private_id(&self) -> u64 {
        self.id.0
    }

    /// What the runtime's conversions look for: a value with this attribute is a reference to that node.
    #[getter(_gnoblin_node_id)]
    fn gnoblin_node_id(&self) -> u64 {
        self.id.0
    }

    /// The node's id in the tree.
    #[getter]
    fn node_id(&self) -> u64 {
        self.id.0
    }

    /// Whether the node is in the tree now.
    fn is_alive(&self) -> PyResult<bool> {
        tree::node_exists(self.id.0)
    }

    /// The schema name of the node's class.
    fn class_name(&self) -> PyResult<String> {
        tree::node_class(self.id.0)
    }

    /// The node's children, in order.
    fn children<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
        tree::node_children(self.id.0)?
            .into_iter()
            .map(|child| of(py, child, None))
            .collect()
    }

    /// Reads a property by its schema name.
    fn get<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        tree::node_get(py, self.id.0, name)
    }

    /// Writes a property by its schema name; `seconds` asks the compositor to animate an animated property.
    #[pyo3(signature = (name, value, seconds = 0.0))]
    fn set(&self, name: &str, value: &Bound<'_, PyAny>, seconds: f64) -> PyResult<()> {
        tree::node_set(self.id.0, name, value, seconds)
    }

    /// Calls a method by its schema name.
    #[pyo3(signature = (method, *args, on_answer = None))]
    fn call(&self, method: &str, args: &Bound<'_, PyTuple>, on_answer: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        tree::node_call(self.id.0, method, Some(args), on_answer)
    }

    /// Puts the node under `parent` at `index` (the top when `None`), moving it from where it was.
    #[pyo3(signature = (parent, index = None))]
    fn insert_into(&self, parent: &Bound<'_, PyAny>, index: Option<usize>) -> PyResult<()> {
        tree::node_insert(self.id.0, id_of(parent)?, index)
    }

    /// Destroys the node and everything beneath it. To close a window, call its `close` method.
    fn destroy(&self) -> PyResult<()> {
        tree::node_destroy(self.id.0)
    }

    fn __richcmp__(&self, py: Python<'_>, other: &Bound<'_, PyAny>, op: CompareOp) -> PyResult<Py<PyAny>> {
        let same = other.cast::<Self>().is_ok_and(|other| other.get().id.0 == self.id.0);
        match op {
            CompareOp::Eq => same.into_py_any(py),
            CompareOp::Ne => (!same).into_py_any(py),
            _ => Ok(py.NotImplemented()),
        }
    }

    fn __hash__(&self) -> isize {
        // `hash(int)` of an id: the id itself, but for the values Python reserves.
        match isize::try_from(self.id.0) {
            Ok(-1) | Err(_) => -2,
            Ok(id) => id,
        }
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        Ok(format!("<{} {}>", slf.get_type().name()?, slf.get().id.0))
    }
}

/// A property of a node class, read and written by name.
#[pyclass(name = "Property", module = "gnoblin._base")]
pub(crate) struct PropertyDescriptor {
    #[pyo3(get)]
    name: String,
    #[pyo3(get, name = "type")]
    ty: Py<PyAny>,
    #[pyo3(get)]
    readonly: bool,
    #[pyo3(get)]
    animated: bool,
    #[pyo3(get)]
    attribute: String,
}

#[pymethods]
impl PropertyDescriptor {
    #[new]
    #[pyo3(signature = (name, r#type = None, readonly = false, animated = false))]
    fn new(py: Python<'_>, name: String, r#type: Option<Py<PyAny>>, readonly: bool, animated: bool) -> Self {
        Self {
            attribute: name.clone(),
            name,
            ty: r#type.unwrap_or_else(|| py.get_type::<pyo3::types::PyAny>().into_any().unbind()),
            readonly,
            animated,
        }
    }

    fn __set_name__(&mut self, _owner: &Bound<'_, PyAny>, attribute: String) {
        self.attribute = attribute;
    }

    /// `Property[int]`: the class is generic in the type of its value, for annotations.
    #[classmethod]
    fn __class_getitem__<'py>(cls: &Bound<'py, PyType>, item: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        cls.py().import("types")?.getattr("GenericAlias")?.call1((cls, item))
    }

    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        node: Option<&Bound<'py, PyAny>>,
        owner: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = owner;
        match node {
            None => Ok(slf.clone().into_any()),
            Some(node) => tree::node_get(slf.py(), id_of(node)?, &slf.borrow().name),
        }
    }

    fn __set__(&self, node: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.readonly {
            return Err(PyAttributeError::new_err(format!(
                "{}.{} is read-only",
                node.get_type().name()?,
                self.attribute
            )));
        }
        tree::node_set(id_of(node)?, &self.name, value, 0.0)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Property({}, readonly={}, animated={})",
            repr_of(py, &self.name)?,
            if self.readonly { "True" } else { "False" },
            if self.animated { "True" } else { "False" }
        ))
    }
}

/// A method of a node class. Calling it asks the compositor; it returns `None` and the answer, if wanted, goes to
/// `on_answer(result: bytes, error: str | None)`.
#[pyclass(name = "Method", module = "gnoblin._base")]
pub(crate) struct MethodDescriptor {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    params: Py<PyTuple>,
    #[pyo3(get)]
    returns: Py<PyAny>,
    #[pyo3(get)]
    attribute: String,
    /// How many of the parameters a call must give: those that are not marked optional.
    required: usize,
}

#[pymethods]
impl MethodDescriptor {
    #[new]
    #[pyo3(signature = (name, params = None, returns = None))]
    fn new(
        py: Python<'_>,
        name: String,
        params: Option<&Bound<'_, PyAny>>,
        returns: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let params: Bound<'_, PyTuple> = match params {
            Some(params) => py.get_type::<PyTuple>().call1((params,))?.cast_into()?,
            None => PyTuple::empty(py),
        };
        let mut required = 0;
        for param in &params {
            let optional = param.len()? >= 3 && param.get_item(2)?.is_truthy()?;
            if !optional {
                required += 1;
            }
        }
        Ok(Self {
            attribute: name.clone(),
            name,
            params: params.unbind(),
            returns: returns.unwrap_or_else(|| py.None()),
            required,
        })
    }

    fn __set_name__(&mut self, _owner: &Bound<'_, PyAny>, attribute: String) {
        self.attribute = attribute;
    }

    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        node: Option<&Bound<'py, PyAny>>,
        owner: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let _ = owner;
        match node {
            None => Ok(slf.clone().into_any()),
            Some(node) => Ok(Bound::new(
                slf.py(),
                BoundMethod {
                    method: slf.clone().unbind(),
                    node: node.clone().unbind(),
                },
            )?
            .into_any()),
        }
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Method({}, {})",
            repr_of(py, &self.name)?,
            self.params.bind(py).repr()?
        ))
    }
}

/// A [`MethodDescriptor`] of one node: calling it calls the method on that node.
#[pyclass(frozen, name = "_BoundMethod", module = "gnoblin._base")]
struct BoundMethod {
    method: Py<MethodDescriptor>,
    node: Py<PyAny>,
}

#[pymethods]
impl BoundMethod {
    #[pyo3(signature = (*args, on_answer = None))]
    fn __call__(
        &self,
        py: Python<'_>,
        args: &Bound<'_, PyTuple>,
        on_answer: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let method = self.method.bind(py).borrow();
        let total = method.params.bind(py).len();
        if !(method.required..=total).contains(&args.len()) {
            let count = if method.required == total {
                method.required.to_string()
            } else {
                format!("{} to {total}", method.required)
            };
            return Err(PyTypeError::new_err(format!(
                "{}() takes {count} argument(s), {} given",
                method.name,
                args.len()
            )));
        }
        tree::node_call(id_of(self.node.bind(py))?, &method.name, Some(args), on_answer)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "<method {} of {}>",
            self.method.bind(py).borrow().name,
            self.node.bind(py).repr()?
        ))
    }
}

/// A signal of a node class. `Window.closed` listens to every window, `window.closed` to that one; both are used as a decorator
/// (`@Window.closed`) or called with the function (`Window.closed(fn)`), which returns the function. `.subscribe(fn)` returns the
/// `Subscription` instead.
#[pyclass(name = "Signal", module = "gnoblin._base")]
pub(crate) struct SignalDescriptor {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    fields: Py<PyTuple>,
    #[pyo3(get)]
    attribute: String,
}

#[pymethods]
impl SignalDescriptor {
    #[new]
    #[pyo3(signature = (name, fields = None))]
    fn new(py: Python<'_>, name: String, fields: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let fields: Bound<'_, PyTuple> = match fields {
            Some(fields) => py.get_type::<PyTuple>().call1((fields,))?.cast_into()?,
            None => PyTuple::empty(py),
        };
        Ok(Self {
            attribute: name.clone(),
            name,
            fields: fields.unbind(),
        })
    }

    fn __set_name__(&mut self, _owner: &Bound<'_, PyAny>, attribute: String) {
        self.attribute = attribute;
    }

    fn __get__<'py>(
        slf: &Bound<'py, Self>,
        node: Option<&Bound<'py, PyAny>>,
        owner: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = slf.py();
        let hook = match node {
            None => SignalHook {
                owner: owner.map(|owner| owner.clone().unbind()),
                signal: slf.clone().unbind(),
                node: None,
            },
            Some(node) => SignalHook {
                owner: Some(node.get_type().into_any().unbind()),
                signal: slf.clone().unbind(),
                node: Some(node.clone().unbind()),
            },
        };
        Ok(Bound::new(py, hook)?.into_any())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Signal({}, {})",
            repr_of(py, &self.name)?,
            self.fields.bind(py).repr()?
        ))
    }
}

/// A [`SignalDescriptor`] seen from a class or from one node.
#[pyclass(frozen, name = "_SignalHook", module = "gnoblin._base")]
struct SignalHook {
    owner: Option<Py<PyAny>>,
    signal: Py<SignalDescriptor>,
    node: Option<Py<PyAny>>,
}

#[pymethods]
impl SignalHook {
    /// Registers `callback(source, values)`; returns the `Subscription`.
    fn subscribe(&self, py: Python<'_>, callback: &Bound<'_, PyAny>) -> PyResult<SubscriptionHandle> {
        let callback = match &self.node {
            None => callback.clone(),
            Some(node) => {
                let (inner, node) = (callback.clone().unbind(), node.clone_ref(py));
                PyCFunction::new_closure(
                    py,
                    None,
                    None,
                    move |args: &Bound<'_, PyTuple>, _| -> PyResult<Py<PyAny>> {
                        let py = args.py();
                        if args.get_item(0)?.eq(node.bind(py))? {
                            Ok(inner.bind(py).call1(args)?.unbind())
                        } else {
                            Ok(py.None())
                        }
                    },
                )?
                .into_any()
            }
        };
        let class_name = match &self.owner {
            Some(owner) => owner
                .bind(py)
                .getattr_opt("__class_name__")?
                .map_or_else(|| Ok("node".to_owned()), |name| name.extract())?,
            None => "node".to_owned(),
        };
        let token = events::on_signal(&class_name, &self.signal.bind(py).borrow().name, &callback)?;
        Ok(SubscriptionHandle::new(token))
    }

    fn __call__<'py>(&self, py: Python<'py>, callback: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let subscription = Bound::new(py, self.subscribe(py, callback)?)?;
        remember(py, callback, &subscription)?;
        Ok(callback.clone())
    }
}

/// Fills the module `gnoblin._base`.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<ScriptNode>()?;
    module.add_class::<PropertyDescriptor>()?;
    module.add_class::<MethodDescriptor>()?;
    module.add_class::<SignalDescriptor>()?;
    module.add("CLASSES", classes(module.py()))?;
    Ok(())
}
