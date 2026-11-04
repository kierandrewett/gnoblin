//! Compile the checked-in `.proto` files into the crate's wire types.
//!
//! The `.proto` files are the single source of those types
//! (design/rust-compositor.md, "One IPC" and D16): `cargo xtask gen-proto` writes
//! one per schema namespace, `envelope.proto` is hand-written because the envelope
//! is not a schema member, and this script compiles all of them the same way.
//!
//! The front end is `protox`, which is the protobuf compiler written in Rust. That
//! keeps the build hermetic: no `protoc` binary and no C++ toolchain have to exist
//! on a runner, and nothing depends on the architecture of a vendored executable.
//!
//! The descriptor set that the Rust types are generated from is also written to
//! `OUT_DIR/descriptor.bin`. `src/proto_tags.rs` compares it with the checked-in
//! text, so a field tag in the compiled types cannot drift from the tag the
//! `.proto` file states.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use prost::Message as _;

fn main() -> ExitCode {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let proto_dir = manifest.join("proto");
    // The whole directory, so adding a namespace needs no edit here.
    println!("cargo:rerun-if-changed=proto");

    let mut files: Vec<PathBuf> = match fs::read_dir(&proto_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("proto"))
            .collect(),
        Err(error) => {
            eprintln!("[proto] cannot read {}: {error}", proto_dir.display());
            return ExitCode::FAILURE;
        }
    };
    files.sort();
    if files.is_empty() {
        eprintln!("[proto] no .proto files in {}", proto_dir.display());
        return ExitCode::FAILURE;
    }

    let descriptors = match protox::compile(&files, [proto_dir.as_path()]) {
        Ok(descriptors) => descriptors,
        Err(error) => {
            eprintln!("[proto] cannot compile the checked-in .proto files: {error}");
            return ExitCode::FAILURE;
        }
    };

    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        eprintln!("[proto] OUT_DIR is not set");
        return ExitCode::FAILURE;
    };
    if let Err(error) = fs::write(out_dir.join("descriptor.bin"), descriptors.encode_to_vec()) {
        eprintln!("[proto] cannot write the descriptor set: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = prost_build::compile_fds(descriptors) {
        eprintln!("[proto] cannot generate Rust from the descriptor set: {error}");
        return ExitCode::FAILURE;
    }
    println!("[proto] compiled {} .proto file(s)", files.len());
    ExitCode::SUCCESS
}
