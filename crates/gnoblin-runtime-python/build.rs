//! Hides every export of the library but its entry symbol, with a linker version script (`exports.map`).
//!
//! PyO3 exports `PyInit__gnoblin_core` and `PyInit__gnoblin_std`, the initialisers of the built-in modules the runtime registers with the interpreter's
//! inittab. The interpreter reaches it by address, so the name need not be exported, and a runtime that exports a second
//! name could answer a `dlsym` it was never asked for. rustc passes its own version script that lists the symbols as global; the one
//! here names them as local, which wins. `cargo xtask dist-check` and `cargo xtask stage` refuse a runtime that
//! exports anything but `gnoruntime_entry_v1`.

fn main() {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets the manifest directory");
    let map = std::path::Path::new(&manifest).join("exports.map");
    println!("cargo:rerun-if-changed={}", map.display());
    println!("cargo:rustc-cdylib-link-arg=-Wl,--version-script={}", map.display());
}
