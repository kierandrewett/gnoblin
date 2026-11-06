//! Generate `gnoblinctl`'s command tree, in `OUT_DIR`.
//!
//! The tree is generated from `crates/gnoblin-schema/schema/*.toml` on every
//! build, so a schema member cannot exist without a subcommand and a subcommand
//! cannot exist without a schema member (design/rust-compositor.md, "One IPC":
//! "Generated from the schema: ... `gnoblinctl` subcommands and completion").
//!
//! The generator itself is `src/generate.rs`, included here so `build.rs` and the
//! crate's tests run exactly the same code. `OUT_DIR/cli_tree.rs` is data;
//! `src/tree.rs` includes it and gives it behaviour.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "src/generate.rs"]
mod generate;

/// The schema directory, relative to this crate.
const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[gnoblinctl] {error}");
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

    watch(&schema_dir)?;

    let catalog = gnoblin_schema::Catalog::load_dir(&schema_dir)
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;
    let rendered = generate::render(&catalog, env!("CARGO_PKG_VERSION")).map_err(|problems| {
        format!(
            "the schema cannot be rendered as a command tree:\n  {}",
            problems.join("\n  ")
        )
    })?;

    let path = out_dir.join("cli_tree.rs");
    fs::write(&path, rendered).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(())
}

/// Tell Cargo to rerun when anything in `dir` changes.
///
/// A directory alone is not enough: cargo compares the timestamps it saw, and
/// editing a file that already exists does not move its parent's. Missing a schema
/// edit would leave the previous build's command tree, which is the drift this
/// generator exists to prevent.
fn watch(dir: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
    Ok(())
}
