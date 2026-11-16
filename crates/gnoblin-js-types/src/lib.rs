//! The editor files of the JavaScript and TypeScript shell: the real library as types, the two globals, and the
//! `tsconfig.json` that ties them together.
//!
//! The types of `import ... from "gnoblin-core"`, `"gnoblin-std"` and their submodules are the library's own
//! TypeScript sources, written out as files that the generated `tsconfig.json` maps those names to, so go-to-definition
//! opens the file the shell runs and a class extends the class it really extends. This crate does not embed those
//! sources (the shell does, in `gnoblin-js`); the caller hands them to [`ensure_user_files`]. What the crate adds is
//! [`render_dts`], the two globals a script has without an import (`gnoblin`, whose type is the library's `Gnoblin`
//! class, and `console`), and the two `tsconfig.json` files.
//!
//! ```no_run
//! let library = vec![("index.ts".to_owned(), "export {};\n".to_owned())];
//! let written = gnoblin_js_types::ensure_user_files(std::path::Path::new("/home/me/.config/gnoblin"), &library)?;
//! println!("types at {}", written.types.display());
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! The crate depends on nothing of the workspace: it names no engine, and it does not run or parse TypeScript. The
//! checked-in `tests/golden/` files are what it writes; regenerate them with
//! `UPDATE_GOLDEN=1 cargo test -p gnoblin-js-types --test golden`.

mod render;
mod user_files;

pub use render::render_dts;
pub use user_files::{
    GENERATED_DIR, GENERATED_TSCONFIG_FILE, JsWritten, LIBRARY_DIR, TSCONFIG_FILE, TYPES_FILE, TsconfigOutcome,
    VERSION_FILE, ensure_user_files, render_generated_tsconfig, render_user_tsconfig,
};
