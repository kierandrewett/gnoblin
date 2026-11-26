//! Workspace automation: dependency-rule checks over `cargo metadata`, the
//! duplicate-type report over every crate's sources, and the code generation the
//! schema drives.

mod abi_check;
mod allow;
mod compat_report;
mod deps;
mod dist;
mod dist_check;
mod docs_examples;
mod docs_parity;
mod docs_tabs;
mod dupes;
mod elf;
mod gen_proto;
mod gen_sdks;
mod golden;
mod logging;
mod metadata;
mod native_libs;
mod native_probe;
mod oracle;
mod profiles;
mod protocol_crates;
mod stage;
#[cfg(test)]
mod test_fixtures;
mod watch_shell;

// The C-type grammar and the header renderer live in the generator library, which needs them to read the ABIs; the commands here
// keep their `crate::abi_c` spelling.
pub(crate) use gnoblin_sdkgen::abi_c;

use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <subcommand>

subcommands:
  deps               check the workspace dependency rules,
                     and that no Cargo profile sets panic = abort
  native-libs       fail on a -sys crate in a lock file whose system packages the CI install list lacks
  native-probe       probe an image's root file system for the native libraries the workspace needs; see
                     xtask/src/native_probe.rs (list, packages, check, unprobed, matrix)
  gen-proto          write the generated .proto text to target/gen-proto, to read
  compat-report      print every compat adapter's declared support matrix
  dupes              fail on a struct or enum two crates declare twice
  dupes --warn       the same report, printing every hit and exiting zero
  logging            fail on a direct logging-crate dependency or a stock print in library code
  docs-parity        fail on a documented gnoblin name the implemented surface lacks, unless allow-listed
  docs-examples [--lang typescript|python|rust]... [--out DIR] [PAGE.md ...]
                     check the examples of every `::: tabs` group of docs/ in its own language: tsc over the installed
                     declarations, pyright over the installed stubs, cargo check against the SDK crates; a failure names the
                     page and the line; see xtask/src/docs_examples.rs
  docs-reference [--out DIR]
                     write the generated API reference pages (TypeScript, Python, Rust) into infra/site/.generated/docs,
                     where the website reads them; they are never committed
  compat-report --json  the same report as JSON
  dist install|uninstall --prefix P [--destdir D] [--profile NAME] [--libdir NAME]
                     install or remove the session tree; see xtask/src/dist.rs
  dist-check ...     check an installed or staged tree: core SONAME and exports, NEEDED entries and how they resolve,
                     links, header, pkg-config, manifest and layout, glibc floor; see xtask/src/dist_check.rs
  watch-shell PROJECT [--once] [--release] [--debounce MS] [-- CARGO_ARGS...]
                     rebuild a native shell project when its source changes, so a compositor watching the library reloads it;
                     a failed build leaves the library and prints the compiler's output; see xtask/src/watch_shell.rs
  stage [--release | --profile NAME] [--no-build]
                     build gnoblin-core-impl and name it libgnoblin-core.so.1 beside the binary, so `cargo build -p gnoblin`
                     output runs; see xtask/src/stage.rs
  abi-check [--update]  fail when crates/gnoblin-core-abi/abi.lock or crates/gnoblin-std-abi/abi.lock and its ABI crate
                     disagree, or a released entry changed
  abi-header [--std] [--out FILE]  write the C header generated from the ABI crate (include/gnoblin/core.h, or std.h
                     with --std)
  gen-sdks [--out DIR] [--check] [--update] [--target NAME]
                     write a readable copy of every generated SDK file to target/sdk; --check proves the generator renders
                     deterministically, its Python compiles and the public surface and the C ABI match their locks; --update
                     records an additive change in crates/gnoblin-schema/schema/lock; see xtask/src/gen_sdks.rs
  oracle record | check [--repeat N] | list
                     write, or prove stable, the golden fixtures recorded from the TypeScript std for the native std port;
                     see xtask/src/oracle.rs
  protocol-crates [--check]  write the manifests of the Wayland protocol crates (crates/protocols) and their aggregate from
                     xtask/protocols.toml, or with --check fail when a generated file is stale or the table and the tree differ";

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
        ("gen-sdks", _) => gen_sdks::run(&rest),
        ("protocol-crates", _) => protocol_crates::run(&rest),
        ("logging", []) => logging::run(),
        ("native-libs", []) => native_libs::run(),
        ("stage", _) => stage::run(&rest),
        ("watch-shell", _) => watch_shell::run(&rest),
        ("docs-parity", []) => docs_parity::run(),
        ("docs-examples", _) => docs_examples::run_examples(&rest),
        ("docs-reference", _) => docs_examples::run_reference(&rest),
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
        ("oracle", _) => oracle::run(&rest),
        ("deps" | "gen-proto" | "dupes" | "compat-report" | "logging" | "docs-parity" | "native-libs", _) => {
            usage_error("unexpected argument")
        }
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
    let mut violations: Vec<String> = deps::check(&graph).iter().map(ToString::to_string).collect();
    match profiles::check(Path::new(".")) {
        Ok(aborting) => violations.extend(aborting.into_iter().map(|line| format!("[profile-unwind] {line}"))),
        Err(error) => {
            eprintln!("[deps] {error}");
            return ExitCode::from(2);
        }
    }
    if violations.is_empty() {
        println!(
            "[deps] {} packages checked, no violations (panic strategy: unwind)",
            graph.packages.len()
        );
        return ExitCode::SUCCESS;
    }
    for violation in &violations {
        println!("[deps] {violation}");
    }
    eprintln!("[deps] {} violation(s) in dependency rules", violations.len());
    ExitCode::FAILURE
}
