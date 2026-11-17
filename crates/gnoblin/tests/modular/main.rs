//! The examples of `design/modular-framework.md` section 12 that are scripts, run for real: `gnoblin ./shell.ts`, `./shell.js` and
//! `./shell.py` in a headless session, with a Wayland client that stays connected (one cargo test target, `modular`).
//!
//! Nothing here is linked into the test. The session is the shipped binary, which links `libgnoblin-core.so.1` and serves the
//! compositor role from it. The shell is a script the core's loader runs through the language runtime its extension names
//! (`libgnoblin-runtime-js.so`, `libgnoblin-runtime-python.so`, built into the harness's own target directory and found through
//! `GNOBLIN_RUNTIME_DIR`); the stock library and the Python package are read from the repository's `data/` trees, which stand for
//! an installed prefix's `share/gnoblin/`. What each test asserts is what the compositor's tree holds (read through observer
//! connections), what the process maps, and what it logs, so it is what a user of the session would see and not what the shell
//! believes.
//!
//! The binary finds the core through its run path (`$ORIGIN`), so the core must be staged beside it: `cargo xtask stage`, or a
//! workspace build.

#[path = "../support/mod.rs"]
mod support;

mod document;
mod js_esm;
mod no_std;
mod script_binding;
mod script_reload;
mod ts_types;

use std::fs;
use std::path::{Path, PathBuf};

use support::native_shell::{Gnoblin, screen_size};

/// The repository's root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

/// A copy of the example `name` (a directory of `examples/`) in a directory of its own, so that a test that edits the shell, or a
/// session that watches its directory, never touches the repository.
pub fn copy_example(name: &str) -> tempfile::TempDir {
    let source = repo_root().join("examples").join(name);
    let dir = tempfile::tempdir().expect("a directory for the shell");
    for entry in fs::read_dir(&source).expect("the example's directory") {
        let path = entry.expect("an entry").path();
        if path.is_file() {
            fs::copy(&path, dir.path().join(path.file_name().expect("a file name"))).expect("the example's file");
        }
    }
    dir
}

/// Waits until the compositor serves and the core's loader has the script running (the line it logs when a shell serves in its
/// process).
#[track_caller]
pub fn serving(session: &mut Gnoblin) {
    session.wait_serving();
    session.wait_log("the script shell serves in process", |line| {
        line.contains("serves in process")
    });
}

/// Whether two numbers of a layout are the same to half a pixel.
pub fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.5
}

/// The size of the session's first screen.
pub fn screen_of(session: &Gnoblin) -> (f64, f64) {
    screen_size(&session.ipc())
}

/// Saves `text` as the file `path` the way an editor that saves atomically does: written under another name and renamed over the
/// file, so a reload reads the old file or the whole new one and never one that a write has emptied and not yet filled.
pub fn save(path: &Path, text: &str) {
    let staged = path.with_extension("part");
    fs::write(&staged, text).expect("the staged file");
    fs::rename(&staged, path).expect("the file replaced");
}

/// The files the process maps whose path says they are the runtime's private copy or the core: for the assertions about what a
/// script shell maps.
pub fn mapped(session: &Gnoblin) -> Vec<String> {
    session.mapped_files()
}

/// Whether `path` names a language runtime's copy (the loader opens a runtime from a unique copy of its library).
pub fn is_runtime_copy(path: &str) -> bool {
    path.contains("/gen-") && path.contains("runtime")
}
