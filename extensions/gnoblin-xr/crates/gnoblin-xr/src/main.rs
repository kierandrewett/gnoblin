//! `gnoblin-xr`: the out-of-process OpenXR extension for Gnoblin.
//!
//! With no arguments it connects to the owner socket, attaches to the tree and
//! plans the `room` subtree for every OpenXR frame. Each planned View's picture is copied into a swapchain and
//! submitted as a layer.

mod bridge;
mod dump;
mod pointer;
mod supervise;
mod surfaces;

use xr_capture::views::ViewCaptures;
use xr_gpu::{GpuError, Pixels, Presenter, Target, VulkanPresenter};
use xr_layers::Notice;
use xr_scene::Options;
use xr_session::frame_loop::FrameLoop;
use xr_session::openxr_runtime::OpenxrRuntime;
use xr_session::runtime::Runtime;
use xr_tree::TreeClient;

use crate::bridge::{BridgeError, Outcome};
use crate::dump::Dump;
use crate::supervise::Failure;
use crate::surfaces::Fault;

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

/// The presenter for a session with no graphics binding. The bridge never reaches it: with no swapchain
/// capability it makes no swapchain, and says so once.
struct NoPresenter;

impl Presenter for NoPresenter {
    fn upload(&mut self, _target: Target, _pixels: &Pixels<'_>) -> Result<(), GpuError> {
        Err(GpuError::Unavailable("the session has no graphics device".to_owned()))
    }
}

/// Connects to the owner, then runs one OpenXR session after another until the owner goes.
///
/// The runtime may be absent, refuse the session, lose it or be restarted under it: each of those ends the session
/// and a new one is tried after a wait (see [`supervise`]), over the same owner connection. Only the owner ends the
/// program.
fn run_bridge() -> Result<(), String> {
    let socket = gnoblin_client::unix::socket_path(|name| std::env::var(name).ok())?;
    let mut tree = TreeClient::connect(&socket).map_err(|error| format!("owner socket: {error}"))?;
    // Capture is best effort: without a view-source connection the bridge still plans and submits.
    let client = match xr_capture::wayland::Client::connect_from_env() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("gnoblin-xr: view capture disabled: {error}");
            None
        }
    };
    let mut captures = ViewCaptures::new(client);
    supervise::supervise(
        &mut tree,
        &mut supervise::Backoff::default(),
        &mut |tree| session(tree, &mut captures),
        &mut supervise::wait,
        &mut |line| eprintln!("gnoblin-xr: {line}"),
    )
}

/// One OpenXR session, from connecting to the runtime to its end.
///
/// The session is a Vulkan one, so Views are shown. When the runtime cannot give one, the reason is printed and the
/// bridge falls back to a headless session, which plans and drives the pointer but shows nothing.
fn session(tree: &mut TreeClient, captures: &mut ViewCaptures) -> Result<Outcome, Failure> {
    match OpenxrRuntime::connect_vulkan("gnoblin-xr") {
        Ok((runtime, device)) => {
            let mut presenter = VulkanPresenter::new(device).map_err(|error| Failure::Runtime(error.to_string()))?;
            drive(tree, FrameLoop::new(runtime), captures, &mut presenter)
        }
        Err(graphics) => {
            let runtime = OpenxrRuntime::connect("gnoblin-xr")
                .map_err(|headless| Failure::Runtime(format!("no OpenXR session: {graphics}; headless: {headless}")))?;
            eprintln!("gnoblin-xr: no graphics session: {graphics}");
            eprintln!("gnoblin-xr: running a headless session: layers are planned and no picture is shown");
            drive(tree, FrameLoop::new(runtime), captures, &mut NoPresenter)
        }
    }
}

/// Whether `GNOBLIN_LOG` asks for debug output from the extension: a bare `debug` or `trace`, or one of those for the
/// target `xr` or `gnoblin-xr`. The extension has no `gnoblin-log` of its own, so this is the one place it reads the
/// project's filter variable.
fn debug_enabled(filter: Option<&str>) -> bool {
    let Some(filter) = filter else { return false };
    filter.split(',').any(|part| {
        let (target, level) = part.split_once('=').unwrap_or(("", part));
        matches!(target.trim(), "" | "xr" | "gnoblin-xr") && matches!(level.trim(), "debug" | "trace")
    })
}

/// Runs the bridge over one session, logging what changes from frame to frame. With `GNOBLIN_LOG` at `debug`, every
/// frame also logs how many layers it planned and submitted.
fn drive<R: Runtime, P: Presenter>(
    tree: &mut TreeClient,
    mut frames: FrameLoop<R>,
    captures: &mut ViewCaptures,
    presenter: &mut P,
) -> Result<Outcome, Failure> {
    // Planning runs every frame, but a line is only worth printing when it changes.
    let mut last_summary = None;
    let mut last_submitted = None;
    let mut last_notices: Vec<Notice> = Vec::new();
    let mut last_faults: Vec<Fault> = Vec::new();
    let debug = debug_enabled(std::env::var("GNOBLIN_LOG").ok().as_deref());
    let dump = Dump::from_value(std::env::var(dump::DUMP_VARIABLE).ok());
    let mut frame_number = 0_u64;
    let outcome = bridge::run(
        tree,
        &mut frames,
        &Options::default(),
        dump,
        captures,
        presenter,
        |report| {
            frame_number += 1;
            let summary = (report.plan.layers.len(), report.plan.skipped.len());
            if last_summary != Some(summary) {
                eprintln!("gnoblin-xr: plan: {} layers, {} skipped", summary.0, summary.1);
                last_summary = Some(summary);
            }
            if last_submitted != Some(report.submitted) {
                eprintln!(
                    "gnoblin-xr: frame {frame_number}: submitted {} layers",
                    report.submitted
                );
                last_submitted = Some(report.submitted);
            }
            if debug {
                eprintln!(
                    "gnoblin-xr: debug: frame {frame_number}: planned {}, submitted {}",
                    summary.0, report.submitted
                );
            }
            if last_notices != report.notices {
                for notice in report.notices {
                    eprintln!("gnoblin-xr: view {}: {}", notice.view, notice.kind.name());
                }
                last_notices = report.notices.to_vec();
            }
            if last_faults != report.faults {
                for fault in report.faults {
                    eprintln!("gnoblin-xr: {fault}");
                }
                last_faults = report.faults.to_vec();
            }
        },
    )
    .map_err(|error| match error {
        BridgeError::Tree(error) => Failure::Owner(format!("owner socket: {error}")),
        BridgeError::Runtime(error) => Failure::Runtime(error.to_string()),
    })?;
    eprintln!("gnoblin-xr: session stopped: {outcome:?}");
    Ok(outcome)
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
    fn debug_is_asked_for_by_a_level_or_by_the_extensions_own_target() {
        assert!(debug_enabled(Some("debug")));
        assert!(debug_enabled(Some("warn,xr=debug")));
        assert!(debug_enabled(Some("gnoblin-xr=trace")));
        assert!(!debug_enabled(Some("wayland=debug,warn")));
        assert!(!debug_enabled(Some("info")));
        assert!(!debug_enabled(None));
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
