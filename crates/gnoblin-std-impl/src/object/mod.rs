//! `gnoblin.std` through the object protocol: the classes of the catalog (`crates/gnoblin-std-abi/schema`) and how they are served.
//!
//! `gnoblin-object` does the protocol; this module gives it the catalog (written by `gnoblin-sdkgen` at build time), the hooks that tie
//! it to the core (arrange for the end of a generation, post the turn that tells listeners, record an error) and the classes. A class
//! is added by a catalog file and an implementation here, and needs no function in `GnoblinStdApi`.
#![allow(unsafe_code)]

pub(crate) mod entry_model;
mod focus;
pub(crate) mod record;

use gnoblin_core::LogLevel;
use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinObjectApi};
use gnoblin_object::server::{Builder, Hooks, Library, Provider, api_table};
use gnoblin_object::{Fail, Res};

#[allow(dead_code, clippy::all)]
mod catalog {
    include!(concat!(env!("OUT_DIR"), "/rust-objects/std_catalog.rs"));
}

use catalog::{CATALOG, CATALOG_HASH};

thread_local! {
    /// The library's objects: one per thread, and a shell has one thread.
    static LIB: Library = build();
}

struct Std;

impl Provider for Std {
    fn with<R>(work: impl FnOnce(&Library) -> R) -> R {
        LIB.with(work)
    }
}

/// Runs `work` with the library that serves `gnoblin.std` on this thread: for the toolkit's turn, which acts on objects from outside
/// a call.
pub(crate) fn with_lib<R>(work: impl FnOnce(&Library) -> R) -> R {
    LIB.with(work)
}

/// The object protocol of `gnoblin.std`, behind `GnoblinStdApi::object_api`.
pub(crate) static API: GnoblinObjectApi = api_table::<Std>(CATALOG_HASH);

fn build() -> Library {
    let mut builder = Builder::new(&CATALOG);
    entry_model::register(&mut builder);
    focus::register(&mut builder);
    crate::ui::register(&mut builder);
    crate::scene::register(&mut builder);
    crate::style::objects::register(&mut builder);
    crate::animation::objects::register(&mut builder);
    builder.finish(HOOKS)
}

const HOOKS: Hooks = Hooks {
    first_use,
    schedule,
    set_last,
    log,
    foreign,
};

/// The object protocol of the core, for a class of `gnoblin.std` whose base class is one of `gnoblin.model`: the library is a guest of
/// the core and reaches it through the table the shell's generation holds.
fn foreign(ctx: GnoblinCtx, namespace: &str) -> Res<gnoblin_object::client::Api> {
    let g = gnoblin_core::private::gnoblin_for(ctx).map_err(core_fail)?;
    let table = gnoblin_core::private::api_of(&g);
    let mut raw: *const GnoblinObjectApi = core::ptr::null();
    // SAFETY: `raw` is a live local the core writes, and `namespace` is borrowed for the call.
    let code = unsafe { (table.object_api)(ctx, abi::GnoblinStr::borrowed(namespace), &raw mut raw) };
    if code != abi::E_OK {
        return Err(Fail::new(code, gnoblin_core::private::last_error()));
    }
    // SAFETY: the table the core returns is static.
    let raw = unsafe { raw.as_ref() }.ok_or_else(|| Fail::err("the core gave no object table"))?;
    Ok(gnoblin_object::client::Api::new(
        raw,
        ctx,
        gnoblin_core::private::last_error,
    ))
}

/// The object protocol of the core (`gnoblin.model`), for a class that reads the object model as a binding's function does.
pub(crate) fn core_api(ctx: GnoblinCtx) -> Res<gnoblin_object::client::Api> {
    foreign(ctx, "gnoblin.model")
}

fn core_fail(error: gnoblin_core::CoreError) -> Fail {
    Fail::new(error.kind().code(), error.to_string())
}

/// A generation made its first object: its end takes every object, listener and closure with it.
fn first_use(ctx: GnoblinCtx) -> Res<()> {
    let g = gnoblin_core::private::gnoblin_for(ctx).map_err(core_fail)?;
    g.on_shutdown(move || LIB.with(|lib| lib.retire(ctx)))
        .map(drop)
        .map_err(core_fail)
}

/// Posts the turn that tells the listeners what the library queued.
fn schedule(ctx: GnoblinCtx) {
    let Ok(g) = gnoblin_core::private::gnoblin_for(ctx) else {
        return;
    };
    let _ = g.poster().post(move |_g| LIB.with(|lib| lib.flush(ctx)));
}

fn set_last(fail: &Fail) {
    crate::error::set_last(format!("{}: {}", crate::error::code_name(fail.code), fail.message));
}

fn log(ctx: GnoblinCtx, level: u32, message: &str) {
    if let Ok(g) = gnoblin_core::private::gnoblin_for(ctx) {
        let level = if level <= abi::LOG_ERROR {
            LogLevel::CoreError
        } else {
            LogLevel::Warn
        };
        g.log_at(level, &format!("gnoblin-std: {message}"));
    }
}

/// `object_api(ctx, namespace, out)`: the table of `gnoblin.std`.
pub(crate) unsafe extern "C" fn object_api(
    _ctx: GnoblinCtx,
    namespace: abi::GnoblinStr,
    out: *mut *const GnoblinObjectApi,
) -> i32 {
    crate::table::call("object_api", || {
        // SAFETY: the table's contract passes `namespace` as readable text.
        let namespace = unsafe { crate::table::owned(&namespace) }?;
        if namespace != CATALOG.namespace {
            return Err(crate::error::Fail::no_such(format!(
                "libgnoblin-std serves `{}`, not `{namespace}`",
                CATALOG.namespace
            )));
        }
        // SAFETY: the table's contract passes `out` as writable.
        let out =
            unsafe { out.as_mut() }.ok_or_else(|| crate::error::Fail::err("object_api: a null output pointer"))?;
        *out = &raw const API;
        Ok(abi::E_OK)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two-way completeness: every class and member of the catalog has an implementation, and every implementation names a member of
    /// the catalog.
    #[test]
    fn the_catalog_and_the_implementation_cover_each_other() {
        let problems = LIB.with(Library::problems);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn the_table_carries_the_catalog_hash() {
        assert_eq!(API.catalog_hash, CATALOG_HASH);
        assert_ne!(CATALOG_HASH, 0);
    }
}
