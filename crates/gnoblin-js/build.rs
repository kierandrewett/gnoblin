//! Records which engine and type-stripper versions this build links, for the bytecode cache's directory name.
//!
//! The stock library is not embedded: it is installed data (`share/gnoblin/js`), read at run time (`src/stock.rs`).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-env=GNOBLIN_JS_TOOLCHAIN={}", toolchain(&manifest));
    ExitCode::SUCCESS
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
