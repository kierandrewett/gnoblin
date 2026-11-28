//! The `gnoblin` package, served from the library itself.
//!
//! Nothing of the package is read from disk. `gnoblin-sdkgen` renders the typed node classes, the ABI constants, the verbs and the object
//! classes (`gnoblin._generated.*`, `gnoblin.std`, `gnoblin.objects.*`) in `build.rs`, and the rest of the package (`gnoblin`,
//! `gnoblin._base`, `gnoblin.api`, `gnoblin.errors`, `gnoblin.events`, `gnoblin.host`) is Rust in this crate.
//! A module can never describe a different catalog from the libraries the runtime was built with, and a prefix needs no Python source.
//!
//! The finder sits first on `sys.meta_path`, so an older file of the same name left in a prefix is never imported instead. It answers
//! only for the names of the package. Everything else (the shell's own modules, the standard library) goes on to the finders after it.
//! A generated module is compiled from its embedded source and run in the module's namespace, as the source file loader does; a module
//! of this crate is built by its Rust function when the import system executes it.

use pyo3::exceptions::PyImportError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::{descriptors, errors, singleton, subscription};

// `MODULES`: `(dotted name, is a package, source)` for every generated module. `build.rs` writes the table.
include!(concat!(env!("OUT_DIR"), "/python_embedded.rs"));

/// A module this crate builds: its name, whether it is a package, and what fills it.
struct RustModule {
    name: &'static str,
    package: bool,
    build: fn(&Bound<'_, PyModule>) -> PyResult<()>,
}

const NATIVE: [RustModule; 6] = [
    RustModule {
        name: "gnoblin",
        package: true,
        build: build_package,
    },
    RustModule {
        name: "gnoblin._base",
        package: false,
        build: descriptors::build,
    },
    RustModule {
        name: "gnoblin.api",
        package: false,
        build: singleton::build,
    },
    RustModule {
        name: "gnoblin.errors",
        package: false,
        build: errors::build,
    },
    RustModule {
        name: "gnoblin.events",
        package: false,
        build: subscription::build,
    },
    RustModule {
        name: "gnoblin.host",
        package: false,
        build: build_host,
    },
];

/// What the package says of itself, as `gnoblin.__doc__`.
const DOC: &str = "Gnoblin shells in Python.

A shell is an ordinary script. Run it with ``gnoblin ./shell.py``; the shell reloads it when it or a module it imports is saved::

    from gnoblin import gnoblin

    @gnoblin.on_client_created
    def place(window):
        window.x, window.y = 0, 0

    @gnoblin.bind(\"<Super>Return\")
    def terminal():
        gnoblin.spawn(\"foot\")

See ``docs/python-shell.md``. The package root has the classes of the object model (``Gnoblin``, ``Client``, ``Output``, ...), generated
from the schema, the ``gnoblin`` object, the ``verbs`` and the errors. The tree layer under them (the node classes ``Window``, ``Seat``,
... and the event values) is ``gnoblin.host``, which the runtime and its tests use.
";

/// The error classes the root re-exports from `gnoblin.errors`: the one thing of the runtime besides the `gnoblin` object and the `verbs`
/// that a script takes from the package root.
const ERRORS: [&str; 9] = [
    "BusyError",
    "GnoblinError",
    "MemberError",
    "NodeGoneError",
    "RefusedError",
    "RetiredError",
    "StaleError",
    "ThreadError",
    "UnsupportedError",
];

/// The event values `gnoblin.host` re-exports from `gnoblin.events`.
const EVENTS: [&str; 9] = [
    "ChordEvent",
    "GestureEvent",
    "NodeEvent",
    "PointerButton",
    "PointerDrag",
    "Process",
    "ProcessExit",
    "Subscription",
    "WindowRequest",
];

/// Fills the package `gnoblin`: the classes of the object model, the object a script uses, the verbs and the errors.
fn build_package(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("__doc__", DOC)?;
    module.add("__version__", "0.2.0")?;
    // The node classes are not names of the root, but defining them is what fills `CLASSES`, which the tree reads to give a node its typed
    // handle. They are defined with the package, as they were when the root exported them, and `gnoblin.host` only names them.
    py.import("gnoblin._generated.classes")?;
    module.add("verbs", py.import("gnoblin._generated.verbs")?)?;
    module.add("gnoblin", py.import("gnoblin.api")?.getattr("gnoblin")?)?;
    let errors = py.import("gnoblin.errors")?;
    for name in ERRORS {
        module.add(name, errors.getattr(name)?)?;
    }
    let all = PyList::new(py, ERRORS)?;
    all.append("gnoblin")?;
    all.append("verbs")?;
    let model = py.import("gnoblin.objects.model")?;
    for name in defined_in(&model)? {
        module.add(name.as_str(), model.getattr(name.as_str())?)?;
        if !all.contains(name.as_str())? {
            all.append(name)?;
        }
    }
    module.add("__all__", all)?;
    Ok(())
}

/// Fills `gnoblin.host`, the tree layer under the object model: the node classes generated from the schema and their descriptors, and
/// the event values the `gnoblin` object hands to a callback. The runtime and its tests use it; a shell is written on the package root.
fn build_host(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    let base = py.import("gnoblin._base")?;
    let classes = py.import("gnoblin._generated.classes")?;
    let events = py.import("gnoblin.events")?;
    let all = PyList::empty(py);
    let add = |from: &Bound<'_, PyModule>, name: &str| -> PyResult<()> {
        module.add(name, from.getattr(name)?)?;
        all.append(name)
    };
    for name in ["CLASSES", "Method", "Node", "Property", "Signal"] {
        add(&base, name)?;
    }
    let generated: Vec<String> = classes.getattr("__all__")?.extract()?;
    for name in &generated {
        add(&classes, name)?;
    }
    add(&py.import("gnoblin.api")?, "Event")?;
    for name in EVENTS {
        add(&events, name)?;
    }
    module.add("__all__", all)?;
    Ok(())
}

/// The public names a generated module defines itself, as opposed to the ones it imported.
fn defined_in(module: &Bound<'_, PyModule>) -> PyResult<Vec<String>> {
    let own: String = module.getattr("__name__")?.extract()?;
    let mut names = Vec::new();
    for name in module.dir()?.iter() {
        let name: String = name.extract()?;
        if name.starts_with('_') {
            continue;
        }
        let item = module.getattr(name.as_str())?;
        let home = item
            .getattr("__module__")
            .and_then(|home| home.extract::<String>())
            .ok();
        if home.as_deref() == Some(own.as_str()) {
            names.push(name);
        }
    }
    Ok(names)
}

/// The generated module `name`, if there is one.
fn generated(name: &str) -> Option<&'static (&'static str, bool, &'static str)> {
    MODULES.iter().find(|(module, _, _)| *module == name)
}

/// The module of this crate `name`, if there is one.
fn native(name: &str) -> Option<&'static RustModule> {
    NATIVE.iter().find(|module| module.name == name)
}

/// The finder: a spec for each module of the package, `None` for any other name.
#[pyclass(frozen, module = "gnoblin_runtime")]
struct EmbeddedFinder;

#[pymethods]
impl EmbeddedFinder {
    #[pyo3(signature = (fullname, path = None, target = None))]
    fn find_spec<'py>(
        &self,
        py: Python<'py>,
        fullname: &str,
        path: Option<&Bound<'py, PyAny>>,
        target: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let _ = (path, target);
        let (name, package, origin) = if let Some(module) = native(fullname) {
            (module.name, module.package, format!("<native:{}>", module.name))
        } else if let Some(&(name, package, _)) = generated(fullname) {
            (name, package, format!("<generated:{name}>"))
        } else {
            return Ok(None);
        };
        let kwargs = PyDict::new(py);
        kwargs.set_item("is_package", package)?;
        kwargs.set_item("origin", origin)?;
        let spec = py
            .import("importlib.machinery")?
            .getattr("ModuleSpec")?
            .call((name, Py::new(py, EmbeddedLoader)?), Some(&kwargs))?;
        Ok(Some(spec))
    }

    /// Nothing is cached, so there is nothing to invalidate; the import system calls it on every finder.
    fn invalidate_caches(&self) {}
}

/// The loader: runs the embedded source in the module's own namespace, or the Rust function that fills the module.
#[pyclass(frozen, module = "gnoblin_runtime")]
struct EmbeddedLoader;

#[pymethods]
impl EmbeddedLoader {
    /// The default module object is what is wanted.
    fn create_module(&self, _spec: &Bound<'_, PyAny>) -> Option<Py<PyAny>> {
        None
    }

    fn exec_module(&self, py: Python<'_>, module: &Bound<'_, PyAny>) -> PyResult<()> {
        let name: String = module.getattr("__name__")?.extract()?;
        if let Some(native) = native(&name) {
            return (native.build)(module.cast::<PyModule>()?);
        }
        let Some(&(_, _, source)) = generated(&name) else {
            return Err(PyImportError::new_err(format!(
                "`{name}` is not one of the modules this runtime carries"
            )));
        };
        let builtins = py.import("builtins")?;
        let code = builtins.call_method1("compile", (source, format!("<generated:{name}>"), "exec"))?;
        builtins.call_method1("exec", (code, module.getattr("__dict__")?))?;
        Ok(())
    }
}

/// Puts the finder first on `sys.meta_path` of this interpreter.
pub(crate) fn install(py: Python<'_>) -> PyResult<()> {
    let finder = Py::new(py, EmbeddedFinder)?;
    py.import("sys")?
        .getattr("meta_path")?
        .call_method1("insert", (0, finder))?;
    Ok(())
}
