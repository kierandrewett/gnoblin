//! The typed classes of `gnoblin.std`.
//!
//! The classes are written by `gnoblin-sdkgen` from the catalog (`crates/gnoblin-std-abi/schema`) into `OUT_DIR` at build time, over the
//! client of the object protocol (`gnoblin-object`). [`Std::objects`] is the one way in: it asks the installed library for the table
//! that serves the namespace and gives the handle every class is made from. A class added to the catalog is a class here with no
//! change to this file.
//!
//! The members the library adds to the core's typed classes (the `[[augment]]` blocks of its catalog) are the extension traits
//! `ModelRootStdExt`, for the root's `workspaces`, `displays`, `scene`, `gestures` and `request_transition`, `OutputCollectionStdExt`,
//! for `profiles`, `InputContextStdExt`, for `active_display`, and `ClientCollectionStdExt`, for `style`. `gnoblin_std::prelude` brings
//! them in; a shell that does not use it has none of them, and a call of one does not compile:
//!
//! ```compile_fail,E0599
//! use gnoblin_core::objects::ModelRoot;
//!
//! fn read(model: &ModelRoot) {
//!     // `ModelRootStdExt` is not in scope: the core's root has no `workspaces`.
//!     let _ = model.workspaces();
//! }
//! ```
#![allow(unsafe_code)]

use std::cell::RefCell;

use gnoblin_core::ErrorKind;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinObjectApi};
use gnoblin_object::client::Api;

use crate::api::{Std, last_error_text};
use crate::error::{Error, Result};

#[allow(dead_code, missing_docs, clippy::all)]
mod generated_api {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/std_api.rs"));
}

pub use generated_api::*;

thread_local! {
    /// The handle `Std::objects` made last on this thread, with the generation it is for: a shell opens the classes once.
    static OPENED: RefCell<Option<(GnoblinCtx, Objects)>> = const { RefCell::new(None) };
}

impl Std {
    /// The typed classes of `gnoblin.std` for this generation. Asked again, it gives the handle it made before, so the ids of the
    /// classes are looked up once.
    ///
    /// # Errors
    ///
    /// The installed library does not serve the namespace.
    pub fn objects(&self) -> Result<Objects> {
        let ctx = self.ctx();
        if let Some(objects) = OPENED.with(|opened| {
            opened
                .borrow()
                .as_ref()
                .filter(|(for_ctx, _)| *for_ctx == ctx)
                .map(|(_, objects)| objects.clone())
        }) {
            return Ok(objects);
        }
        let mut raw: *const GnoblinObjectApi = core::ptr::null();
        // SAFETY: `raw` is a live local the library writes, and the name is borrowed for the call.
        let code = unsafe { (self.api().object_api)(ctx, abi::GnoblinStr::borrowed("gnoblin.std"), &raw mut raw) };
        self.check(code)?;
        // SAFETY: the table the library returns is static.
        let raw =
            unsafe { raw.as_ref() }.ok_or_else(|| Error::new(ErrorKind::Abi, "libgnoblin-std gave no object table"))?;
        let objects = Objects::new(Api::new(raw, ctx, last_error_text));
        OPENED.with(|opened| *opened.borrow_mut() = Some((ctx, objects.clone())));
        Ok(objects)
    }
}

/// The typed classes of `gnoblin.std` for the generation the core object table `core` belongs to: what the members this library adds to a
/// core class (`ModelRootStdExt`) use, since a core object holds the core's table and not this library's. A shell of a generation that
/// has retired gets the error the retired generation gives, and never the library of a newer one.
///
/// # Errors
///
/// The installed std library is not usable by this SDK, or the generation has retired.
pub(crate) fn std_objects(core: gnoblin_object::client::Api) -> gnoblin_object::Res<Objects> {
    let fail = |error: Error| gnoblin_object::Fail::err(error.to_string());
    let g = gnoblin_core::__private::gnoblin_for(core.ctx()).map_err(|error| fail(error.into()))?;
    Std::open(&g).map_err(fail)?.objects().map_err(fail)
}
