//! `gnoblin.d.ts` and `tsconfig.json` for the JavaScript and TypeScript shell.
//!
//! The declarations come from the same schema catalog as the runtime, so an editor and the runtime cannot disagree about a
//! member: [`render_dts`] writes one interface per schema object, with the schema's value
//! kinds as TypeScript types, followed by the proposed stock library
//! (`static/gnoblin-stock.d.ts`), which is hand-written.
//!
//! ```no_run
//! let written = gnoblin_js_types::ensure_user_files(std::path::Path::new("/home/me/.config/gnoblin"))?;
//! println!("types at {}", written.types.display());
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! The crate depends on the schema and nothing else of the workspace: it names no engine,
//! and it does not run or parse TypeScript. The checked-in `tests/golden/gnoblin.d.ts` is the
//! rendered stock schema; regenerate it with
//! `UPDATE_GOLDEN=1 cargo test -p gnoblin-js-types --test golden`.

mod render;
mod stock;
mod user_files;

pub use render::render_dts;
pub use stock::stock_catalog;
pub use user_files::{JsWritten, TSCONFIG_FILE, TYPES_FILE, ensure_user_files, render_tsconfig};
