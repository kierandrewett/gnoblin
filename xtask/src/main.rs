//! Workspace automation: dependency-rule checks over `cargo metadata`, the
//! duplicate-type report over every crate's sources, and the code generation the
//! schema drives.

mod compat_report;
mod deps;
mod dupes;
mod gen_proto;
mod metadata;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <subcommand>

subcommands:
  deps               check the workspace dependency rules from design/rust-compositor.md
  gen-proto          write crates/gnoblin-proto/proto from the object schema
  gen-proto --check  fail when a checked-in .proto file is stale or orphaned
  compat-report      print the declared support matrix of every compat adapter
  dupes              list the structs and enums two crates declare twice
  dupes --deny       the same report, failing on a hit not in xtask/dupes-allow.toml
  compat-report --json  the same report as JSON";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(subcommand) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest: Vec<String> = args.collect();
    match (subcommand.as_str(), rest.as_slice()) {
        ("deps", []) => deps_command(),
        ("gen-proto", []) => gen_proto::run(false),
        ("gen-proto", [flag]) if flag == "--check" => gen_proto::run(true),
        ("compat-report", []) => compat_report::run(false),
        ("compat-report", [flag]) if flag == "--json" => compat_report::run(true),
        ("dupes", []) => dupes::run(false),
        ("dupes", [flag]) if flag == "--deny" => dupes::run(true),
        ("deps" | "gen-proto" | "dupes" | "compat-report", _) => usage_error("unexpected argument"),
        (other, _) => usage_error(&format!("unknown subcommand `{other}`")),
    }
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("[xtask] {message}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// Load the workspace graph and report every dependency-rule violation.
///
/// The graph comes from the tree `cargo xtask` was run in, so a lane checks its
/// own checkout rather than whichever checkout the binary was last built from.
/// Failures are reported as data rather than panics so CI prints one line per
/// broken edge, and so the fix names the crate pair that has to change.
fn deps_command() -> ExitCode {
    let graph = match metadata::load(Path::new(".")) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("[deps] {error}");
            return ExitCode::from(2);
        }
    };
    let violations = deps::check(&graph);
    if violations.is_empty() {
        println!("[deps] {} packages checked, no violations", graph.packages.len());
        return ExitCode::SUCCESS;
    }
    for violation in &violations {
        println!("[deps] {violation}");
    }
    eprintln!("[deps] {} violation(s) in dependency rules", violations.len());
    ExitCode::FAILURE
}
