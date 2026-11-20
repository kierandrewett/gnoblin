//! `gnoblin.api`: the `gnoblin` object, what a shell script talks to.
//!
//! Everything here is a thin spelling of the built-in module `_gnoblin_core` (itself a layer over the installed `libgnoblin-core`) and
//! adds no compositor logic of its own. The typed classes of the tree are generated.
//!
//! Registering. A *named subscription* is an attribute of `gnoblin` that registers a function:
//!
//! ```python
//! @gnoblin.on_client_created              # bare: the function is the callback, and it is returned unchanged
//! def place(window): ...
//!
//! @gnoblin.bind("<Super>Return")           # parameters first: the call returns the decorator
//! def terminal(): gnoblin.spawn("foot")
//!
//! sub = gnoblin.bind("<Super>d", launcher)       # parameters and the function: returns the Subscription
//! sub = gnoblin.on_client_created.subscribe(place)   # a bare event's Subscription
//! ```
//!
//! A registration belongs to the generation of the script that made it. A reload removes all of them and runs the script again, so a
//! script never undoes its own listeners; `Subscription.unsubscribe()` and `gnoblin.off(function)` are for changing one's mind while the
//! shell runs. Callbacks run on the shell thread. An exception in one is logged with its traceback and the next event is delivered.

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyCFunction, PyDict, PyList, PyString, PyTuple};

use crate::descriptors::of;
use crate::subscription::{Subscription, forget, remember};
use crate::{events, input, module, process, tree, values};

const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

/// The named subscriptions, with how many parameters each takes before the function.
#[derive(Clone, Copy)]
enum Kind {
    ClientCreated,
    ClientDestroyed,
    TitleChanged,
    AppIdChanged,
    FocusChanged,
    WindowPropertyChanged,
    NodeCreated,
    NodeDestroyed,
    ClassPropertyChanged,
    PropertyChanged,
    WindowRequest,
    NodeEvent,
    Signal,
    Shutdown,
    Bind,
    BindEvents,
    BindPointer,
    BindGesture,
    ClaimPointerButtons,
}

/// `(attribute, parameters before the function, how it registers)`.
const EVENTS: [(&str, usize, Kind); 19] = [
    ("on_client_created", 0, Kind::ClientCreated),
    ("on_client_destroyed", 0, Kind::ClientDestroyed),
    ("on_title_changed", 0, Kind::TitleChanged),
    ("on_app_id_changed", 0, Kind::AppIdChanged),
    ("on_focus_changed", 0, Kind::FocusChanged),
    ("on_window_property_changed", 1, Kind::WindowPropertyChanged),
    ("on_node_created", 1, Kind::NodeCreated),
    ("on_node_destroyed", 1, Kind::NodeDestroyed),
    ("on_class_property_changed", 2, Kind::ClassPropertyChanged),
    ("on_property_changed", 2, Kind::PropertyChanged),
    ("on_window_request", 0, Kind::WindowRequest),
    ("on_node_event", 1, Kind::NodeEvent),
    ("on_signal", 2, Kind::Signal),
    ("on_shutdown", 0, Kind::Shutdown),
    ("bind", 1, Kind::Bind),
    ("bind_events", 1, Kind::BindEvents),
    ("bind_pointer", 1, Kind::BindPointer),
    ("bind_gesture", 2, Kind::BindGesture),
    ("claim_pointer_buttons", 1, Kind::ClaimPointerButtons),
];

/// A Python callable made from a closure over the arguments the core passes.
fn closure<'py>(
    py: Python<'py>,
    body: impl Fn(&Bound<'_, PyTuple>) -> PyResult<Py<PyAny>> + Send + Sync + 'static,
) -> PyResult<Bound<'py, PyAny>> {
    Ok(PyCFunction::new_closure(
        py,
        None,
        None,
        move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| body(args),
    )?
    .into_any())
}

/// `function(class(*fields))`: the callback the core calls with plain fields, as the script's function wants them.
fn with_record<'py>(
    py: Python<'py>,
    function: &Bound<'py, PyAny>,
    class: Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let (function, class) = (function.clone().unbind(), class.unbind());
    closure(py, move |args| {
        let py = args.py();
        let record = class.bind(py).call1(args.clone())?;
        Ok(function.bind(py).call1((record,))?.unbind())
    })
}

/// The id of a node a script names as a parameter.
fn node_id(node: &Bound<'_, PyAny>) -> PyResult<u64> {
    node.getattr("node_id")?.extract()
}

/// One named subscription: `nparams` parameters (a class name, a chord) and then the function.
///
/// `event(fn)` for an event without parameters, and `event(*params)(fn)`, register and return the function. `event(*params, fn)` and
/// `subscribe` register and return the `Subscription`.
#[pyclass(frozen, name = "Event", module = "gnoblin.api")]
pub(crate) struct Event {
    name: &'static str,
    nparams: usize,
    kind: Kind,
}

impl Event {
    /// Registers `args[-1]` with the parameters before it.
    fn register(&self, py: Python<'_>, args: &Bound<'_, PyTuple>) -> PyResult<u64> {
        let function = args.get_item(self.nparams)?;
        let param = |index: usize| args.get_item(index);
        let text = |index: usize| -> PyResult<String> { param(index)?.extract() };
        match self.kind {
            Kind::ClientCreated => events::on_node_created("window", &function),
            Kind::ClientDestroyed => events::on_node_destroyed("window", &function),
            Kind::TitleChanged => events::on_class_property_changed("window", "title", &function),
            Kind::AppIdChanged => events::on_class_property_changed("window", "app_id", &function),
            Kind::FocusChanged => {
                let function = function.unbind();
                let hook = closure(py, move |args| {
                    let py = args.py();
                    Ok(function.bind(py).call1((args.get_item(1)?,))?.unbind())
                })?;
                events::on_class_property_changed("seat", "focus", &hook)
            }
            Kind::WindowPropertyChanged => events::on_class_property_changed("window", &text(0)?, &function),
            Kind::NodeCreated => events::on_node_created(&text(0)?, &function),
            Kind::NodeDestroyed => events::on_node_destroyed(&text(0)?, &function),
            Kind::ClassPropertyChanged => events::on_class_property_changed(&text(0)?, &text(1)?, &function),
            Kind::PropertyChanged => events::on_property_changed(node_id(&param(0)?)?, &text(1)?, &function),
            Kind::WindowRequest => {
                let function = function.unbind();
                let class = py.get_type::<values::WindowRequest>().into_any().unbind();
                let hook = closure(py, move |args| {
                    let py = args.py();
                    let request = class
                        .bind(py)
                        .call1((args.get_item(1)?, args.get_item(2)?, args.get_item(3)?))?;
                    Ok(function.bind(py).call1((args.get_item(0)?, request))?.unbind())
                })?;
                events::on_window_request(&hook)
            }
            Kind::NodeEvent => {
                let hook = with_record(py, &function, py.get_type::<values::NodeEvent>().into_any())?;
                events::on_node_event(node_id(&param(0)?)?, &hook)
            }
            Kind::Signal => events::on_signal(&text(0)?, &text(1)?, &function),
            Kind::Shutdown => events::on_shutdown(&function),
            Kind::Bind => input::bind(&text(0)?, &function),
            Kind::BindEvents => {
                let hook = with_record(py, &function, py.get_type::<values::ChordEvent>().into_any())?;
                input::bind_events(&text(0)?, &hook)
            }
            Kind::BindPointer => {
                let hook = with_record(py, &function, py.get_type::<values::PointerDrag>().into_any())?;
                input::bind_pointer(&text(0)?, &hook)
            }
            Kind::BindGesture => {
                let hook = with_record(py, &function, py.get_type::<values::GestureEvent>().into_any())?;
                input::bind_gesture(&text(0)?, param(1)?.extract()?, &hook)
            }
            Kind::ClaimPointerButtons => {
                let hook = with_record(py, &function, py.get_type::<values::PointerButton>().into_any())?;
                input::claim_pointer_buttons(&text(0)?, &hook)
            }
        }
    }

    /// Registers `args[-1]` with the parameters before it; returns the `Subscription` and notes it under the function.
    fn subscribe_all<'py>(&self, py: Python<'py>, args: &Bound<'py, PyTuple>) -> PyResult<Bound<'py, Subscription>> {
        let function = args.get_item(args.len().saturating_sub(1)).ok();
        let Some(function) = function.filter(|function| args.len() == self.nparams + 1 && function.is_callable())
        else {
            return Err(PyTypeError::new_err(format!(
                "{} takes {} parameter(s) and then a function",
                self.name, self.nparams
            )));
        };
        let subscription = Bound::new(py, Subscription::new(self.register(py, args)?))?;
        remember(py, &function, &subscription)?;
        Ok(subscription)
    }
}

#[pymethods]
impl Event {
    /// Registers `args[-1]` with the parameters before it; returns the `Subscription`.
    #[pyo3(signature = (*args))]
    fn subscribe<'py>(&self, py: Python<'py>, args: &Bound<'py, PyTuple>) -> PyResult<Bound<'py, Subscription>> {
        self.subscribe_all(py, args)
    }

    #[pyo3(signature = (*args))]
    fn __call__<'py>(
        slf: &Bound<'py, Self>,
        py: Python<'py>,
        args: &Bound<'py, PyTuple>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let this = slf.get();
        if this.nparams == 0 {
            let function = args
                .get_item(0)
                .ok()
                .filter(|function| args.len() == 1 && function.is_callable());
            let Some(function) = function else {
                return Err(PyTypeError::new_err(format!(
                    "{} is a decorator: use @gnoblin.{} on a function",
                    this.name, this.name
                )));
            };
            this.subscribe_all(py, args)?;
            return Ok(function);
        }
        if args.len() == this.nparams {
            let (event, params) = (slf.clone().unbind(), args.clone().unbind());
            let decorator = closure(py, move |call| {
                let py = call.py();
                let function = call.get_item(0)?;
                let mut all: Vec<Bound<'_, PyAny>> = params.bind(py).iter().collect();
                all.push(function.clone());
                let all = PyTuple::new(py, all)?;
                event.bind(py).get().subscribe_all(py, &all)?;
                Ok(function.unbind())
            })?;
            return Ok(decorator);
        }
        if args.len() == this.nparams + 1 {
            return Ok(this.subscribe_all(py, args)?.into_any());
        }
        Err(PyTypeError::new_err(format!(
            "{} takes {} parameter(s), optionally followed by the function",
            this.name, this.nparams
        )))
    }

    fn __repr__(&self) -> String {
        format!("<gnoblin.{}>", self.name)
    }
}

/// The shell's handle on the compositor. Use the instance `gnoblin`, not the class.
#[pyclass(dict, name = "Gnoblin", module = "gnoblin.api")]
pub(crate) struct Gnoblin;

#[pymethods]
impl Gnoblin {
    #[new]
    fn new() -> Self {
        Self
    }

    fn __init__(slf: &Bound<'_, Self>) -> PyResult<()> {
        let py = slf.py();
        for (name, nparams, kind) in EVENTS {
            slf.setattr(name, Bound::new(py, Event { name, nparams, kind })?)?;
        }
        Ok(())
    }

    // ---- the tree -------------------------------------------------------------------------------------------------

    /// The root of the tree.
    #[getter]
    fn root<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        of(py, tree::root()?, None)
    }

    /// The windows now in the tree, in stacking order (the first is the bottom).
    #[getter]
    fn windows<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
        tree::windows()?
            .into_iter()
            .map(|id| of(py, id, Some("window")))
            .collect()
    }

    /// The handle for the node with this id.
    fn node<'py>(&self, py: Python<'py>, id: u64) -> PyResult<Bound<'py, PyAny>> {
        of(py, id, None)
    }

    /// Makes a node of the class that is in no parent yet; put it in the tree with `node.insert_into(parent)`.
    fn create_node<'py>(&self, py: Python<'py>, class_name: &str) -> PyResult<Bound<'py, PyAny>> {
        of(py, tree::create_node(class_name)?, Some(class_name))
    }

    /// Finds the node of this class and name an earlier generation made, or makes it. Returns the node and whether it was found.
    #[pyo3(signature = (class_name, name, *, ephemeral = false))]
    fn adopt_node<'py>(
        &self,
        py: Python<'py>,
        class_name: &str,
        name: &str,
        ephemeral: bool,
    ) -> PyResult<(Bound<'py, PyAny>, bool)> {
        let (id, adopted) = tree::adopt_node(class_name, name, ephemeral)?;
        Ok((of(py, id, Some(class_name))?, adopted))
    }

    /// Calls `on_done(frame, error)` once the compositor has applied everything written so far.
    fn flush(&self, on_done: &Bound<'_, PyAny>) -> PyResult<()> {
        tree::flush(on_done)
    }

    // ---- timers, programs -----------------------------------------------------------------------------------------

    /// Calls `function()` once, `ms` milliseconds from now.
    fn after(&self, ms: u32, function: &Bound<'_, PyAny>) -> PyResult<Subscription> {
        Ok(Subscription::new(events::after(ms, function)?))
    }

    /// Calls `function()` every `ms` milliseconds.
    fn every(&self, ms: u32, function: &Bound<'_, PyAny>) -> PyResult<Subscription> {
        Ok(Subscription::new(events::every(ms, function)?))
    }

    /// Starts a program in a process group of its own. The name is looked up on `PATH`; it is not run through a shell.
    ///
    /// `stdout` and `stderr` are `inherit` (the session log), `discard` or `capture` (kept, up to `output_limit` bytes, for `on_exit`).
    /// `timeout` is in seconds. A program the script's top level starts waits until the generation takes over, so a reload that fails
    /// starts nothing, and one the next generation starts again is adopted, not doubled.
    #[pyo3(signature = (
        program, *args, cwd = None, env = None, stdin = None, stdout = "inherit", stderr = "inherit",
        output_limit = 0, timeout = None, on_exit = None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &self,
        py: Python<'_>,
        program: &Bound<'_, PyAny>,
        args: &Bound<'_, PyTuple>,
        cwd: Option<&Bound<'_, PyAny>>,
        env: Option<&Bound<'_, PyAny>>,
        stdin: Option<String>,
        stdout: &str,
        stderr: &str,
        output_limit: u64,
        timeout: Option<f64>,
        on_exit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<values::Process> {
        let mut argv: Vec<String> = if program.is_instance_of::<PyString>() {
            vec![program.extract()?]
        } else {
            program.extract()?
        };
        argv.extend(args.extract::<Vec<String>>()?);
        let hook = match on_exit {
            Some(function) => Some(with_record(
                py,
                function,
                py.get_type::<values::ProcessExit>().into_any(),
            )?),
            None => None,
        };
        let cwd: Option<String> = match cwd {
            Some(cwd) => Some(py.import("os")?.call_method1("fspath", (cwd,))?.extract()?),
            None => None,
        };
        let env: Option<Vec<(String, String)>> = match env {
            Some(env) => Some(
                py.get_type::<PyList>()
                    .call1((env.call_method0("items")?,))?
                    .extract()?,
            ),
            None => None,
        };
        let timeout_ms = timeout.map_or(0, |seconds| (seconds * 1000.0) as u32);
        let token = process::spawn(
            argv,
            cwd,
            env,
            stdin,
            stdout,
            stderr,
            output_limit,
            timeout_ms,
            hook.as_ref(),
        )?;
        Ok(values::Process::new(token))
    }

    // ---- input claims that return no events -----------------------------------------------------------------------

    /// Takes a device from the compositor's own input handling.
    fn claim_device(&self, device: &str) -> PyResult<Subscription> {
        Ok(Subscription::new(input::claim_device(device)?))
    }

    /// Sends the `pointer`, the `keyboard` or `both` to the node until the claim is removed.
    #[pyo3(signature = (node, scope = "both"))]
    fn grab_input(&self, node: &Bound<'_, PyAny>, scope: &str) -> PyResult<Subscription> {
        Ok(Subscription::new(input::grab_input(node_id(node)?, scope)?))
    }

    // ---- the shell ------------------------------------------------------------------------------------------------

    /// Removes every registration made for `function`; returns how many were removed. One already removed is skipped.
    fn off(&self, py: Python<'_>, function: &Bound<'_, PyAny>) -> PyResult<usize> {
        let mut removed = 0;
        for subscription in forget(py, function)? {
            if subscription.borrow().active() {
                match subscription.borrow_mut().unsubscribe() {
                    Ok(()) => removed += 1,
                    Err(error) if error.is_instance_of::<crate::errors::GnoblinError>(py) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(removed)
    }

    /// Writes a line to the compositor's log, from any thread.
    #[pyo3(signature = (message, level = "info"))]
    fn log(&self, message: &Bound<'_, PyAny>, level: &str) -> PyResult<()> {
        if !LEVELS.contains(&level) {
            return Err(PyValueError::new_err(format!(
                "a log level is one of {}, not {}",
                LEVELS.join(", "),
                level.into_pyobject(message.py())?.repr()?
            )));
        }
        module::log(level, &message.str()?.to_cow()?)
    }

    /// Runs `function()` on the shell thread at its next turn. The one call that is safe from any thread.
    fn post(&self, function: &Bound<'_, PyAny>) -> PyResult<()> {
        module::post(function)
    }

    /// Asks the host to reload this shell after the current callback returns.
    fn request_reload(&self) -> PyResult<()> {
        module::request_reload()
    }

    /// Asks the host to exit the compositor with `code`.
    #[pyo3(signature = (code = 0))]
    fn request_exit(&self, code: i32) -> PyResult<()> {
        module::request_exit(code)
    }

    /// Keeps `data` for the next generation of this shell (a reload). The store is dropped when the compositor restarts.
    #[pyo3(signature = (key, data, version = 0))]
    fn keep_state(&self, py: Python<'_>, key: &str, data: &Bound<'_, PyAny>, version: u32) -> PyResult<()> {
        let bytes = py.get_type::<PyBytes>().call1((data,))?;
        module::keep_state(key, bytes.cast::<PyBytes>()?.as_bytes(), version)
    }

    /// What an earlier generation kept under `key` as `(data, version)`, or `None`.
    fn kept_state<'py>(&self, py: Python<'py>, key: &str) -> PyResult<Option<(Bound<'py, PyBytes>, u32)>> {
        module::kept_state(py, key)
    }

    /// The generation: `generation`, `reload_count`, `module_path`, `state_dir`, `isolate`.
    #[getter]
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        module::info(py)
    }

    /// The core's counters: registrations, callbacks, panics, retired generations, stale calls.
    #[getter]
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        module::stats(py)
    }
}

/// Fills the module `gnoblin.api`.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add_class::<Event>()?;
    module.add_class::<Gnoblin>()?;
    // The one object a script uses: `from gnoblin import gnoblin`.
    module.add("gnoblin", py.get_type::<Gnoblin>().call0()?)?;
    Ok(())
}
