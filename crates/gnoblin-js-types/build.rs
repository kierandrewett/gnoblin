//! Embed the checked-in schema files so the declarations can be generated at run time,
//! by a process that has no schema directory beside it (`gnoblinctl`, the runtime).
//!
//! The files are listed from the directory, not named here, so a new schema file reaches
//! the generator without an edit to this crate.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[js-types] {error}");
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

    // A file appearing or disappearing changes the directory and not the files already
    // inside it, so the directories are watched as well as the files.
    let mut files = Vec::new();
    collect(&schema_dir, &mut files)?;
    // `Catalog::load_dir` reads in path order, and the catalog keeps that order, so the
    // embedded list has to match it or the same schema would render differently.
    files.sort();
    if files.is_empty() {
        return Err(format!("no .toml schema files in {}", schema_dir.display()));
    }

    let mut text = String::from("pub const STOCK_FILES: &[(&str, &str)] = &[\n");
    for path in &files {
        let name = path
            .strip_prefix(&schema_dir)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| format!("{} is not a UTF-8 schema path", path.display()))?;
        let full = path
            .to_str()
            .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))?;
        let _ = writeln!(text, "    ({name:?}, include_str!({full:?})),");
    }
    text.push_str("];\n");
    fs::write(out_dir.join("stock_files.rs"), text).map_err(|error| format!("cannot write stock_files.rs: {error}"))
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("cannot read {}: {error}", dir.display()))?
            .path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
            files.push(path);
        }
    }
    Ok(())
}
