//! Loads the shell script, and the modules it imports, one isolated generation at a time.
//!
//! [`load`] *stages* a generation, [`activate`] makes it the one that serves when the host commits it, and [`abandon`] throws it away. A
//! reload is the same sequence again.
//!
//! What makes a generation isolated:
//!
//! - The script and every module found beside it (under its directory, at any depth) are compiled from their source each time, never from
//!   a `.pyc`: an edit that keeps the file's size and lands in the same second as the last one is still seen.
//! - Before a generation is evaluated, the modules the previous one imported are taken out of `sys.modules`; the new one imports them
//!   fresh. If the new one fails, they are put back and nothing of the failed attempt stays, so the running generation, which still holds
//!   its own module objects, goes on with exactly the code it had, and an `import` it makes later finds its own modules.
//! - `__main__` is the generation's script while it runs, as under `python shell.py`, and is restored after a failure.
//! - Libraries installed on the system (everything not beside the script) are shared by every generation, as in any Python process: a
//!   generation's monkey-patches of them are not undone.
//!
//! The listeners a generation registered are not tracked here. They belong to the generation's context in the core, which removes them
//! when the generation retires; a script that is evaluated again registers exactly what the new text registers.
//!
//! Two classes do the importing for the files beside the script: [`FreshFinder`] sits on `sys.meta_path` ahead of the path finder and
//! answers only for names under the script's directory, and [`FreshLoader`] compiles from the file and records what the file looked like
//! when it was read, which is what [`crate::watch`] compares to see an edit.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyModule, PySlice};

use crate::callback::traceback_text;
use crate::watch::Stamp;

/// The names a script's own module can never shadow: the package and the runtime's built-in module.
const RESERVED: [&str; 2] = ["gnoblin", "_gnoblin_core"];

/// `os.path.realpath`.
fn realpath(py: Python<'_>, path: &str) -> PyResult<String> {
    py.import("os")?
        .getattr("path")?
        .call_method1("realpath", (path,))?
        .extract()
}

/// Whether `path` is `root` or beneath it, as the file system names it.
fn inside(py: Python<'_>, path: &str, root: &str) -> PyResult<bool> {
    let path = realpath(py, path)?;
    Ok(path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/')))
}

/// Finds the modules beside the script, and gives them a loader that never reads a `.pyc`.
#[pyclass(frozen, name = "_Finder", module = "gnoblin")]
pub(crate) struct FreshFinder {
    root: String,
    /// Each file a module of this generation was read from, as it looked when it was read.
    stamps: Mutex<BTreeMap<String, Stamp>>,
}

impl FreshFinder {
    fn new(root: String) -> Self {
        Self {
            root,
            stamps: Mutex::new(BTreeMap::new()),
        }
    }

    fn record(&self, path: &str) {
        let stamp = Stamp::of(Path::new(path));
        self.stamps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), stamp);
    }

    fn stamps(&self) -> BTreeMap<String, Stamp> {
        self.stamps.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[pymethods]
impl FreshFinder {
    #[pyo3(signature = (fullname, path = None, target = None))]
    fn find_spec<'py>(
        slf: &Bound<'py, Self>,
        fullname: &str,
        path: Option<&Bound<'py, PyAny>>,
        target: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Option<Bound<'py, PyAny>>> {
        let py = slf.py();
        let finder = slf.get();
        if RESERVED.contains(&fullname.split('.').next().unwrap_or(fullname)) {
            return Ok(None);
        }
        let search: Vec<String> = match path.filter(|path| !path.is_none()) {
            None => vec![finder.root.clone()],
            Some(path) => {
                let mut inside_root = Vec::new();
                for entry in path.try_iter()? {
                    let entry: String = entry?.extract()?;
                    if inside(py, &entry, &finder.root)? {
                        inside_root.push(entry);
                    }
                }
                if inside_root.is_empty() {
                    return Ok(None);
                }
                inside_root
            }
        };
        let machinery = py.import("importlib.machinery")?;
        let spec = machinery
            .getattr("PathFinder")?
            .call_method1("find_spec", (fullname, search, target))?;
        if spec.is_none() {
            return Ok(None);
        }
        let loader = spec.getattr("loader")?;
        if loader.is_instance(&machinery.getattr("SourceFileLoader")?)? {
            let fresh = FreshLoader {
                name: loader.getattr("name")?.extract()?,
                path: loader.getattr("path")?.extract()?,
                finder: slf.clone().unbind(),
            };
            spec.setattr("loader", Bound::new(py, fresh)?)?;
        }
        Ok(Some(spec))
    }

    /// Nothing is cached, so there is nothing to invalidate; the import system calls it on every finder.
    fn invalidate_caches(&self) {}
}

/// Compiles a module from its source on every import and records what the file looked like when it was read.
#[pyclass(frozen, name = "_FreshLoader", module = "gnoblin")]
pub(crate) struct FreshLoader {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    path: String,
    finder: Py<FreshFinder>,
}

#[pymethods]
impl FreshLoader {
    #[pyo3(signature = (fullname = None))]
    fn get_filename(&self, fullname: Option<&str>) -> String {
        let _ = fullname;
        self.path.clone()
    }

    fn get_data<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &std::fs::read(path)?))
    }

    fn get_source(&self, py: Python<'_>, fullname: &str) -> PyResult<Py<PyAny>> {
        let _ = fullname;
        let bytes = PyBytes::new(py, &std::fs::read(&self.path)?);
        Ok(py
            .import("importlib.util")?
            .call_method1("decode_source", (bytes,))?
            .unbind())
    }

    fn is_package(&self, fullname: &str) -> bool {
        let file = Path::new(&self.path).file_stem().and_then(|stem| stem.to_str());
        file == Some("__init__") && fullname.rsplit('.').next() != Some("__init__")
    }

    fn create_module(&self, spec: &Bound<'_, PyAny>) -> Option<Py<PyAny>> {
        let _ = spec;
        None
    }

    fn get_code<'py>(&self, py: Python<'py>, fullname: &str) -> PyResult<Bound<'py, PyAny>> {
        let _ = fullname;
        // Before the read: an edit during it makes the stamp older than the file, so it is seen.
        self.finder.get().record(&self.path);
        compile(py, &std::fs::read(&self.path)?, &self.path)
    }

    fn exec_module(&self, py: Python<'_>, module: &Bound<'_, PyAny>) -> PyResult<()> {
        let name: String = module.getattr("__name__")?.extract()?;
        let code = self.get_code(py, &name)?;
        py.import("builtins")?
            .call_method1("exec", (code, module.getattr("__dict__")?))?;
        Ok(())
    }
}

/// `compile(source, path, "exec", dont_inherit=True)`.
fn compile<'py>(py: Python<'py>, source: &[u8], path: &str) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("dont_inherit", true)?;
    py.import("builtins")?
        .getattr("compile")?
        .call((PyBytes::new(py, source), path, "exec"), Some(&kwargs))
}

/// What a generation changed in the process, so a failed one can be undone.
struct GenerationStage {
    previous_modules: Py<PyDict>,
    previous_main: Option<Py<PyAny>>,
    previous_finder: Option<Py<FreshFinder>>,
    previous_stamps: BTreeMap<String, Stamp>,
    finder: Py<FreshFinder>,
}

#[derive(Default)]
struct LoaderState {
    serving: Option<GenerationStage>,
    staged: Option<GenerationStage>,
    original_main: Option<Py<PyAny>>,
}

thread_local! {
    static LOADER: RefCell<LoaderState> = RefCell::new(LoaderState::default());
}

/// `sys.modules`.
fn sys_modules<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    Ok(py.import("sys")?.getattr("modules")?.cast_into()?)
}

/// The modules whose loader is a [`FreshLoader`]: those of the script's directory.
fn user_modules<'py>(py: Python<'py>) -> PyResult<Vec<(String, Bound<'py, PyAny>)>> {
    let mut found = Vec::new();
    for item in sys_modules(py)?.items() {
        let (name, module) = item.extract::<(String, Bound<'py, PyAny>)>()?;
        let loader = match module.getattr_opt("__spec__")?.filter(|spec| !spec.is_none()) {
            Some(spec) => spec.getattr_opt("loader")?,
            None => None,
        };
        if loader.is_some_and(|loader| loader.is_instance_of::<FreshLoader>()) {
            found.push((name, module));
        }
    }
    Ok(found)
}

/// Takes every [`FreshFinder`] off `sys.meta_path`.
fn remove_finders(py: Python<'_>) -> PyResult<()> {
    let meta_path = py.import("sys")?.getattr("meta_path")?;
    let kept: Vec<Bound<'_, PyAny>> = meta_path
        .try_iter()?
        .collect::<PyResult<Vec<_>>>()?
        .into_iter()
        .filter(|entry| !entry.is_instance_of::<FreshFinder>())
        .collect();
    meta_path.set_item(PySlice::full(py), kept)
}

/// Puts `finder` on `sys.meta_path` just before the path finder, and no other [`FreshFinder`].
fn install(py: Python<'_>, finder: &Bound<'_, FreshFinder>) -> PyResult<()> {
    remove_finders(py)?;
    let meta_path = py.import("sys")?.getattr("meta_path")?;
    let path_finder = py.import("importlib.machinery")?.getattr("PathFinder")?;
    let mut index = meta_path.len()?;
    for (at, entry) in meta_path.try_iter()?.enumerate() {
        if entry?.is(&path_finder) {
            index = at;
            break;
        }
    }
    meta_path.call_method1("insert", (index, finder))?;
    Ok(())
}

/// The one-line reason (`FILE:LINE: Type: message`) and the full traceback of a failed evaluation.
fn describe(py: Python<'_>, error: &PyErr, root: &str) -> PyResult<(String, String)> {
    py.import("linecache")?.call_method0("clearcache")?;
    let detail = traceback_text(py, error);
    let value = error.value(py);
    let syntax_file = if value.is_instance_of::<pyo3::exceptions::PySyntaxError>() {
        value
            .getattr("filename")?
            .extract::<Option<String>>()?
            .filter(|file| !file.is_empty())
    } else {
        None
    };
    let (place, message) = if let Some(file) = syntax_file {
        let line = value.getattr("lineno")?.extract::<Option<i64>>()?.unwrap_or(0);
        (format!("{file}:{line}"), value.getattr("msg")?.str()?.to_string())
    } else {
        let frames: Vec<Bound<'_, PyAny>> = py
            .import("traceback")?
            .call_method1("extract_tb", (error.traceback(py),))?
            .try_iter()?
            .collect::<PyResult<_>>()?;
        let mut user = Vec::new();
        for frame in &frames {
            if inside(py, &frame.getattr("filename")?.extract::<String>()?, root)? {
                user.push(frame);
            }
        }
        let last = user.last().copied().or(frames.last());
        let place = match last {
            Some(frame) => format!(
                "{}:{}",
                frame.getattr("filename")?.extract::<String>()?,
                frame
                    .getattr("lineno")?
                    .extract::<Option<i64>>()?
                    .map_or_else(|| "None".to_owned(), |line| line.to_string())
            ),
            None => root.to_owned(),
        };
        (place, value.str()?.to_string())
    };
    let first = if message.is_empty() {
        String::new()
    } else {
        message
            .into_pyobject(py)?
            .call_method0("splitlines")?
            .extract::<Vec<String>>()?
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    let reason = format!("{place}: {}: {first}", value.get_type().name()?);
    Ok((reason.trim_end_matches([':', ' ']).to_owned(), detail))
}

/// The files to look at for the next edit: those `extra` names, and every file `finder` read.
fn watch_of(finder: &FreshFinder, extra: &BTreeMap<String, Stamp>) -> Vec<(String, i128, u64, u64)> {
    let mut all = extra.clone();
    all.extend(finder.stamps());
    all.into_iter().map(|(path, stamp)| stamp.entry(&path)).collect()
}

/// What staging a generation came to: whether it evaluated, why not, the traceback, and the files to watch.
pub(crate) struct Loaded {
    pub(crate) ok: bool,
    pub(crate) reason: String,
    pub(crate) detail: String,
    pub(crate) watch: Vec<(String, i128, u64, u64)>,
}

/// Stages a generation: evaluates `script` in a fresh namespace beside the one that serves.
///
/// On failure `reason` and `detail` are the one-line reason and the traceback, and nothing of the attempt stays. `watch` is the files to
/// look at for the next edit: those of the generation that would serve if the host commits this one, or, after a failure, those of the one
/// that serves plus the attempt's (and the script's directory, so a module that was missing and is then created is seen).
///
/// # Errors
///
/// A failure of the loader itself, not of the script.
pub(crate) fn load(py: Python<'_>, script: &str) -> PyResult<Loaded> {
    if LOADER.with(|loader| loader.borrow().staged.is_some()) {
        abandon(py)?;
    }
    let path = realpath(py, script)?;
    let root = Path::new(&path)
        .parent()
        .map_or_else(String::new, |parent| parent.display().to_string());
    let modules = sys_modules(py)?;
    let previous_modules = PyDict::new(py);
    for (name, module) in user_modules(py)? {
        previous_modules.set_item(name, module)?;
    }
    let previous_main = modules.get_item("__main__")?;
    let (previous_stamps, previous_finder) = LOADER.with(|loader| {
        let mut loader = loader.borrow_mut();
        if loader.original_main.is_none() {
            loader.original_main = previous_main.as_ref().map(|main| main.clone().unbind());
        }
        match &loader.serving {
            Some(serving) => (serving.finder.get().stamps(), Some(serving.finder.clone_ref(py))),
            None => (BTreeMap::new(), None),
        }
    });
    for name in previous_modules.keys() {
        modules.del_item(name).ok();
    }
    py.import("importlib")?.call_method0("invalidate_caches")?;
    let finder = Bound::new(py, FreshFinder::new(root.clone()))?;
    install(py, &finder)?;
    let sys_path = py.import("sys")?.getattr("path")?;
    if !sys_path.contains(&root)? {
        sys_path.call_method1("insert", (0, &root))?;
    }
    let stage = GenerationStage {
        previous_modules: previous_modules.unbind(),
        previous_main: previous_main.map(Bound::unbind),
        previous_finder,
        previous_stamps,
        finder: finder.clone().unbind(),
    };
    let evaluated = (|| -> PyResult<()> {
        finder.get().record(&path);
        let source = std::fs::read(&path)?;
        let code = compile(py, &source, &path)?;
        let main = PyModule::new(py, "__main__")?;
        main.setattr("__file__", &path)?;
        main.setattr("__builtins__", py.import("builtins")?)?;
        modules.set_item("__main__", &main)?;
        py.import("sys")?.setattr("argv", PyList::new(py, [&path])?)?;
        py.import("builtins")?.call_method1("exec", (code, main.dict()))?;
        Ok(())
    })();
    match evaluated {
        Ok(()) => {
            let watch = watch_of(finder.get(), &BTreeMap::new());
            LOADER.with(|loader| loader.borrow_mut().staged = Some(stage));
            Ok(Loaded {
                ok: true,
                reason: String::new(),
                detail: String::new(),
                watch,
            })
        }
        Err(error) => {
            let (reason, detail) = describe(py, &error, &root)?;
            let mut watch = watch_of(finder.get(), &stage.previous_stamps);
            watch.push(Stamp::of(Path::new(&root)).entry(&root));
            undo(py, &stage)?;
            Ok(Loaded {
                ok: false,
                reason,
                detail,
                watch,
            })
        }
    }
}

/// Puts the process as it was before `stage` was evaluated.
fn undo(py: Python<'_>, stage: &GenerationStage) -> PyResult<()> {
    let modules = sys_modules(py)?;
    for (name, _) in user_modules(py)? {
        modules.del_item(name).ok();
    }
    modules.call_method1("update", (stage.previous_modules.bind(py),))?;
    if let Some(main) = &stage.previous_main {
        modules.set_item("__main__", main)?;
    }
    match &stage.previous_finder {
        Some(finder) => install(py, finder.bind(py))?,
        None => remove_finders(py)?,
    }
    Ok(())
}

/// Makes the staged generation the one that serves.
pub(crate) fn activate() {
    LOADER.with(|loader| {
        let mut loader = loader.borrow_mut();
        if let Some(staged) = loader.staged.take() {
            loader.serving = Some(staged);
        }
    });
}

/// Throws the staged generation away: its modules leave, the serving generation's come back.
///
/// # Errors
///
/// A failure of the loader itself.
pub(crate) fn abandon(py: Python<'_>) -> PyResult<()> {
    let staged = LOADER.with(|loader| loader.borrow_mut().staged.take());
    match staged {
        Some(stage) => undo(py, &stage),
        None => Ok(()),
    }
}

/// Ends the shell: no module of the script stays, `__main__` is as it was before the first generation.
///
/// # Errors
///
/// A failure of the loader itself.
pub(crate) fn end(py: Python<'_>) -> PyResult<()> {
    abandon(py)?;
    let modules = sys_modules(py)?;
    for (name, _) in user_modules(py)? {
        modules.del_item(name).ok();
    }
    remove_finders(py)?;
    let (serving, original) = LOADER.with(|loader| {
        let mut loader = loader.borrow_mut();
        let serving = loader.serving.take().is_some();
        (serving, loader.original_main.as_ref().map(|main| main.clone_ref(py)))
    });
    if let (true, Some(original)) = (serving, original) {
        modules.set_item("__main__", original)?;
    }
    Ok(())
}
