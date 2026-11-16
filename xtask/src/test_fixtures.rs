//! Compiled fixtures for the tests of `dist` and `dist-check`.
//!
//! The checks read real ELF files, so the tests build real ones with the C compiler: a core-like library with the right
//! SONAME and exports, shells that link it, a launcher, and deliberately wrong variants of each. A machine that links Rust
//! has a C compiler (rustc drives the link through `cc`), so the fixtures need nothing the build did not already.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gnoblin_core_abi as abi;

/// A fresh scratch directory for one test.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gnoblin-xtask-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Compiles `source` in `dir` with the C compiler and `args`.
pub fn cc(dir: &Path, source_name: &str, source: &str, args: &[&str]) {
    let path = dir.join(source_name);
    fs::write(&path, source).unwrap();
    let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    let output = Command::new(&compiler)
        .current_dir(dir)
        .arg(&path)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("the C compiler `{compiler}` is needed for these fixtures: {error}"));
    assert!(
        output.status.success(),
        "cc {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The C source of a library defining every export of the core (except `omit`), then `extra`. `gnoblin_abi_version` really
/// reports the ABI version, so a program that calls it can be checked end to end; the other exports are empty functions.
pub fn exports_source(extra: &str, omit: &str) -> String {
    let mut source = String::new();
    for symbol in abi::export_names().filter(|symbol| *symbol != omit) {
        if symbol == "gnoblin_abi_version" {
            source.push_str(&format!(
                "void gnoblin_abi_version(unsigned *major, unsigned *minor, unsigned *patch) {{ *major = {}; *minor = {}; *patch = {}; }}\n",
                abi::ABI_MAJOR,
                abi::ABI_MINOR,
                abi::ABI_PATCH
            ));
        } else {
            source.push_str(&format!("void {symbol}(void) {{}}\n"));
        }
    }
    source.push_str(extra);
    source
}

/// A core-like library in `dir`, named `libgnoblin-core.so` (the linker name): SONAME `libgnoblin-core.so.1`, `NODELETE`,
/// and exactly the core's exports.
pub fn good_core(dir: &Path) -> PathBuf {
    cc(
        dir,
        "core.c",
        &exports_source("", ""),
        &[
            "-shared",
            "-fPIC",
            "-Wl,-soname,libgnoblin-core.so.1",
            "-Wl,-z,nodelete",
            "-o",
            "libgnoblin-core.so",
        ],
    );
    dir.join("libgnoblin-core.so")
}

/// The source of a shell's one export. It references a core symbol, so a linker that drops unused libraries
/// (`--as-needed`, the default on Debian derivatives) still records the core as NEEDED.
pub const ENTRY: &str = "extern int gnoblin_get_api(unsigned, unsigned, const void **);\n\
                         const void *gnoshell_entry_v1(void) { const void *table = 0; gnoblin_get_api(1, 0, &table); return table; }\n";

/// Links a shell from `source` against the core in `dir`, plus `extra` linker arguments.
pub fn shell(dir: &Path, name: &str, source: &str, extra: &[&str]) -> PathBuf {
    let mut args = vec!["-shared", "-fPIC", "-L", ".", "-lgnoblin-core", "-o", name];
    args.extend_from_slice(extra);
    cc(dir, &format!("{name}.c"), source, &args);
    dir.join(name)
}

/// A launcher that references the core, with the run path `dist install` gives `bin/gnoblin`.
pub fn launcher(dir: &Path, name: &str, core_dir: &Path, rpath: &str) -> PathBuf {
    let source = "extern void gnoblin_abi_version(unsigned *, unsigned *, unsigned *);\n\
                  int main(void) { unsigned a, b, c; gnoblin_abi_version(&a, &b, &c); return 0; }\n";
    let search = core_dir.display().to_string();
    cc(
        dir,
        &format!("{name}.c"),
        source,
        &[
            "-L",
            &search,
            "-lgnoblin-core",
            &format!("-Wl,-rpath,{rpath}"),
            "-o",
            name,
        ],
    );
    dir.join(name)
}
