//! The runtime's pieces for a host that runs the engine in its own process, over a core it holds itself.
//!
//! The runtime library is loaded by the core's loader and handed the core's table (`start`). The tests of the engine have no loader:
//! they link the core and the std library into the test binary, make a generation of their own, and drive a [`gnoblin_js::JsShell`]
//! directly. What they need of the runtime is what makes that shell the one `start` makes: the compositor's link as the core relays
//! it (the engine's mirror a view of the core's), `__gnoblinStd` over the std library, and the core's dispatch between frames, with
//! the library's callbacks entering the shell. This module is those, and nothing else of the runtime: no loop, no reload, no
//! configuration document.

use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};
use gnoblin_js::rquickjs::{self, Ctx};
use gnoblin_js::{JsNative, JsShell};
use gnoblin_std_dyn::StdLib;
use gnoblin_tree_client::tree_mirror::TreeLink;

use crate::link::{RelayedLink, Serving};
use crate::std_native::{self, Enter};
use crate::table::{self, CoreTable};

/// A core this process holds and the generation a shell runs as.
pub struct Embedded {
    table: CoreTable,
    ctx: GnoblinCtx,
    serving: Serving,
}

impl Embedded {
    /// The runtime over the core behind `api`, for the generation `ctx` of this thread's shell instance.
    pub fn new(api: &'static GnoblinApi, ctx: GnoblinCtx) -> Self {
        let table = CoreTable::new(api);
        table::remember(table);
        table::set_log_context(ctx);
        std_native::set_context(ctx);
        Self {
            table,
            ctx,
            serving: std::rc::Rc::new(std::cell::Cell::new(ctx)),
        }
    }

    /// Hands the runtime the std library, which this process holds.
    pub fn install_std(&self, library: StdLib) {
        std_native::install_library(library);
    }

    /// The compositor's link, as the engine drives it: a view of the core's tree (`tree_link_open`).
    ///
    /// # Errors
    ///
    /// The core could not open the link.
    pub fn link(&self) -> Result<Box<dyn TreeLink>, String> {
        Ok(Box::new(RelayedLink::open(self.table, self.serving.clone())?))
    }

    /// `__gnoblinStd`, for the config of each generation the shell evaluates.
    pub fn std_native(&self) -> JsNative {
        std_native::native()
    }

    /// `__gnoblinObjects`, the object protocol of the libraries, for the config of each generation the shell evaluates.
    pub fn object_native(&self) -> JsNative {
        crate::object_native::native()
    }

    /// Serves the namespace `namespace` of a library this process holds (a test's own catalog) to the scripts, ahead of the core and the
    /// std library: `__gnoblinObjects.open(namespace)` finds `table`.
    pub fn serve_namespace(
        &self,
        namespace: &str,
        table: &'static gnoblin_core_abi::GnoblinObjectApi,
        last_error: fn() -> String,
    ) {
        crate::object_native::serve(namespace, table, last_error);
    }

    /// Runs `body` with `shell` as the way a callback of a library enters the running generation: what the host does around the turn of
    /// a library it holds itself.
    pub fn enter<R>(&self, shell: &JsShell, body: impl FnOnce() -> R) -> R {
        std_native::with_enter(&ShellEnter(shell), body)
    }

    /// Runs what the core queued for the generation: what the library announces and what a layout is asked, with the library's
    /// callbacks entering `shell` as they do in the runtime's turn. Returns the number of callbacks run.
    pub fn dispatch(&self, shell: &JsShell) -> i32 {
        std_native::with_enter(&ShellEnter(shell), || self.table.dispatch(self.ctx))
    }

    /// Scripts the engine evaluates from now belong to the generation `ctx`, as the runtime's `start` sets for a reload before the
    /// engine evaluates the new script: what they register is the staged generation's until it takes over.
    pub fn belong_to(&self, ctx: GnoblinCtx) {
        std_native::set_context(ctx);
        table::set_log_context(ctx);
    }

    /// The generation `ctx` serves from now: the engine's link and [`dispatch`](Self::dispatch) speak for it, as the runtime moves
    /// to the new generation once a reload took.
    pub fn serve(&mut self, ctx: GnoblinCtx) {
        self.serving.set(ctx);
        self.ctx = ctx;
    }
}

/// How the library's callbacks enter a shell: its own entry, under the listener budget.
struct ShellEnter<'a>(&'a JsShell);

impl Enter for ShellEnter<'_> {
    fn enter(&self, body: &mut dyn for<'js> FnMut(&Ctx<'js>) -> rquickjs::Result<()>) -> Result<(), String> {
        self.0.call_script(|ctx| body(ctx)).map_err(|error| error.to_string())
    }
}
