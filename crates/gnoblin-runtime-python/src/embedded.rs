//! The generated half of the `gnoblin` package, served from the library itself.
//!
//! `gnoblin-sdkgen` renders the typed node classes, the ABI constants, the verbs and the std bindings (`gnoblin._generated.*` and
//! `gnoblin.std`) in `build.rs`, and nothing of them is committed or installed as source: they are embedded here, so a module can
//! never describe a different catalog from the libraries the runtime was built with. The hand-written package on disk finds them
//! through a finder on `sys.meta_path`, ahead of the path finder, so an older file of the same name left in a prefix is never
//! imported instead.
//!
//! The finder answers only for the names in [`MODULES`]. Everything else (`gnoblin`, `gnoblin._base`, the shell's own modules) goes
//! on to the path finder as before. The loader compiles the embedded source and runs it in the module's namespace, as the source
//! file loader does, so a module behaves like any other.

use pyo3::prelude::*;
use pyo3::types::PyDict;

// `MODULES`: `(dotted name, is a package, source)` for every generated module. `build.rs` writes the table.
include!(concat!(env!("OUT_DIR"), "/python_embedded.rs"));

fn find(name: &str) -> Option<&'static (&'static str, bool, &'static str)> {
    MODULES.iter().find(|(module, _, _)| *module == name)
}

/// The finder: a spec for each embedded module, `None` for any other name.
#[pyclass(frozen, module = "gnoblin_runtime")]
struct EmbeddedFinder;

#[pymethods]
impl EmbeddedFinder {
    #[pyo3(signature = (fullname, _path = None, _target = None))]
    fn find_spec<'py>(
        &self,
        py: Python<'py>,
        fullname: &str,
        _path: Option<&Bound<'py, PyAny>>,
        _target: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let Some(&(name, package, _)) = find(fullname) else {
            return Ok(None);
        };
        let loader = Py::new(py, EmbeddedLoader)?;
        let kwargs = PyDict::new(py);
        kwargs.set_item("is_package", package)?;
        kwargs.set_item("origin", format!("<generated:{name}>"))?;
        let spec = py
            .import("importlib.machinery")?
            .getattr("ModuleSpec")?
            .call((name, loader), Some(&kwargs))?;
        Ok(Some(spec))
    }

    /// Nothing is cached, so there is nothing to invalidate; the import system calls it on every finder.
    fn invalidate_caches(&self) {}
}

/// The loader: compiles the embedded source and runs it in the module's own namespace.
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
        let Some(&(_, _, source)) = find(&name) else {
            return Err(pyo3::exceptions::PyImportError::new_err(format!(
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
