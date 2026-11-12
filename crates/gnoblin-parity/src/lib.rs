//! The parity scenario runner.
//!
//! Parity is data in this workspace: one declarative scenario runs against
//! several targets, and the report says whether the behaviour matched, differed,
//! or could not be run yet (design/rust-compositor.md, "The parity harness").
//! The pieces are separated so each can be tested on its own:
//!
//! - [`scenario`] reads the TOML files under `tests/parity/scenarios`.
//! - [`target`] obtains observations. `recorded` and `command` replay checked-in
//!   output; `candidate` runs the Rust compositor headless and drives it over IPC
//!   and Wayland. `mutter51` runs stock Mutter 51 headless through a launcher the
//!   environment names and is skipped, with the reason, where none is configured;
//!   `mutter-gnoblin` is not implemented and fails loudly, so a scenario never
//!   passes on a stand-in.
//! - [`normaliser`] removes the ids, serials and timestamps two conforming
//!   targets never agree on, so a comparison is about behaviour.
//! - [`diff`] and [`report`] say what differed, in words, per scenario.
//! - [`runner`] wires them together; the `gnoblin-parity` binary is a thin
//!   wrapper that prints the report and sets the exit status.

pub mod candidate;
pub mod diff;
pub mod mutter;
pub mod normaliser;
mod probes;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod target;
mod wayland_peer;
pub use candidate::CandidateTarget;
pub use diff::{Diff, DiffLine};
pub use mutter::Mutter51Target;
pub use normaliser::Normaliser;
pub use report::{Report, ScenarioReport, Verdict};
pub use runner::{RunError, run_directory, run_directory_in, run_scenario, run_scenario_file, scenario_files};
pub use scenario::{Scenario, ScenarioError};
pub use target::{CommandTarget, Observations, RecordedTarget, RunEnvironment, Target, TargetError};
