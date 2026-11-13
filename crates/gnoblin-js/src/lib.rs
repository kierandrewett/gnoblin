//! The JavaScript shell runtime: TypeScript stripping, the module loader, and one QuickJS generation.
//!
//! The design is `design/js-runtime.md`. This crate is the engine core only: it runs a user's `init.ts`
//! in a sandboxed QuickJS context, bounded in time and memory, and reports every failure at the line
//! the author wrote. It has no objects of its own and does not talk to the compositor.
//!
//! - [`strip_types`] turns a TypeScript module into JavaScript and a source map.
//! - [`JsResolver`] and [`JsLoader`] load modules from a config root and from host-embedded sources.
//! - [`JsGeneration`] is one runtime and context with a wall-clock budget per call and a memory limit.
//! - [`JsHost`] stages a new generation, keeps the current one on failure, and commits on success.

mod error;
mod generation;
mod host;
mod loader;
mod ts;

pub use error::{JsError, JsErrorKind, JsLoadError, JsLocation, JsTsError};
pub use generation::{JsConfig, JsGeneration};
pub use host::JsHost;
pub use loader::{JsLoadState, JsLoader, JsResolver};
pub use rquickjs;
pub use ts::{JsStripped, strip_types};
