//! Native objects a host library adds to every generation's global scope.
//!
//! The engine installs `__gnoblinHost` itself. A library that runs the engine (`libgnoblin-runtime-js`) has natives of its own
//! to give a script: `__gnoblinStd`, the whole table of `libgnoblin-std`. It hands the engine a [`JsNative`] in
//! [`JsConfig::natives`](crate::JsConfig::natives), and the engine calls it for each generation it builds, the way it installs the
//! host functions, with what a native needs of the engine: how a node's id becomes the number a script holds ([`ScriptIds`]).

use std::sync::Arc;

use rquickjs::Ctx;

use crate::bindings::SharedState;

/// How node ids cross into script numbers, and back.
///
/// A script number holds an integer exactly up to 2^53, and the compositor's own nodes carry ids with bit 63 set, so an id cannot
/// be the number a script sees. An id below the handle band is its own number; any other gets the next handle in the band the first
/// time it crosses and keeps it for the life of the shell, so a reloaded generation sees the same number for the same window.
#[derive(Clone)]
pub struct ScriptIds {
    state: SharedState,
}

impl ScriptIds {
    pub(crate) fn new(state: &SharedState) -> ScriptIds {
        ScriptIds { state: state.clone() }
    }

    /// The first number that is a handle and not an id: a script number at or above it must go through [`ScriptIds::from_script`].
    pub const HANDLE_BASE: f64 = ((1_u64 << 53) - (1 << 24)) as f64;

    /// The number a script holds for the node `id`.
    #[must_use]
    pub fn to_script(&self, id: u64) -> f64 {
        self.state.borrow().handles.to_script(id)
    }

    /// The id a script number names, or `None` for a handle that was never given out.
    #[must_use]
    pub fn from_script(&self, number: f64) -> Option<u64> {
        self.state.borrow().handles.resolve(number)
    }
}

/// A function that installs a native object in a generation's global scope.
#[derive(Clone)]
pub struct JsNative(Arc<NativeInstall>);

type NativeInstall = dyn for<'js> Fn(&Ctx<'js>, &ScriptIds) -> rquickjs::Result<()> + Send + Sync;

impl JsNative {
    /// A native that `install` puts in each generation's context.
    pub fn new(
        install: impl for<'js> Fn(&Ctx<'js>, &ScriptIds) -> rquickjs::Result<()> + Send + Sync + 'static,
    ) -> JsNative {
        JsNative(Arc::new(install))
    }

    pub(crate) fn install<'js>(&self, ctx: &Ctx<'js>, ids: &ScriptIds) -> rquickjs::Result<()> {
        (self.0)(ctx, ids)
    }
}

impl std::fmt::Debug for JsNative {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JsNative")
    }
}
