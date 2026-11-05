//! `cargo xtask gen-proto`: write the generated `.proto` text where a human can
//! read it.
//!
//! The build does not need this: `gnoblin-proto`'s build script renders the same
//! text into `OUT_DIR` and compiles it there (design/rust-compositor.md, "One IPC"
//! and D16). This command exists so a reviewer can read a wire change without
//! hunting through `target/`, and it writes to a gitignored directory under
//! `target/` so nothing generated can be committed by accident.
//!
//! The rendering itself lives in `gnoblin_schema::proto`, the one implementation
//! both the build script and this command call, so what a reviewer reads is
//! byte-for-byte what the build compiles.

use std::path::Path;
use std::process::ExitCode;

use gnoblin_schema::{Catalog, proto};

/// Schema files, relative to the workspace root (the `just` working directory).
const SCHEMA_DIR: &str = proto::SCHEMA_DIR;

/// Where the readable copies go, relative to the workspace root. Covered by the
/// `/target/` rule in `.gitignore`, which is why nothing here is checked in.
const OUTPUT_DIR: &str = "target/gen-proto";

/// Render the schema into [`OUTPUT_DIR`] and list what was written.
pub fn run() -> ExitCode {
    let catalog = match Catalog::load_dir(Path::new(SCHEMA_DIR)) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("[gen-proto] {error}");
            return ExitCode::from(2);
        }
    };
    match write(&catalog, Path::new(OUTPUT_DIR)) {
        Ok(paths) => {
            for path in paths {
                println!("[gen-proto] wrote {}", path.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("[gen-proto] {error}");
            ExitCode::from(2)
        }
    }
}

/// Every file written, in namespace order, with the table last.
fn write(catalog: &Catalog, dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    let mut written = Vec::new();
    let mut files = proto::proto_files(catalog);
    files.push(("sensitive.rs".to_owned(), proto::sensitive_table(catalog)));
    for (name, text) in files {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}
