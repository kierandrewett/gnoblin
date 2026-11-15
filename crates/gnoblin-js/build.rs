//! Embed the stock library: every `.ts` file under `data/js/gnoblin`, as the table `stock.rs` strips and hands to the
//! loader.
//!
//! The files are listed from the directory, not named in `stock.rs`, so a new module reaches the binary without an edit
//! to a hand-kept array (whose length and entries used to be the thing a lane collided on). The import name is the path
//! under the directory without its extension, prefixed `gnoblin/`; `index.ts` is the user-facing `gnoblin` itself.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const STOCK_DIR: &str = "../../data/js/gnoblin";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[js] {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let stock_dir = manifest.join(STOCK_DIR);
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "OUT_DIR is not set".to_owned())?;

    // A file appearing or disappearing changes the directory and not the files already inside it, so the directories
    // are watched as well as the files.
    let mut files = Vec::new();
    collect(&stock_dir, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Err(format!("no .ts files in {}", stock_dir.display()));
    }

    let mut text = String::from("/// `(import name, file under data/js/gnoblin, source)`, in path order.\n");
    text.push_str("const MODULES: &[(&str, &str, &str)] = &[\n");
    for path in &files {
        let file = path
            .strip_prefix(&stock_dir)
            .ok()
            .and_then(Path::to_str)
            .ok_or_else(|| format!("{} is not a UTF-8 stock path", path.display()))?;
        let full = path
            .to_str()
            .ok_or_else(|| format!("{} is not a UTF-8 path", path.display()))?;
        let _ = writeln!(text, "    ({:?}, {file:?}, include_str!({full:?})),", import_name(file));
    }
    text.push_str("];\n");
    fs::write(out_dir.join("stock_modules.rs"), text).map_err(|error| format!("cannot write stock_modules.rs: {error}"))
}

/// The name a script imports `file` (a path under the stock directory, with `/` separators) by.
fn import_name(file: &str) -> String {
    let stem = file.strip_suffix(".ts").unwrap_or(file);
    if stem == "index" {
        "gnoblin".to_owned()
    } else {
        format!("gnoblin/{stem}")
    }
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
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("ts") {
            files.push(path);
        }
    }
    Ok(())
}
