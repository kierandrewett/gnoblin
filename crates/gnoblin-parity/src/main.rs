//! The scenario runner's command line.
//!
//! One subcommand, because the runner has one job: run a directory of scenarios
//! and print the report. `run` exits 0 when no scenario failed, 1 when at least
//! one did, and 2 when the request itself was wrong, so CI distinguishes a
//! behavioural difference from a mistyped path.

use std::path::Path;
use std::process::ExitCode;

/// Usage text, printed for `--help` and for a wrong invocation.
const USAGE: &str = "\
usage: gnoblin-parity run <scenario-directory> [--require <target>]...

  run <dir>           run every *.toml scenario in <dir>, in file-name order. A scenario that names
                      a `mutter51` reference runs it when GNOBLIN_PARITY_MUTTER51 names a launcher
                      (tests/parity/README.md) and is reported as skipped otherwise.
  --require <target>  fail when a row for <target> was skipped, so a job that exists to run the
                      reference cannot pass without having run it

Exit status: 0 when no scenario failed, 1 when one differed or a required target was skipped,
2 on a usage error.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match command.as_str() {
        "run" => run(args.collect()),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("[parity] unknown command `{other}`");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Run the scenarios in one directory and print the report.
fn run(args: Vec<String>) -> ExitCode {
    let mut directory = None;
    let mut required = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--require" {
            let Some(target) = args.next() else {
                eprintln!("[parity] --require needs a target name");
                return ExitCode::from(2);
            };
            required.push(target);
        } else if directory.is_none() {
            directory = Some(arg);
        } else {
            eprintln!("[parity] unexpected argument `{arg}`");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(directory) = directory else {
        eprintln!("[parity] run needs a scenario directory");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match gnoblin_parity::run_directory(Path::new(&directory)) {
        Ok(report) => {
            print!("{report}");
            let unrun: Vec<&str> = required
                .iter()
                .map(String::as_str)
                .filter(|target| report.skipped_for(target) > 0)
                .collect();
            for target in &unrun {
                eprintln!("[parity] a row for the required target `{target}` was skipped, so it did not run");
            }
            if report.is_ok() && unrun.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("[parity] {error}");
            ExitCode::from(2)
        }
    }
}
