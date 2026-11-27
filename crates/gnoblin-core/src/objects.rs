//! The typed object model of the core, `gnoblin.model`.
//!
//! The classes are written by `gnoblin-sdkgen` from the catalog (`crates/gnoblin-core-abi/schema`) into `OUT_DIR` at build time, over the
//! client of the object protocol (`gnoblin-object`). [`Gnoblin::objects`] is the one way in: it asks the installed core for the table
//! that serves the namespace and gives the handle every class is made from. Nothing of the model is a function of the core's own
//! table, so a class added to the catalog is a class here with no change to this file.
#![allow(unsafe_code)]

use std::cell::RefCell;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinObjectApi};
use gnoblin_object::client::Api;

use crate::error::{CoreError, ErrorKind, Result};
use crate::gnoblin::Gnoblin;

/// The failure a closure of a typed class answers with (`Result<_, Fail>`). `?` on a call of the core converts into it.
pub use gnoblin_object::Fail;

#[allow(dead_code, missing_docs, unused_variables, clippy::all)]
mod generated_api {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/model_api.rs"));
}

pub use generated_api::*;

thread_local! {
    /// The handle `Gnoblin::objects` made last on this thread, with the generation it is for: a shell opens the model once.
    static OPENED: RefCell<Option<(GnoblinCtx, Objects)>> = const { RefCell::new(None) };
}

impl From<Fail> for CoreError {
    fn from(fail: Fail) -> CoreError {
        CoreError::new(ErrorKind::from_code(fail.code), fail.message)
    }
}

/// What a handler of a typed class answers the core's errors as: `?` on a call of the core (`g.objects()?`) inside a closure that
/// returns `Result<_, Fail>` ends the handler with the core's failure, its kind as the code and its text as the message.
impl From<CoreError> for Fail {
    fn from(error: CoreError) -> Fail {
        Fail::new(error.kind().code(), error.to_string())
    }
}

impl Gnoblin {
    /// The typed object model of `gnoblin.model` for this generation. Asked again, it gives the handle it made before, so the ids of
    /// the classes are looked up once.
    ///
    /// # Errors
    ///
    /// The installed core does not serve the namespace.
    pub fn objects(&self) -> Result<Objects> {
        let ctx = self.ctx();
        if let Some(objects) = OPENED.with(|opened| {
            opened
                .borrow()
                .as_ref()
                .filter(|(for_ctx, _)| *for_ctx == ctx)
                .map(|(_, o)| o.clone())
        }) {
            return Ok(objects);
        }
        let mut raw: *const GnoblinObjectApi = core::ptr::null();
        // SAFETY: `raw` is a live local the core writes, and the name is borrowed for the call.
        let code = unsafe { (self.api().object_api)(ctx, abi::GnoblinStr::borrowed("gnoblin.model"), &raw mut raw) };
        crate::error::check(code)?;
        // SAFETY: the table the core returns is static.
        let raw =
            unsafe { raw.as_ref() }.ok_or_else(|| CoreError::new(ErrorKind::Abi, "the core gave no object table"))?;
        let objects = Objects::new(Api::new(raw, ctx, crate::private::last_error));
        OPENED.with(|opened| *opened.borrow_mut() = Some((ctx, objects.clone())));
        Ok(objects)
    }
}

/// The typed model of the generation `ctx`: what another library's typed API reads a `Client` (or any class of the model) it hands out
/// with, since that value names the core's table and not its own. It is [`Gnoblin::objects`] of that generation.
///
/// # Errors
///
/// The installed core is not usable by this SDK, or the generation has retired.
#[doc(hidden)]
pub fn for_generation(ctx: GnoblinCtx) -> gnoblin_object::Res<Objects> {
    crate::private::gnoblin_for(ctx)
        .and_then(|g| g.objects())
        .map_err(Fail::from)
}
