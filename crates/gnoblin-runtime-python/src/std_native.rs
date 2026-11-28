//! The installed `libgnoblin-std`, opened for the object classes (`gnoblin.objects.std`).
//!
//! The library is opened the first time a script asks, never when the runtime starts: a shell that does not use std runs on a machine
//! that does not have it. The classes themselves are generated from the library's catalog and reach it through the object protocol
//! (`object_native`), so this module only finds and keeps the library.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_std_dyn::StdLib;
use pyo3::exceptions::PyImportError;
use pyo3::prelude::*;

use crate::errors::to_py;
use crate::generation::{self, with_state};

thread_local! {
    /// The library, once opened: it belongs to the shell thread like every handle of a generation.
    static LIBRARY: RefCell<Option<Rc<StdLib>>> = const { RefCell::new(None) };
}

/// The library, opened now if it was not.
pub(crate) fn library() -> PyResult<Rc<StdLib>> {
    if let Some(library) = LIBRARY.with(|slot| slot.borrow().clone()) {
        return Ok(library);
    }
    // Any address inside the core says where the core is: the table's own functions live there.
    let anchor = with_state(|state| state.api.log as *const std::ffi::c_void)
        .ok_or_else(|| to_py(None, generation::WRONG_THREAD))?;
    let library = Rc::new(StdLib::load(anchor).map_err(PyImportError::new_err)?);
    LIBRARY.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&library)));
    Ok(library)
}
