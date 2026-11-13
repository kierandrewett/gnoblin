//! The JavaScript shell runtime: TypeScript stripping, the module loader, and one QuickJS generation.
//!
//! The design is `design/js-runtime.md`. This crate is the engine core only: it runs a user's `init.ts`
//! in a QuickJS context, bounded in time and memory, and reports every failure at the line the author
//! wrote. It has no objects of its own and does not talk to the compositor.
//!
//! - [`strip_types`] turns a TypeScript module into JavaScript and a source map.

mod error;
mod ts;

pub use error::{JsError, JsErrorKind, JsLoadError, JsLocation, JsTsError};
pub use ts::{JsStripped, strip_types};
