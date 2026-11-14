//! The JavaScript shell runtime: TypeScript stripping, the module loader, and one QuickJS generation.
//!
//! The design is `design/js-runtime.md`. This crate is the engine core of the JS shell: it runs a user's
//! `init.ts` in a sandboxed QuickJS context, bounded in time and memory, and reports every failure at the
//! line the author wrote.
//!
//! - [`strip_types`] turns a TypeScript module into JavaScript and a source map.
//! - [`JsResolver`] and [`JsLoader`] load modules from a config root and from host-embedded sources.
//! - [`JsGeneration`] is one runtime and context with a wall-clock budget per call and a memory limit.
//! - [`JsHost`] stages a new generation, keeps the current one on failure, and commits on success.
//! - [`ConfigWatcher`] turns editor save bursts into one [`ReloadRequest`]; [`JsReloader`] stages and
//!   commits on a request and reports failures at the author's `.ts` line; [`JsSupervisor`] counts
//!   interrupts and panics and decides between restaging and giving up.
//! - [`stock_modules`] is the TypeScript stock library (`data/js/gnoblin/`) as embedded `gnoblin` modules.
//! - [`JsShell`] runs the shell over the compositor's tree: `gnoblin.clients` and the rest of the object
//!   model in `js/gnoblin.ts` (the `gnoblin/host` module), over the host functions in `bindings`, with the
//!   frame loop of `ShellVm`. A workspace is a script object: [`JsWorkspaceSnapshot`] carries which window was on
//!   which one from a shell to the next that reloads.
//! - [`JsRole`] is the runtime role's shell: it connects a [`JsShell`] to the owner socket, loads the user's
//!   `init.ts`, and owns the watcher, reloader and supervisor that swap a new generation in or keep the old one.

mod bindings;
mod error;
mod generation;
mod host;
mod input;
mod loader;
mod reload;
mod role;
mod shell;
mod stock;
mod supervisor;
mod ts;
mod ui;
mod watch;
mod workspaces;

pub use bindings::{JsSpawnError, JsSpawnRequest, JsSpawner};
pub use error::{JsError, JsErrorKind, JsLoadError, JsLocation, JsTsError};
pub use generation::{JsConfig, JsGeneration};
pub use host::{JsHost, JsPrepare};
pub use loader::{JsLoadState, JsLoader, JsResolver};
pub use reload::{JsDiagnostic, JsReloader, ReloadReport};
pub use role::{JsRole, JsRoleConfig, RELOAD_DEBOUNCE, config_dir, entry_in};
pub use rquickjs;
pub use shell::{JsFrame, JsShell, JsShellConfig, JsShellError, JsStaging, STALE_RETRIES};
pub use stock::{stock_modules, stock_modules_in};
pub use supervisor::{JsClock, JsFault, JsFaultAction, JsHealth, JsStrikePolicy, JsSupervisor, JsSystemClock};
pub use ts::{JsStripped, strip_types};
pub use watch::{ConfigWatcher, JsWatchError, ReloadRequest};
pub use workspaces::{JsWorkspaceEntry, JsWorkspaceSnapshot};
