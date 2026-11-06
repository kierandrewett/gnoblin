//! Generate the wire types from the object schema, in `OUT_DIR`.
//!
//! Nothing the IPC compiles is checked in: `cargo build` runs this script, which
//! reads `crates/gnoblin-schema/schema/*.toml`, renders one `.proto` document per
//! namespace and the Rust table of secret members, and compiles them where they
//! were written (design/rust-compositor.md, "One IPC" and D16). A clean checkout
//! therefore needs no generator step first, and a schema edit is a build rather
//! than a regeneration somebody has to remember.
//!
//! `proto/envelope.proto` is the one hand-written file beside this crate, because
//! the envelope is not a schema member but the layer that carries them. It is
//! compiled with the generated documents, so its types come from its own text.
//!
//! The front end is `protox`, the Protocol Buffers compiler written in Rust, so the
//! build is hermetic: no `protoc` binary and no C++ toolchain have to exist on a
//! runner, and nothing depends on the architecture of a vendored executable.
//!
//! Three files are written for the crate and its tests: `OUT_DIR/proto/` holds the
//! generated text that was compiled, `OUT_DIR/sensitive.rs` the table
//! `src/generated.rs` includes, and `OUT_DIR/descriptor.bin` the descriptor set.
//! `src/proto_tags.rs` compares that text with that descriptor.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gnoblin_schema::Catalog;
use prost::Message as _;

/// The schema directory, relative to this crate.
const SCHEMA_DIR: &str = "../gnoblin-schema/schema";

/// The namespace whose version is the IPC version, which is exactly the Gnoblin version.
const OWN_NAMESPACE: &str = "gnoblin";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[proto] {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let hand_written_dir = manifest.join("proto");
    let schema_dir = manifest.join(SCHEMA_DIR);
    let out_dir = std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "OUT_DIR is not set".to_owned())?;
    let generated_dir = out_dir.join("proto");

    // Every schema file by name, so editing one regenerates. The directory is
    // watched as well, because a file appearing or disappearing changes its
    // timestamp and not the one of the files already inside it.
    watch(&schema_dir)?;
    watch(&hand_written_dir)?;

    let catalog = Catalog::load_dir(&schema_dir)
        .map_err(|error| format!("cannot load the schema in {}: {error}", schema_dir.display()))?;

    fs::create_dir_all(&generated_dir)
        .map_err(|error| format!("cannot create {}: {error}", generated_dir.display()))?;
    let mut sources: Vec<PathBuf> = Vec::new();
    for (name, text) in gnoblin_schema::proto::proto_files(&catalog) {
        let path = generated_dir.join(name);
        fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        sources.push(path);
    }
    // The version every IPC peer advertises is the schema's, and the handshake is what
    // compares it. It is emitted here, once, because this is the crate that already reads
    // the schema and the crate both sides of a handshake depend on; a literal in each role
    // would be a second place to forget to bump.
    let namespace = gnoblin_schema::Namespace::parse(OWN_NAMESPACE)
        .map_err(|reason| format!("namespace `{OWN_NAMESPACE}`: {reason}"))?;
    let version = catalog.version(&namespace).ok_or_else(|| {
        format!("the schema declares no `{OWN_NAMESPACE}` namespace, so there is no IPC version to advertise")
    })?;
    println!("cargo:rustc-env=GNOBLIN_IPC_VERSION={version}");

    let sensitive = out_dir.join("sensitive.rs");
    fs::write(&sensitive, gnoblin_schema::proto::sensitive_table(&catalog))
        .map_err(|error| format!("cannot write {}: {error}", sensitive.display()))?;

    let envelope = hand_written_dir.join("envelope.proto");
    if !envelope.is_file() {
        return Err(format!("{} is missing", envelope.display()));
    }
    sources.push(envelope);
    // Sorted, so the descriptor set does not depend on directory order.
    sources.sort();

    let descriptors = protox::compile(&sources, [generated_dir.as_path(), hand_written_dir.as_path()])
        .map_err(|error| format!("cannot compile the generated .proto text: {error}"))?;

    let descriptor_path = out_dir.join("descriptor.bin");
    fs::write(&descriptor_path, descriptors.encode_to_vec())
        .map_err(|error| format!("cannot write {}: {error}", descriptor_path.display()))?;
    prost_build::compile_fds(descriptors)
        .map_err(|error| format!("cannot generate Rust from the descriptor set: {error}"))?;

    println!("[proto] compiled {} .proto file(s)", sources.len());
    Ok(())
}

/// Tell Cargo to rerun when anything in `dir` changes.
///
/// A directory is not enough on its own: cargo compares the timestamps it saw, and
/// editing a file that already exists does not move its parent's. Missing a schema
/// edit would leave the wire types from the previous schema, which is the failure
/// this whole arrangement exists to prevent.
fn watch(dir: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", dir.display());
    let entries = fs::read_dir(dir).map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
    Ok(())
}
