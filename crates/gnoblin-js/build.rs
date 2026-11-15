//! Embed the stock library: every `.ts` file under `data/js/gnoblin`, as the table `stock.rs` strips and hands to the
//! loader.
//!
//! The files are listed from the directory, not named in `stock.rs`, so a new module reaches the binary without an edit
//! to a hand-kept array (whose length and entries used to be the thing a lane collided on). The import name is the path
//! under the directory without its extension, prefixed `gnoblin/`; `index.ts` is the user-facing `gnoblin` itself, and
//! the `index.ts` of a subdirectory is that directory's import name (`extensions/xr/index.ts` is
//! `gnoblin/extensions/xr`), the same rule TypeScript applies when it resolves a directory.

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
    fs::write(out_dir.join("stock_modules.rs"), text)
        .map_err(|error| format!("cannot write stock_modules.rs: {error}"))?;
    println!("cargo:rustc-env=GNOBLIN_JS_TOOLCHAIN={}", toolchain(&manifest));
    Ok(())
}

/// The crates whose versions decide what bytecode a module compiles to: the engine, and the oxc crates that strip the
/// types before it sees the text. `src/bytecode.rs` keys its cache directory on them.
const TOOLCHAIN_CRATES: &[&str] = &[
    "rquickjs-sys",
    "oxc_allocator",
    "oxc_codegen",
    "oxc_parser",
    "oxc_semantic",
    "oxc_sourcemap",
    "oxc_span",
    "oxc_transformer",
];

/// `name=version;` for each of [`TOOLCHAIN_CRATES`], from the workspace's `Cargo.lock`, or the empty string when the
/// lock file cannot be read or does not list one of them, which turns the bytecode cache off: a build that cannot name
/// its toolchain must not share a directory with one that can.
fn toolchain(manifest: &Path) -> String {
    let lock = manifest.join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let Ok(text) = fs::read_to_string(&lock) else {
        return String::new();
    };
    let mut found: Vec<(String, String)> = Vec::new();
    let mut name: Option<&str> = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"'));
        } else if let (Some(value), Some(current)) = (line.strip_prefix("version = "), name) {
            if TOOLCHAIN_CRATES.contains(&current) {
                found.push((current.to_owned(), value.trim_matches('"').to_owned()));
            }
            name = None;
        }
    }
    found.sort();
    found.dedup();
    if !TOOLCHAIN_CRATES
        .iter()
        .all(|wanted| found.iter().any(|(name, _)| name == wanted))
    {
        return String::new();
    }
    found.iter().fold(String::new(), |mut out, (name, version)| {
        let _ = write!(out, "{name}={version};");
        out
    })
}

/// The name a script imports `file` (a path under the stock directory, with `/` separators) by.
fn import_name(file: &str) -> String {
    let stem = file.strip_suffix(".ts").unwrap_or(file);
    if stem == "index" {
        "gnoblin".to_owned()
    } else if let Some(directory) = stem.strip_suffix("/index") {
        format!("gnoblin/{directory}")
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
