//! The typed classes of `gnoblin.std`.
//!
//! The classes are written by `gnoblin-sdkgen` from the catalog (`crates/gnoblin-std-abi/schema`) into `OUT_DIR` at build time, over the
//! client of the object protocol (`gnoblin-object`). [`Std::objects`] is the one way in: it asks the installed library for the table
//! that serves the namespace and gives the handle every class is made from. A class added to the catalog is a class here with no
//! change to this file.
//!
//! The members the library adds to the core's typed classes (the `[[augment]]` blocks of its catalog) are the extension traits
//! `GnoblinStdExt`, for the root's `workspaces`, `displays`, `scene`, `gestures` and `request_transition`, `OutputCollectionStdExt`,
//! for `profiles`, `InputContextStdExt`, for `active_display`, `ClientCollectionStdExt`, for `style`, `ClientStdExt`, for a window's
//! `workspace`, `move_to_workspace`, `style`, `computed_style`, `style_warnings`, `move_with_keys` and `resize_with_keys`, and
//! `OutputStdExt`, for an output's `workspace` and `show`. `gnoblin_std::prelude` brings them in; a shell that does not use it has none
//! of them, and a call of one does not compile:
//!
//! ```compile_fail,E0599
//! use gnoblin_core::Gnoblin;
//!
//! fn read(model: &Gnoblin) {
//!     // `GnoblinStdExt` is not in scope: the core's root has no `workspaces`.
//!     let _ = model.workspaces();
//! }
//! ```
#![allow(unsafe_code)]

use std::cell::RefCell;

use gnoblin_core::host::ErrorKind;
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
/// core class (`GnoblinStdExt`) use, since a core object holds the core's table and not this library's. A shell of a generation that
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

/// The typed classes of `gnoblin.std` from the core's: a shell that has `g.objects()?` and makes classes of both libraries reaches the
/// second from the first, and keeps one binding for the two (`let core = g.objects()?; let std = core.std()?;`). `use
/// gnoblin_std::prelude::*` brings it in.
pub trait ObjectsStdExt {
    /// The typed classes of `gnoblin.std` for the generation these core classes are for: what [`Std::objects`] gives for it.
    ///
    /// # Errors
    ///
    /// The installed std library is not usable by this SDK, or the generation has retired.
    fn std(&self) -> gnoblin_object::Res<Objects>;
}

impl ObjectsStdExt for gnoblin_core::objects::Objects {
    fn std(&self) -> gnoblin_object::Res<Objects> {
        std_objects(self.api())
    }
}
