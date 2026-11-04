//! The parity scenario runner.
//!
//! Parity is data in this workspace: one declarative scenario runs against
//! several targets, and the report says whether the behaviour matched, differed,
//! or could not be run yet (design/rust-compositor.md, "The parity harness").
//! The pieces are separated so each can be tested on its own:
//!
//! - [`scenario`] reads the TOML files under `tests/parity/scenarios`.
//! - [`target`] obtains observations. Two targets exist today, `recorded` and
//!   `command`; the three live ones name the lane that will bring them and fail
//!   loudly until it lands, so a scenario never passes on a stand-in.
//! - [`normaliser`] removes the ids, serials and timestamps two conforming
//!   targets never agree on, so a comparison is about behaviour.
//! - [`diff`] and [`report`] say what differed, in words, per scenario.
//! - [`runner`] wires them together; the `gnoblin-parity` binary is a thin
//!   wrapper that prints the report and sets the exit status.

pub mod diff;
pub mod normaliser;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod target;

pub use diff::{Diff, DiffLine};
pub use normaliser::Normaliser;
pub use report::{Report, ScenarioReport, Verdict};
pub use runner::{RunError, run_directory, run_scenario, run_scenario_file, scenario_files};
pub use scenario::{Scenario, ScenarioError};
pub use target::{CommandTarget, Observations, RecordedTarget, Target, TargetError};
