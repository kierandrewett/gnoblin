//! The scenario runner's command line.
//!
//! `run` runs a directory of scenarios and prints the report. It exits 0 when no scenario failed, 1 when at least
//! one did, and 2 when the request itself was wrong, so CI distinguishes a behavioural difference from a mistyped
//! path. `perf` takes the performance baseline of one target, and `serve-candidate` is the process `perf` measures
//! when the target is the Rust candidate.

use std::path::Path;
use std::process::ExitCode;

use gnoblin_parity::RunEnvironment;
use gnoblin_parity::perf;

/// Usage text, printed for `--help` and for a wrong invocation.
const USAGE: &str = "\
usage: gnoblin-parity run <scenario-directory> [--require <target>]...
       gnoblin-parity perf <candidate|mutter51|mutter-gnoblin> [--json <file>]

  run <dir>           run every *.toml scenario in <dir>, in file-name order. A scenario that names
                      a `mutter51` or `mutter-gnoblin` target or reference runs it when
                      GNOBLIN_PARITY_MUTTER51 or GNOBLIN_PARITY_MUTTER_GNOBLIN names a launcher
                      (tests/parity/README.md) and is reported as skipped otherwise.
  --require <target>  fail when a row for <target> was skipped, so a job that exists to run a
                      reference cannot pass without having run it
  perf <target>       measure frame time, key latency, idle CPU and memory with 0, 10 and 50 windows
                      on one target, from outside its process, and print the result; --json also
                      writes it to <file> (tests/parity/README.md, \"Performance baseline\")

Exit status: 0 when no scenario failed, 1 when one differed or a required target was skipped,
2 on a usage error.";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(command) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match command.to_string_lossy().as_ref() {
        "run" => run(args.map(|arg| arg.to_string_lossy().into_owned()).collect()),
        "perf" => measure(args.collect()),
        // Internal: the candidate as a process of its own, started by `perf`. Not in the usage text.
        "serve-candidate" => serve_candidate(args.collect()),
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

/// Take a baseline and print it.
fn measure(args: Vec<std::ffi::OsString>) -> ExitCode {
    let request = match perf::PerfRequest::parse(args) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("[parity] {error}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let environment = RunEnvironment::from_process();
    let report = match perf::run(request.target, &environment) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("[parity] {error}");
            return ExitCode::from(1);
        }
    };
    print!("{}", perf::render(&report));
    if let Some(path) = request.json {
        let written = serde_json::to_string_pretty(&report)
            .map_err(|error| error.to_string())
            .and_then(|text| std::fs::write(&path, text + "\n").map_err(|error| error.to_string()));
        if let Err(error) = written {
            eprintln!("[parity] cannot write {}: {error}", path.display());
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}

/// Serve the candidate until killed: `serve-candidate <runtime-dir> <display>`.
fn serve_candidate(args: Vec<std::ffi::OsString>) -> ExitCode {
    let [directory, display] = args.as_slice() else {
        eprintln!("[parity] serve-candidate needs a runtime directory and a display name");
        return ExitCode::from(2);
    };
    match perf::serve_candidate(Path::new(directory), &display.to_string_lossy()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[parity] {error}");
            ExitCode::from(1)
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
