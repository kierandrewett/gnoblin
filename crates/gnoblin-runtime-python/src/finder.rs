//! The `gnoblin` package, served from the library itself.
//!
//! Nothing of the package is read from disk. `gnoblin-sdkgen` renders the typed node classes, the ABI constants, the verbs and the std
//! bindings (`gnoblin._generated.*`, `gnoblin.std`, `gnoblin.objects.*`) in `build.rs`, and the rest of the package (`gnoblin`,
//! `gnoblin._base`, `gnoblin.api`, `gnoblin.errors`, `gnoblin.events`, `gnoblin._std_base`, `gnoblin._std_sugar`) is Rust in this crate.
//! A module can never describe a different catalog from the libraries the runtime was built with, and a prefix needs no Python source.
//!
//! The finder sits first on `sys.meta_path`, so an older file of the same name left in a prefix is never imported instead. It answers
//! only for the names of the package. Everything else (the shell's own modules, the standard library) goes on to the finders after it.
//! A generated module is compiled from its embedded source and run in the module's namespace, as the source file loader does; a module
//! of this crate is built by its Rust function when the import system executes it.

use pyo3::exceptions::PyImportError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::{descriptors, errors, singleton, std_base, std_sugar, subscription};

// `MODULES`: `(dotted name, is a package, source)` for every generated module. `build.rs` writes the table.
include!(concat!(env!("OUT_DIR"), "/python_embedded.rs"));

/// A module this crate builds: its name, whether it is a package, and what fills it.
struct Native {
    name: &'static str,
    package: bool,
    build: fn(&Bound<'_, PyModule>) -> PyResult<()>,
}

const NATIVE: [Native; 7] = [
    Native {
        name: "gnoblin",
        package: true,
        build: build_package,
    },
    Native {
        name: "gnoblin._base",
        package: false,
        build: descriptors::build,
    },
    Native {
        name: "gnoblin.api",
        package: false,
        build: singleton::build,
    },
    Native {
        name: "gnoblin.errors",
        package: false,
        build: errors::build,
    },
    Native {
        name: "gnoblin.events",
        package: false,
        build: subscription::build,
    },
    Native {
        name: "gnoblin._std_base",
        package: false,
        build: std_base::build,
    },
    Native {
        name: "gnoblin._std_sugar",
        package: false,
        build: std_sugar::build,
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

See ``docs/python-shell.md``. The typed classes of the tree (``Window``, ``Seat``, ...) are generated from the schema; the rest of
the package is a thin spelling of the core's C ABI and holds no compositor logic.
";

/// The names `gnoblin` exports besides the generated classes.
const EXPORTS: [&str; 27] = [
    "BusyError",
    "CLASSES",
    "ChordEvent",
    "Event",
    "GestureEvent",
    "Gnoblin",
    "GnoblinError",
    "MemberError",
    "Method",
    "Node",
    "NodeEvent",
    "NodeGoneError",
    "PointerButton",
    "PointerDrag",
    "Process",
    "ProcessExit",
    "Property",
    "RefusedError",
    "RetiredError",
    "Signal",
    "StaleError",
    "Subscription",
    "ThreadError",
    "UnsupportedError",
    "WindowRequest",
    "gnoblin",
    "verbs",
];

/// Fills the package `gnoblin`: the base, the generated classes and verbs, the object a script uses, the errors and the event values.
fn build_package(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("__doc__", DOC)?;
    module.add("__version__", "0.2.0")?;
    let copy = |from: &Bound<'_, PyModule>, names: &[&str]| -> PyResult<()> {
        for name in names {
            module.add(*name, from.getattr(*name)?)?;
        }
        Ok(())
    };
    copy(
        &py.import("gnoblin._base")?,
        &["CLASSES", "Method", "Node", "Property", "Signal"],
    )?;
    let classes = py.import("gnoblin._generated.classes")?;
    let generated: Vec<String> = classes.getattr("__all__")?.extract()?;
    for name in &generated {
        module.add(name.as_str(), classes.getattr(name.as_str())?)?;
    }
    module.add("verbs", py.import("gnoblin._generated.verbs")?)?;
    copy(&py.import("gnoblin.api")?, &["Event", "Gnoblin", "gnoblin"])?;
    copy(
        &py.import("gnoblin.errors")?,
        &[
            "BusyError",
            "GnoblinError",
            "MemberError",
            "NodeGoneError",
            "RefusedError",
            "RetiredError",
            "StaleError",
            "ThreadError",
            "UnsupportedError",
        ],
    )?;
    copy(
        &py.import("gnoblin.events")?,
        &[
            "ChordEvent",
            "GestureEvent",
            "NodeEvent",
            "PointerButton",
            "PointerDrag",
            "Process",
            "ProcessExit",
            "Subscription",
            "WindowRequest",
        ],
    )?;
    let all = PyList::new(py, EXPORTS)?;
    for name in &generated {
        all.append(name)?;
    }
    module.add("__all__", all)?;
    Ok(())
}

/// The generated module `name`, if there is one.
fn generated(name: &str) -> Option<&'static (&'static str, bool, &'static str)> {
    MODULES.iter().find(|(module, _, _)| *module == name)
}

/// The module of this crate `name`, if there is one.
fn native(name: &str) -> Option<&'static Native> {
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
