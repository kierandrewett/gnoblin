//! `gnoblin-xr`: the out-of-process OpenXR extension for Gnoblin.
//!
//! With no arguments it connects to the owner socket, attaches to the tree and
//! plans the `room` subtree for every OpenXR frame. It submits no layers yet.

mod bridge;
mod pointer;

use xr_scene::Options;
use xr_session::frame_loop::FrameLoop;
use xr_session::openxr_runtime::OpenxrRuntime;
use xr_tree::TreeClient;

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Connect to the owner and run the bridge until the session or the owner ends.
    Run,
    /// Print the usage text and exit successfully.
    Help,
    /// Print the version and exit successfully.
    Version,
    /// Anything else: print the usage text to stderr and exit with status 2.
    Invalid(String),
}

fn parse_args<I>(mut args: I) -> Action
where
    I: Iterator<Item = String>,
{
    match (args.next().as_deref(), args.next()) {
        (Some("--help" | "-h"), None) => Action::Help,
        (Some("--version" | "-V"), None) => Action::Version,
        (Some(other), _) => Action::Invalid(other.to_owned()),
        (None, _) => Action::Run,
    }
}

const USAGE: &str = "usage: gnoblin-xr [--help | --version]

Connects to the Gnoblin owner socket and runs the OpenXR bridge. The socket is
$GNOBLIN_IPC_SOCKET, or $XDG_RUNTIME_DIR/gnoblin/ipc.sock.
See extensions/gnoblin-xr/README.md.";

/// Connects to the owner and the OpenXR runtime, then runs the bridge until it ends.
fn run_bridge() -> Result<(), String> {
    let socket = gnoblin_client::unix::socket_path(|name| std::env::var(name).ok())?;
    let mut tree = TreeClient::connect(&socket).map_err(|error| format!("owner socket: {error}"))?;
    let runtime = OpenxrRuntime::connect("gnoblin-xr").map_err(|error| error.to_string())?;
    let mut frames = FrameLoop::new(runtime);

    // Planning runs every frame, but the summary is only worth a line when it changes.
    let mut last_summary = None;
    let outcome = bridge::run(&mut tree, &mut frames, &Options::default(), |plan| {
        let summary = (plan.layers.len(), plan.skipped.len());
        if last_summary != Some(summary) {
            eprintln!("gnoblin-xr: plan: {} layers, {} skipped", summary.0, summary.1);
            last_summary = Some(summary);
        }
    })
    .map_err(|error| error.to_string())?;
    eprintln!("gnoblin-xr: stopped: {outcome:?}");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Action::Run => match run_bridge() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("gnoblin-xr: {message}");
                std::process::ExitCode::FAILURE
            }
        },
        Action::Help => {
            println!("{USAGE}");
            std::process::ExitCode::SUCCESS
        }
        Action::Version => {
            println!("gnoblin-xr {}", env!("CARGO_PKG_VERSION"));
            std::process::ExitCode::SUCCESS
        }
        Action::Invalid(argument) => {
            eprintln!("gnoblin-xr: unknown argument `{argument}`\n{USAGE}");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Action {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn help_long_and_short_flags_print_help() {
        assert_eq!(parse(&["--help"]), Action::Help);
        assert_eq!(parse(&["-h"]), Action::Help);
    }

    #[test]
    fn version_long_and_short_flags_print_version() {
        assert_eq!(parse(&["--version"]), Action::Version);
        assert_eq!(parse(&["-V"]), Action::Version);
    }

    #[test]
    fn no_arguments_runs_the_bridge() {
        assert_eq!(parse(&[]), Action::Run);
    }

    #[test]
    fn unknown_argument_is_invalid() {
        assert_eq!(parse(&["--serve"]), Action::Invalid("--serve".to_owned()));
    }

    #[test]
    fn extra_arguments_after_a_flag_are_invalid() {
        assert_eq!(parse(&["--help", "extra"]), Action::Invalid("--help".to_owned()));
    }
}
