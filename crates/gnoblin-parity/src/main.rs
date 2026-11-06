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
usage: gnoblin-parity run <scenario-directory>

  run <dir>   run every *.toml scenario in <dir>, in file-name order

Exit status: 0 when no scenario failed, 1 when one differed, 2 on a usage error.";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match command.as_str() {
        "run" => run(args.next()),
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
fn run(directory: Option<String>) -> ExitCode {
    let Some(directory) = directory else {
        eprintln!("[parity] run needs a scenario directory");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match gnoblin_parity::run_directory(Path::new(&directory)) {
        Ok(report) => {
            print!("{report}");
            if report.is_ok() {
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
