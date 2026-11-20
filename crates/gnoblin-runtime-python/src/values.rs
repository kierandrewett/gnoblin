//! The values an event carries, and the handles a program or a registration leaves with a script: the classes of `gnoblin.events`.
//!
//! Each value is a frozen record with the fields the core's callback passes, in that order, so `fn(WindowRequest(name, x, y))` is what the
//! runtime builds and a script can build the same to test its own functions. A record compares and hashes by its fields and prints as
//! `Name(field=value, ...)`, and names its fields in `__match_args__`, so `match event:` works the way it does on a dataclass.

use pyo3::IntoPyObjectExt;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::errors::core_error;

/// Writes a frozen record class: the struct, `__new__`, `__repr__`, `__eq__`, `__hash__` and `__match_args__`. `$extra` holds more
/// methods of the class.
macro_rules! record {
    (
        $(#[$meta:meta])*
        $rust:ident, $name:literal, signature = $signature:tt,
        { $( $(#[$fmeta:meta])* $field:ident : $ty:ty ),+ $(,)? }
        $( , extra { $($extra:item)* } )?
    ) => {
        $(#[$meta])*
        #[pyclass(frozen, name = $name, module = "gnoblin.events")]
        #[derive(Clone, PartialEq)]
        pub(crate) struct $rust {
            $( $(#[$fmeta])* #[pyo3(get)] $field: $ty, )+
        }

        #[pymethods]
        impl $rust {
            #[new]
            #[allow(clippy::too_many_arguments)]
            #[pyo3(signature = $signature)]
            fn new($($field: $ty),+) -> Self {
                Self { $($field),+ }
            }

            #[classattr]
            fn __match_args__(py: Python<'_>) -> PyResult<Bound<'_, PyTuple>> {
                PyTuple::new(py, [$(stringify!($field)),+])
            }

            fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
                let mut parts: Vec<String> = Vec::new();
                $( parts.push(format!(
                    concat!(stringify!($field), "={}"),
                    self.$field.clone().into_bound_py_any(py)?.repr()?
                )); )+
                Ok(format!("{}({})", $name, parts.join(", ")))
            }

            fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
                other.extract::<PyRef<'_, Self>>().is_ok_and(|other| *other == *self)
            }

            fn __hash__(&self, py: Python<'_>) -> PyResult<isize> {
                let items: Vec<Bound<'_, PyAny>> = vec![$( self.$field.clone().into_bound_py_any(py)? ),+];
                PyTuple::new(py, items)?.hash()
            }

            $($($extra)*)?
        }
    };
}

record! {
    /// A request a window's client made that policy decides. It changes nothing until the shell answers it.
    WindowRequest, "WindowRequest", signature = (name, x = 0.0, y = 0.0),
    {
        /// `move`, `resize`, `maximize`, `unmaximize`, `fullscreen`, `close`, `menu`, `activate`, ...
        name: String,
        /// For `menu`: where it opens.
        x: f64,
        y: f64,
    }
}

record! {
    /// An input event sent to a node: a key, a pointer press, motion, a wheel turn.
    NodeEvent, "NodeEvent", signature = (kind, x, y, button, key, text, dx, dy, pressed),
    {
        kind: String,
        x: f64,
        y: f64,
        button: i64,
        key: String,
        text: String,
        dx: f64,
        dy: f64,
        pressed: bool,
    }
}

record! {
    /// A key chord the shell claimed was pressed, repeated or released.
    ChordEvent, "ChordEvent", signature = (chord, state),
    {
        chord: String,
        /// `press`, `release` or `repeat`.
        state: String,
    }
}

record! {
    /// A pointer chord held and dragged.
    PointerDrag, "PointerDrag", signature = (phase, x, y, dx, dy, button, modifiers, time),
    {
        /// `start`, `update`, `end` or `cancel`.
        phase: String,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        button: i64,
        modifiers: String,
        time: u64,
    }
}

record! {
    /// A touchpad gesture.
    GestureEvent, "GestureEvent", signature = (kind, fingers, phase, dx, dy, scale, rotation),
    {
        kind: String,
        fingers: i64,
        phase: String,
        dx: f64,
        dy: f64,
        scale: f64,
        rotation: f64,
    }
}

record! {
    /// A button of a pointing device the shell claimed.
    PointerButton, "PointerButton", signature = (button, pressed, x, y, time),
    {
        button: i64,
        pressed: bool,
        x: f64,
        y: f64,
        time: u64,
    }
}

record! {
    /// How a program the shell started ended.
    ProcessExit, "ProcessExit",
    signature = (code, signal, timed_out, stdout, stderr, stdout_truncated = false, stderr_truncated = false),
    {
        code: Option<i64>,
        signal: Option<String>,
        timed_out: bool,
        stdout: String,
        stderr: String,
        stdout_truncated: bool,
        stderr_truncated: bool,
    },
    extra {
        /// Whether it exited normally with status 0.
        #[getter]
        fn success(&self) -> bool {
            self.code == Some(0)
        }
    }
}

/// A program the shell started, in a process group of its own.
#[pyclass(frozen, name = "Process", module = "gnoblin.events")]
pub(crate) struct ProcessHandle {
    token: u64,
}

impl ProcessHandle {
    pub(crate) const fn new(token: u64) -> Self {
        Self { token }
    }
}

#[pymethods]
impl ProcessHandle {
    #[new]
    fn py_new(token: u64) -> Self {
        Self { token }
    }

    /// Sends a signal (`term`, `kill`, `int`, `hup`, `usr1`, `usr2`, `stop`, `cont`) to the process group.
    #[pyo3(signature = (name = "term"))]
    fn signal(&self, name: &str) -> PyResult<()> {
        crate::generation::process(self.token)?
            .signal(crate::process::signal(name)?)
            .map_err(|error| core_error(&error))
    }

    /// Asks the program to end.
    fn terminate(&self) -> PyResult<()> {
        self.signal("term")
    }

    /// Ends the program at once.
    fn kill(&self) -> PyResult<()> {
        self.signal("kill")
    }

    fn __repr__(&self) -> String {
        format!("<ProcessHandle {}>", self.token)
    }
}
