//! Generate the WIT host API and the member table from the object schema.
//!
//! Nothing this crate compiles from the schema is checked in: `cargo build` runs
//! this script, which reads `crates/gnoblin-schema/schema/*.toml`, renders one WIT
//! package and the Rust table of event members, and writes them where they are
//! used (design/rust-compositor.md, D6). A clean checkout therefore needs no
//! generator step first, and a schema edit is a build rather than a regeneration
//! somebody has to remember. The generated WIT text is gitignored, exactly like
//! the wire types `gnoblin-proto`'s build script writes: generated output in the
//! tree is a copy that goes stale.
//!
//! Two copies of the WIT are written, and the reason is a limitation of
//! `wasmtime::component::bindgen`: its `path` option is a string literal resolved
//! against `CARGO_MANIFEST_DIR`, so the macro cannot read `OUT_DIR`. `OUT_DIR`
//! holds the canonical file (the tests read it, so "the generated WIT parses" is a
//! statement about the bytes the host was compiled against) and
//! `generated/gnoblin-ipc.wit` holds the same bytes for the macro, which `bindgen`
//! also registers with `include_bytes!` so a change to the WIT recompiles this
//! crate.
//!
//! The renderer is `src/wit.rs`, included here rather than compiled into the
//! library: the host speaks the generated API, not the schema, so nothing at
//! runtime needs the generator, and a copy of it in the library would be code
//! nothing calls.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gnoblin_schema::Catalog;

#[path = "src/wit.rs"]
mod wit;

/// The schema directory, relative to this crate.
const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

/// The tree-relative directory `bindgen` reads the package from.
const GENERATED_DIR: &str = "generated";

/// The file name both copies are written under.
const WIT_FILE: &str = "gnoblin-ipc.wit";

/// Schema files that describe wire traffic between services and the compositor, not
/// a module API. Matched on the file name the schema was loaded from.
const WIRE_ONLY_SCHEMAS: &[&str] = &["portal_ui.toml"];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[wasm] {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let schema_dir = manifest.join(SCHEMA_DIR);
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "OUT_DIR is not set".to_owned())?;

    // The WIT is generated from the schema, so the schema is the input; the
    // renderer is the other, and cargo does not track it through `#[path]`.
    watch(&schema_dir)?;
    println!("cargo:rerun-if-changed=src/wit.rs");

    let loaded = Catalog::load_dir(&schema_dir)
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;
    // The portal schema is the services-to-compositor wire (D19, `portal-call`), not an
    // API a module may call. Every object in the WIT is hand-implemented by the host, and
    // the portal has no module-facing implementation, so it is left out of the module surface.
    let schemas: Vec<_> = loaded
        .schemas()
        .iter()
        .filter(|schema| !WIRE_ONLY_SCHEMAS.iter().any(|name| schema.origin.ends_with(name)))
        .cloned()
        .collect();
    let catalog = Catalog::new(schemas).map_err(|error| error.to_string())?;
    // The WIT package carries the crate's version verbatim, prerelease and all: a
    // candidate build generates `gnoblin:ipc@0.2.0-alpha.N`, so a module built
    // against a release candidate cannot be linked into a release compositor by
    // accident. The renderer checks the text is a version WIT accepts.
    let rendered = wit::render(&catalog, env!("CARGO_PKG_VERSION")).map_err(|error| error.to_string())?;

    write(&out_dir.join(WIT_FILE), &rendered.wit)?;
    write(&out_dir.join("members.rs"), &rendered.members)?;
    let generated = manifest.join(GENERATED_DIR);
    fs::create_dir_all(&generated).map_err(|error| format!("cannot create {}: {error}", generated.display()))?;
    write(&generated.join(WIT_FILE), &rendered.wit)?;
    Ok(())
}

/// Write a file, and only touch it when the contents changed.
///
/// The generated WIT is an input to `bindgen`, whose output is part of this
/// crate's compilation: rewriting an identical file on every build would bump its
/// timestamp and recompile the crate for nothing.
fn write(path: &Path, contents: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path)
        && existing == contents
    {
        return Ok(());
    }
    fs::write(path, contents).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Ask cargo to rerun this script when the directory or any file in it changes.
fn watch(dir: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
    Ok(())
}
