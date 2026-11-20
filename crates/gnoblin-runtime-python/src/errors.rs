//! The exceptions `_gnoblin_core` raises: a failed call of the core is a Python exception, never a return code.
//!
//! `GnoblinError` is the base and the kinds of failure the core reports are its subclasses, so a script can catch what it expects
//! (`NodeGoneError` for a window that closed between a read and a write) and let the rest reach the log. A value of the wrong kind
//! is Python's `TypeError` and a deadline is `TimeoutError`, because that is what those mean in Python.

use gnoblin_core::{CoreError, ErrorKind};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyTimeoutError, PyTypeError};
use pyo3::prelude::*;

create_exception!(
    _gnoblin_core,
    GnoblinError,
    PyException,
    "A call of the Gnoblin core failed."
);
create_exception!(
    _gnoblin_core,
    RetiredError,
    GnoblinError,
    "The generation that made this call has been replaced by a reload."
);
create_exception!(
    _gnoblin_core,
    NodeGoneError,
    GnoblinError,
    "The node is not in the tree: its window closed, or it was destroyed."
);
create_exception!(
    _gnoblin_core,
    MemberError,
    GnoblinError,
    "The class has no such property, method or signal."
);
create_exception!(
    _gnoblin_core,
    RefusedError,
    GnoblinError,
    "The compositor refused the request."
);
create_exception!(
    _gnoblin_core,
    StaleError,
    GnoblinError,
    "The handle was removed already, or never existed."
);
create_exception!(
    _gnoblin_core,
    UnsupportedError,
    GnoblinError,
    "The installed core does not have this function."
);
create_exception!(
    _gnoblin_core,
    BusyError,
    GnoblinError,
    "A single-holder resource is taken."
);
create_exception!(
    _gnoblin_core,
    ThreadError,
    GnoblinError,
    "The call was made from a thread that is not the shell thread."
);

/// The exception for a failure of kind `kind` (`None` for one of the runtime's own) with `message`.
pub(crate) fn to_py(kind: Option<ErrorKind>, message: &str) -> PyErr {
    let text = message.to_owned();
    match kind {
        Some(ErrorKind::Retired) => RetiredError::new_err(text),
        Some(ErrorKind::NoSuchNode) => NodeGoneError::new_err(text),
        Some(ErrorKind::NoSuchMember) => MemberError::new_err(text),
        Some(ErrorKind::Refused) => RefusedError::new_err(text),
        Some(ErrorKind::Stale) => StaleError::new_err(text),
        Some(ErrorKind::Unsupported) => UnsupportedError::new_err(text),
        Some(ErrorKind::Busy) => BusyError::new_err(text),
        Some(ErrorKind::Thread) => ThreadError::new_err(text),
        Some(ErrorKind::Type) => PyTypeError::new_err(text),
        Some(ErrorKind::Timeout) => PyTimeoutError::new_err(text),
        Some(_) | None => GnoblinError::new_err(text),
    }
}

/// The exception for a failed call of the core.
pub(crate) fn core_error(error: &CoreError) -> PyErr {
    to_py(Some(error.kind()), &error.to_string())
}

/// Registers the exception classes in the module.
pub(crate) fn add_to(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("GnoblinError", py.get_type::<GnoblinError>())?;
    module.add("RetiredError", py.get_type::<RetiredError>())?;
    module.add("NodeGoneError", py.get_type::<NodeGoneError>())?;
    module.add("MemberError", py.get_type::<MemberError>())?;
    module.add("RefusedError", py.get_type::<RefusedError>())?;
    module.add("StaleError", py.get_type::<StaleError>())?;
    module.add("UnsupportedError", py.get_type::<UnsupportedError>())?;
    module.add("BusyError", py.get_type::<BusyError>())?;
    module.add("ThreadError", py.get_type::<ThreadError>())?;
    Ok(())
}

/// Fills the module `gnoblin.errors`: the same classes, so `except gnoblin.errors.NodeGoneError` catches what `_gnoblin_core` raises.
pub(crate) fn build(module: &Bound<'_, PyModule>) -> PyResult<()> {
    add_to(module)?;
    module.add(
        "__all__",
        [
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
    Ok(())
}
