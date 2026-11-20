//! Hides every export of the library but its entry symbol, with a linker version script (`exports.map`), and embeds the generated
//! half of the `gnoblin` package.
//!
//! PyO3 exports `PyInit__gnoblin_core` and `PyInit__gnoblin_std`, the initialisers of the built-in modules the runtime registers with the interpreter's
//! inittab. The interpreter reaches it by address, so the name need not be exported, and a runtime that exports a second
//! name could answer a `dlsym` it was never asked for. rustc passes its own version script that lists the symbols as global; the one
//! here names them as local, which wins. `cargo xtask dist-check` and `cargo xtask stage` refuse a runtime that
//! exports anything but `gnoruntime_entry_v1`.
//!
//! `gnoblin-sdkgen` renders the typed node classes, the ABI constants, the verbs and the std bindings (`gnoblin/_generated/*.py` and
//! `gnoblin/std.py`) into `OUT_DIR` here, and `python_embedded.rs` lists them for `src/embedded.rs`, which serves them to the
//! interpreter. Nothing of them is committed. The `.pyi` stubs are for editors and go into a prefix with `cargo xtask dist`.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets the manifest directory");
    let map = std::path::Path::new(&manifest).join("exports.map");
    println!("cargo:rerun-if-changed={}", map.display());
    println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script={}", map.display());
    embed_generated();
}

/// Renders the Python target into `OUT_DIR/python/` and writes `OUT_DIR/python_embedded.rs`: `(dotted module name, is a package,
/// source)` for every generated `.py` file, the source included from where it was written.
fn embed_generated() {
    let paths = gnoblin_sdkgen::emit_for_build(&["python"]);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut table = String::from(
        "/// The generated modules of the `gnoblin` package: name, whether it is a package, and its source.\n",
    );
    table.push_str("pub(crate) static MODULES: &[(&str, bool, &str)] = &[\n");
    for path in &paths {
        let Some(file) = path.strip_prefix("python/").and_then(|file| file.strip_suffix(".py")) else {
            continue;
        };
        let (module, package) = match file.strip_suffix("/__init__") {
            Some(package) => (package.replace('/', "."), true),
            None => (file.replace('/', "."), false),
        };
        let source = out.join(path);
        let _ = writeln!(
            table,
            "    ({module:?}, {package}, include_str!({:?})),",
            source.display().to_string()
        );
    }
    table.push_str("];\n");
    fs::write(out.join("python_embedded.rs"), table).expect("OUT_DIR is writable");
}
