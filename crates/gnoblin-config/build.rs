//! Generate the configuration document from `schema/config.toml`.
//!
//! The document is generated rather than hand-kept so a setting's type, range,
//! default, accepted-values text, `since` release and deprecated spellings are
//! declared once, in one file, and every surface that has to agree about them
//! (the structs, the validators, the reference tables, the Lua stubs, the
//! settings catalogue) is produced from that file. Nothing generated is
//! committed: `OUT_DIR` holds the Rust that `lib.rs` compiles with `include!`,
//! and `cargo xtask gen-config` writes the human-readable outputs into a
//! gitignored directory. A clean checkout therefore builds with plain
//! `cargo build`, with no generation step to run first and no stale checked-in
//! copy to drift.
//!
//! A schema mistake fails this build rather than producing a document that
//! cannot be validated, and every mistake is reported at once.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let schema_path = manifest.join("schema").join("config.toml");
    println!("cargo:rerun-if-changed={}", schema_path.display());
    let schema = match fs::read_to_string(&schema_path) {
        Ok(schema) => schema,
        Err(error) => {
            eprintln!("[config] cannot read {}: {error}", schema_path.display());
            return ExitCode::FAILURE;
        }
    };
    let artifacts = match gnoblin_config_codegen::generate(&schema) {
        Ok(artifacts) => artifacts,
        Err(errors) => {
            for error in errors {
                eprintln!("[config] {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    if let Err(error) = fs::write(out.join("generated.rs"), artifacts.rust) {
        eprintln!("[config] cannot write the generated module: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
