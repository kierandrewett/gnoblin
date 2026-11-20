//! Records which engine and type-stripper versions this build links, for the bytecode cache's directory name, and embeds the
//! generated half of the stock library.
//!
//! The hand-written stock library is not embedded: it is installed data (`share/gnoblin/js`), read at run time (`src/stock.rs`).
//! What `gnoblin-sdkgen` generates from the schema and the ABI crates (`gnoblin-core/generated/*` and `gnoblin-std/generated/*`)
//! is nobody's source and is committed nowhere: this script renders it into `OUT_DIR` and writes `stock_embedded.rs`, the table
//! `src/stock.rs` serves those modules from, so they cannot be older or newer than the libraries the runtime is built with.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("cargo:rustc-env=GNOBLIN_JS_TOOLCHAIN={}", toolchain(&manifest));
    embed_generated();
    ExitCode::SUCCESS
}

/// Renders the TypeScript target and the object classes (`object-js`: the JavaScript the engine runs; `object-types`: the declarations
/// an editor reads) into `OUT_DIR/js/` and writes `OUT_DIR/stock_embedded.rs`: `(path under the library root, source)` for every
/// generated module, the source included from where it was written.
fn embed_generated() {
    let paths = gnoblin_sdkgen::emit_for_build(&["typescript", "object-js", "object-types"]);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut table = String::from("/// The generated modules of the stock library, by path under the library root.\n");
    table.push_str("pub(crate) static GENERATED: &[(&str, &str)] = &[\n");
    for path in &paths {
        let Some(file) = path.strip_prefix("js/") else {
            continue;
        };
        let source = out.join(path);
        let _ = writeln!(
            table,
            "    ({file:?}, include_str!({:?})),",
            source.display().to_string()
        );
    }
    table.push_str("];\n");
    fs::write(out.join("stock_embedded.rs"), table).expect("OUT_DIR is writable");
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
