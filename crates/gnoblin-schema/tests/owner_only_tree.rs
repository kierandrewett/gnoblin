//! The owner-only `tree` namespace reaches only the session owner's build.
//!
//! design/node-core.md section 2.3: the tree is the owner socket's vocabulary. A catalog that a
//! non-owner surface builds (the CLI, the Lua bindings, a test fixture) must leave
//! `tree` out with `Catalog::load_dir_except(dir, &["tree"])`. A plain `load_dir` there lets the
//! tree's classes (`lock`, `destroyed`) collide with the public ones. This test scans every Rust
//! source in the workspace for a full-catalog `load_dir(` call and fails on any file that is not
//! on the owner list, so a new consumer cannot reintroduce the leak without editing that list.
//!
//! The scan covers the loader calls. A consumer that parses the schema files itself with
//! `parse_schema` is outside it, and the compositor's embedded catalog is one such path, owned
//! by the compositor.

use std::fs;
use std::path::{Path, PathBuf};

/// The crate that defines the loader and tests both variants. It is not a consumer.
const LOADER_CRATE: &str = "crates/gnoblin-schema/";

/// Files that may call the full-catalog loader, and why. Each must actually call it.
const OWNER_PATHS: &[(&str, &str)] = &[
    (
        "crates/gnoblin-compositor/build.rs",
        "the compositor serves the owner socket, so it embeds the whole catalog",
    ),
    (
        "crates/gnoblin-proto/build.rs",
        "the owner socket's tree wire types are generated from the whole catalog",
    ),
    (
        "crates/gnoblin-lua/build.rs",
        "the shell's tree class table is generated from the whole catalog",
    ),
    (
        "crates/gnoblin-lua/tests/tree_classes.rs",
        "the test pins the shell's tree class table against the whole catalog",
    ),
];

/// Whether a source line calls the full-catalog loader, as `Catalog::load_dir(` or
/// `Self::load_dir(`. A definition (`fn load_dir(`) and the `load_dir_except(` sibling are not.
fn calls_full_loader(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    code.match_indices("load_dir(")
        .any(|(at, _)| code[..at].ends_with("::"))
}

/// Every `.rs` file under `dir`, skipping build output and hidden directories.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn only_the_owner_paths_build_the_full_catalog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);

    let mut offenders = Vec::new();
    let mut owners = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&root)
            .expect("a file under the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.starts_with(LOADER_CRATE) {
            continue;
        }
        let text = fs::read_to_string(file).expect("a readable source file");
        if !text.lines().any(calls_full_loader) {
            continue;
        }
        if OWNER_PATHS.iter().any(|(path, _)| *path == relative) {
            owners.push(relative);
        } else {
            offenders.push(relative);
        }
    }

    assert!(
        offenders.is_empty(),
        "these load the full catalog, owner-only `tree` included. Load it with \
         `Catalog::load_dir_except(dir, &[\"tree\"])` unless the file is an owner path:\n  {}",
        offenders.join("\n  ")
    );
    for (path, why) in OWNER_PATHS {
        assert!(
            owners.iter().any(|owner| owner == path),
            "{path} is listed as an owner path ({why}) but no longer calls the full loader; \
             drop it from OWNER_PATHS"
        );
    }
}

#[test]
fn the_scan_tells_a_call_from_a_definition_and_a_comment() {
    assert!(calls_full_loader("    let catalog = Catalog::load_dir(&dir)?;"));
    assert!(calls_full_loader("        Self::load_dir(dir)"));
    assert!(!calls_full_loader("    Catalog::load_dir_except(&dir, &[\"tree\"])"));
    assert!(!calls_full_loader(
        "    pub fn load_dir(dir: &Path) -> Result<Catalog, SchemaError> {"
    ));
    assert!(!calls_full_loader("//! let catalog = Catalog::load_dir(path)?;"));
}
