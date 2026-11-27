//! `cargo xtask docs-coverage [--fail] [--list]`: how many tables of the libraries' schema files have no description.
//!
//! The reference of the website is built from the schema, so a class, property, method, parameter, signal, signal field, variant, enum,
//! enum value or callback with no `#` comment above its table is a hole in it. The command counts the holes by kind and lists them.
//!
//! - With no flag it prints the counts and the list and exits 0: the gap is reported, not enforced (`just workspace-ci` runs it so).
//! - `--fail` exits 1 when any table has no description, or any file has no `title`: the gate for the day the gap is closed.
//! - `--list` prints only the counts, without the list of names.
//!
//! The description of a table is the comment directly above it (see `crates/gnoblin-sdkgen/src/docs.rs`); a parameter, signal field,
//! enum value, callback or variant is described the same way.

use std::path::Path;
use std::process::ExitCode;

use gnoblin_sdkgen::{SdkSpec, coverage};

const USAGE: &str = "usage: cargo xtask docs-coverage [--fail] [--counts]";

/// The `docs-coverage` command.
pub fn run(args: &[String]) -> ExitCode {
    let mut fail = false;
    let mut counts_only = false;
    for arg in args {
        match arg.as_str() {
            "--fail" => fail = true,
            "--counts" => counts_only = true,
            other => {
                eprintln!("[docs-coverage] unexpected argument `{other}`\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let spec = match SdkSpec::load(Path::new(".")) {
        Ok(spec) => spec,
        Err(error) => {
            eprintln!("[docs-coverage] {error}");
            return ExitCode::from(2);
        }
    };
    let report = coverage(&spec);
    for kind in &report.kinds {
        println!(
            "[docs-coverage] {:<18} {:>5} tables, {:>5} without a description",
            kind.kind,
            kind.total,
            kind.missing.len()
        );
    }
    let total: usize = report.kinds.iter().map(|kind| kind.total).sum();
    println!(
        "[docs-coverage] {:<18} {:>5} tables, {:>5} without a description; {} file(s) without a title",
        "all",
        total,
        report.missing(),
        report.untitled.len()
    );
    if !counts_only {
        for kind in &report.kinds {
            for name in &kind.missing {
                println!("missing {}: {name}", kind.kind);
            }
        }
        for file in &report.untitled {
            println!("missing title: {file}");
        }
    }
    if fail && (report.missing() > 0 || !report.untitled.is_empty()) {
        eprintln!(
            "[docs-coverage] the schema leaves {} table(s) undescribed",
            report.missing()
        );
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
