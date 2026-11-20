//! The typed classes of `gnoblin.std`.
//!
//! The classes are written by `gnoblin-sdkgen` from the catalog (`crates/gnoblin-std-abi/schema`) into `OUT_DIR` at build time, over the
//! client of the object protocol (`gnoblin-object`). [`Std::objects`] is the one way in: it asks the installed library for the table
//! that serves the namespace and gives the handle every class is made from. A class added to the catalog is a class here with no
//! change to this file.
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
