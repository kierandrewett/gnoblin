//! `gnoblin.model` through the object protocol: the catalog of `crates/gnoblin-core-abi/schema` and how it is served.
//!
//! `gnoblin-object` does the protocol; this module gives it the catalog (written by `gnoblin-sdkgen` at build time) and the hooks that
//! tie it to the registry: a generation's end retires its objects, and the turn that tells listeners is a post to the shell thread.
//! The classes of the model are added by the lanes that own its modules, each a catalog file and an implementation here.
#![allow(unsafe_code)]

use core::ffi::c_void;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinObjectApi};
use gnoblin_object::server::{Builder, Hooks, Library, Provider, api_table};
use gnoblin_object::{Fail, Res};

use crate::guard::set_code_error;
use crate::registry::{self, Registering};

#[allow(dead_code, unused_imports, clippy::all)]
mod catalog {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/model_catalog.rs"));
}

use catalog::{CATALOG, CATALOG_HASH};

thread_local! {
    /// The model's objects: one per thread, and a shell has one thread.
    static LIB: Library = build();
}

fn build() -> Library {
    let mut builder = Builder::new(&CATALOG);
    crate::model::register(&mut builder);
    builder.finish(HOOKS)
}

/// Runs `work` on the library of the model.
pub(crate) fn with_lib<R>(work: impl FnOnce(&Library) -> R) -> R {
    LIB.with(work)
}

/// Runs `work` as a call about `obj` of the generation `ctx`, between the calls of the shell: the model telling its objects what the
/// compositor sent.
pub(crate) fn run_in<R>(
    ctx: GnoblinCtx,
    obj: gnoblin_object::Obj,
    work: impl FnOnce(&mut gnoblin_object::server::Cx<'_>) -> R,
) -> Res<R> {
    LIB.with(|lib| lib.run_in(ctx, obj, work))
}

struct Model;

impl Provider for Model {
    fn with<R>(work: impl FnOnce(&Library) -> R) -> R {
        LIB.with(work)
    }
}

/// The object protocol of `gnoblin.model`, behind `GnoblinApi::object_api`.
pub(crate) static API: GnoblinObjectApi = api_table::<Model>(CATALOG_HASH);

const HOOKS: Hooks = Hooks {
    first_use,
    schedule,
    set_last,
    log,
    foreign,
};

/// The core's classes extend no other library's.
fn foreign(_ctx: GnoblinCtx, namespace: &str) -> Res<gnoblin_object::client::Api> {
    Err(Fail::no_such(format!(
        "libgnoblin-core extends no class of `{namespace}`"
    )))
}

/// The context travels as the `user` of the hooks the registry calls back.
fn user_of(ctx: GnoblinCtx) -> *mut c_void {
    ctx as usize as *mut c_void
}

unsafe extern "C" fn retire(user: *mut c_void) {
    let ctx = user as usize as GnoblinCtx;
    // What the bindings began and did not finish is ended while the shell's code is mapped and its objects are still held.
    let _ = std::panic::catch_unwind(|| crate::model::end_inflight(ctx));
    LIB.with(|lib| lib.retire(ctx));
    crate::model::forget(ctx);
}

unsafe extern "C" fn flush(user: *mut c_void) {
    let ctx = user as usize as GnoblinCtx;
    LIB.with(|lib| lib.flush(ctx));
}

/// A generation made its first object: its end takes every object, listener and closure with it.
fn first_use(ctx: GnoblinCtx) -> Res<()> {
    let reg = registry::register(
        ctx,
        "on_shutdown",
        Registering::Shutdown { cb: retire },
        user_of(ctx),
        None,
    );
    if reg == 0 {
        return Err(Fail::new(abi::E_RETIRED, "the generation cannot hold objects any more"));
    }
    Ok(())
}

/// Posts the turn that tells the listeners what the library queued.
fn schedule(ctx: GnoblinCtx) {
    let _ = registry::post(ctx, flush, user_of(ctx), None);
}

fn set_last(fail: &Fail) {
    set_code_error(fail.code, fail.message.clone());
}

fn log(ctx: GnoblinCtx, level: u32, message: &str) {
    let _ = registry::log_line(ctx, level, message);
}

/// The table of `namespace`: `gnoblin.model` is the one this library serves.
pub(crate) fn api_of(namespace: &str) -> Result<&'static GnoblinObjectApi, String> {
    if namespace == CATALOG.namespace {
        Ok(&API)
    } else {
        Err(format!(
            "libgnoblin-core serves `{}`, not `{namespace}`",
            CATALOG.namespace
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_served_and_complete() {
        let problems = LIB.with(|lib| lib.problems());
        assert!(problems.is_empty(), "{problems:#?}");
        assert!(api_of("gnoblin.model").is_ok());
        assert!(api_of("gnoblin.std").is_err());
        assert_eq!(API.catalog_hash, CATALOG_HASH);
    }
}
