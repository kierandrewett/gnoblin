//! The editor files of the JavaScript and TypeScript shell: the real library as types, the schema declarations, and the
//! `tsconfig.json` that ties them together.
//!
//! The types of `import ... from "gnoblin-core"`, `"gnoblin-std"` and their submodules are the library's own
//! TypeScript sources, written out as files that the generated `tsconfig.json` maps those names to, so go-to-definition
//! opens the file the shell runs and a class extends the class it really extends. This crate does not embed those
//! sources (the shell does, in `gnoblin-js`); the caller hands them to [`ensure_user_files`]. What the crate generates
//! is the schema half: [`render_dts`] writes one interface per schema object, with the schema's value kinds as
//! TypeScript types, followed by the two globals a script has without an import. The declarations come from the same
//! schema catalog as the runtime, so an editor and the runtime cannot disagree about a member.
//!
//! ```no_run
//! let library = vec![("index.ts".to_owned(), "export {};\n".to_owned())];
//! let written = gnoblin_js_types::ensure_user_files(std::path::Path::new("/home/me/.config/gnoblin"), &library)?;
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
pub use user_files::{
    GENERATED_DIR, GENERATED_TSCONFIG_FILE, JsWritten, LIBRARY_DIR, TSCONFIG_FILE, TYPES_FILE, TsconfigOutcome,
    VERSION_FILE, ensure_user_files, render_generated_tsconfig, render_user_tsconfig,
};
