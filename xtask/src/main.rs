//! Workspace automation: dependency-rule checks over `cargo metadata`, and
//! later the code generation the schema drives.

mod deps;
mod metadata;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <subcommand>

subcommands:
  deps    check the workspace dependency rules from design/rust-compositor.md";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let subcommand = args.next();
    if args.next().is_some() {
        eprintln!("[xtask] unexpected extra argument");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    match subcommand.as_deref() {
        Some("deps") => deps_command(),
        Some(other) => {
            eprintln!("[xtask] unknown subcommand `{other}`");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        None => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
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
