//! Workspace automation: dependency-rule checks over `cargo metadata`, the
//! duplicate-type report over every crate's sources, and the code generation the
//! schema drives.

mod abi_c;
mod abi_check;
mod allow;
mod compat_report;
mod deps;
mod dist;
mod dist_check;
mod docs_parity;
mod dupes;
mod elf;
mod gen_proto;
mod layout;
mod logging;
mod metadata;
mod native_libs;
mod native_probe;
#[cfg(test)]
mod test_fixtures;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <subcommand>

subcommands:
  deps               check the workspace dependency rules from design/rust-compositor.md
  native-libs       fail on a -sys crate in a lock file whose system packages the CI install list lacks
  native-probe       probe an image's root file system for the native libraries the workspace needs; see
                     xtask/src/native_probe.rs (list, packages, check, unprobed, matrix)
  gen-proto          write the generated .proto text to target/gen-proto, to read
  compat-report      print every compat adapter's declared support matrix
  dupes              fail on a struct or enum two crates declare twice
  dupes --warn       the same report, printing every hit and exiting zero
  layout             fail on a Rust string literal that names the retired source-data tree
  logging            fail on a direct logging-crate dependency or a stock print in library code
  docs-parity        fail on a documented gnoblin name the implemented surface lacks, unless allow-listed
  compat-report --json  the same report as JSON
  dist install|uninstall --prefix P [--destdir D] [--profile NAME] [--libdir NAME]
                     install or remove the session tree; see xtask/src/dist.rs
  dist-check ...     check an installed or staged tree: core SONAME and exports, NEEDED entries, links, header, pkg-config;
                     see xtask/src/dist_check.rs
  abi-check [--update]  fail when crates/gnoblin-core-abi/abi.lock and the ABI crate disagree, or a released entry changed
  abi-header [--out FILE]  write the C header (include/gnoblin/core.h) generated from the ABI crate";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(subcommand) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest: Vec<String> = args.collect();
    match (subcommand.as_str(), rest.as_slice()) {
        ("deps", []) => deps_command(),
        ("gen-proto", []) => gen_proto::run(),
        ("layout", []) => layout::run(),
        ("logging", []) => logging::run(),
        ("native-libs", []) => native_libs::run(),
        ("docs-parity", []) => docs_parity::run(),
        ("compat-report", []) => compat_report::run(false),
        ("compat-report", [flag]) if flag == "--json" => compat_report::run(true),
        ("dupes", []) => dupes::run(true),
        ("dupes", [flag]) if flag == "--deny" => dupes::run(true),
        ("dupes", [flag]) if flag == "--warn" => dupes::run(false),
        ("native-probe", _) => native_probe::run(&rest),
        ("dist", _) => dist::run(&rest),
        ("dist-check", _) => dist_check::run(&rest),
        ("abi-check", _) => abi_check::run_check(&rest),
        ("abi-header", _) => abi_check::run_header(&rest),
        (
            "deps" | "gen-proto" | "dupes" | "compat-report" | "logging" | "layout" | "docs-parity" | "native-libs",
            _,
        ) => usage_error("unexpected argument"),
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
