//! The parity scenario runner.
//!
//! Parity is data in this workspace: one declarative scenario runs against
//! several targets, and the report says whether the behaviour matched, differed,
//! or could not be run yet.
//! The pieces are separated so each can be tested on its own:
//!
//! - [`scenario`] reads the TOML files under `tests/parity/scenarios`.
//! - [`target`] obtains observations. `recorded` and `command` replay checked-in
//!   output; `candidate` runs the Rust compositor headless and drives it over IPC
//!   and Wayland. `mutter51` runs stock Mutter 51 headless through a launcher the
//!   environment names and is skipped, with the reason, where none is configured;
//!   `mutter-gnoblin` does the same for the reference Gnoblin on its patched Mutter,
//!   with a second program for its control socket. No target is a stand-in: one
//!   that cannot start is a skip with its reason or a failure with the
//!   compositor's log.
//! - [`normaliser`] removes the ids, serials and timestamps two conforming
//!   targets never agree on, so a comparison is about behaviour.
//! - [`diff`] and [`report`] say what differed, in words, per scenario.
//! - [`runner`] wires them together; the `gnoblin-parity` binary is a thin
//!   wrapper that prints the report and sets the exit status.
//! - [`perf`] takes the performance baseline of a target from outside its process.
//!
//! Input reaches every live target the same way, through its EIS server: [`mod@ei_peer`] is the client,
//! [`remote_desktop`] the door of the Mutter targets and [`candidate`] the candidate's.

pub mod candidate;
pub mod diff;
mod ei_peer;
mod hold;
mod launcher;
pub mod mutter;
pub mod mutter_gnoblin;
pub mod normaliser;
pub mod perf;
mod probes;
mod procfs;
mod remote_desktop;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod target;
mod wayland_peer;
pub use candidate::CandidateTarget;
pub use diff::{Diff, DiffLine};
pub use hold::hold_window;
pub use mutter::Mutter51Target;
pub use mutter_gnoblin::{MutterGnoblinLaunch, MutterGnoblinTarget};
pub use normaliser::Normaliser;
pub use report::{Report, ScenarioReport, Verdict};
pub use runner::{RunError, run_directory, run_directory_in, run_scenario, run_scenario_file, scenario_files};
pub use scenario::{Scenario, ScenarioError};
pub use target::{CommandTarget, Observations, RecordedTarget, RunEnvironment, Target, TargetError};
