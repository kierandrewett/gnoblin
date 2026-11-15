//! The `mutter51` target: stock Mutter 51 on its headless backend, driven by the same Wayland client the candidate
//! uses.
//!
//! This crate does not build or ship Mutter. The environment names a launcher program, and the target runs it:
//!
//! | Variable                            | Meaning                                                                         |
//! | ----------------------------------- | ------------------------------------------------------------------------------- |
//! | `GNOBLIN_PARITY_MUTTER51`           | The launcher: `<launcher> <runtime-dir> <display>`. Required; without it the scenario is skipped. |
//! | `GNOBLIN_PARITY_RUNTIME_BASE`       | The directory the private runtime directory is made in. Default: the system temporary directory. |
//!
//! [`crate::target::RunEnvironment`] reads both.
//!
//! The launcher starts headless Mutter 51 with `XDG_RUNTIME_DIR` set to `<runtime-dir>` and its Wayland socket named
//! `<display>` inside it, with one virtual monitor, and stays in the foreground until it is killed
//! ([`crate::launcher`]). It also starts the private session bus at `<runtime-dir>/bus`, which is how the input steps
//! reach `org.gnome.Mutter.RemoteDesktop` for an EIS socket ([`crate::remote_desktop`]). The target kills the
//! launcher's whole process group when the steps are done. `scripts/parity-mutter51` is the launcher for a machine
//! where `mutter` is installed.
//!
//! Only the probes every live target answers run here. A step that asks for a candidate-only probe (`window.open`,
//! which reads `gnoblinctl`) fails with `no probe`, so a scenario cannot name `mutter51` for something Mutter
//! has no equivalent of.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::launcher::{BUS_SOCKET, DISPLAY, LauncherSession, Names, private_directory};
use crate::probes::{self, Probes};
use crate::remote_desktop::RemoteDesktopDoor;
use crate::target::{Observations, Target, TargetError};

/// The target name the report prints and the errors name.
const TARGET: &str = "mutter51";
/// The environment variable that names the launcher.
pub const LAUNCHER_VARIABLE: &str = "GNOBLIN_PARITY_MUTTER51";
const NAMES: Names = Names {
    target: TARGET,
    log: "mutter",
};

/// Runs scenarios against stock Mutter 51, started by the launcher the environment names.
#[derive(Debug)]
pub struct Mutter51Target {
    launcher: OsString,
    runtime_base: Option<PathBuf>,
}

impl Mutter51Target {
    /// A target that runs `launcher`, making its runtime directory under `runtime_base` (the system temporary
    /// directory when absent).
    pub fn new(launcher: OsString, runtime_base: Option<PathBuf>) -> Self {
        Self { launcher, runtime_base }
    }
}

fn live(detail: impl Into<String>) -> TargetError {
    TargetError::Live {
        target: TARGET.to_owned(),
        detail: detail.into(),
    }
}

impl Target for Mutter51Target {
    fn name(&self) -> &str {
        TARGET
    }

    fn observe(&self, scenario: &crate::scenario::Scenario) -> Result<Observations, TargetError> {
        if let Some(step) = scenario.steps.iter().find(|step| !probes::handles(&step.id)) {
            return Err(live(format!(
                "no probe for step `{}`: the reference answers only the Wayland probes",
                step.id
            )));
        }
        let directory = private_directory(TARGET, self.runtime_base.as_deref())?;
        let mut session = LauncherSession::start(NAMES, &self.launcher, &[], directory.path())?;
        let socket = directory.path().join(DISPLAY);
        let door = Box::new(RemoteDesktopDoor::new(TARGET, directory.path().join(BUS_SOCKET)));
        let mut probes = Probes::connect(TARGET, &socket, Some(door)).map_err(|error| session.explain(error))?;
        let mut observations = Observations::new();
        for step in &scenario.steps {
            let value = probes.run(&step.id).map_err(|error| session.explain(error))?;
            observations.insert(step.id.clone(), value);
        }
        drop(probes);
        session.stop();
        Ok(observations)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::scenario::Scenario;

    fn scenario(step: &str) -> Scenario {
        Scenario::parse(
            &format!("name = \"s\"\ntarget = {{ mutter51 = {{}} }}\n[[steps]]\nid = \"{step}\"\n"),
            Path::new("<memory>"),
        )
        .expect("scenario parses")
    }

    #[test]
    fn a_step_only_the_candidate_answers_is_refused_before_anything_starts() {
        let target = Mutter51Target::new("/nonexistent/launcher".into(), None);
        let error = target
            .observe(&scenario("window.open"))
            .expect_err("the reference has no such probe");
        assert!(error.to_string().contains("no probe for step `window.open`"), "{error}");
    }

    #[test]
    fn a_launcher_that_exits_before_serving_is_a_failure_with_its_status_and_log() {
        let target = Mutter51Target::new("/bin/false".into(), None);
        let error = target
            .observe(&scenario("focus.first-toplevel"))
            .expect_err("no compositor is serving");
        let message = error.to_string();
        assert!(message.contains("the launcher exited with"), "{message}");
        assert!(message.contains("mutter log"), "{message}");
    }

    #[test]
    fn a_launcher_that_cannot_be_started_is_a_failure_naming_it() {
        let target = Mutter51Target::new("/nonexistent/launcher".into(), None);
        let error = target
            .observe(&scenario("focus.first-toplevel"))
            .expect_err("the launcher is missing");
        assert!(error.to_string().contains("cannot start the launcher"), "{error}");
    }
}
