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
//! [`crate::target::Environment`] reads both.
//!
//! The launcher starts headless Mutter 51 with `XDG_RUNTIME_DIR` set to `<runtime-dir>` and its Wayland socket named
//! `<display>` inside it, with one virtual monitor, and stays in the foreground until it is killed. The target kills
//! the launcher's whole process group when the steps are done. `scripts/parity-mutter51` is the launcher for a
//! machine where `mutter` is installed.
//!
//! Only the probes every live target answers run here. A step that asks for a candidate-only probe (`window.open`,
//! which reads `gnoblinctl`) fails with `no probe`, so a scenario cannot name `mutter51` for something Mutter
//! has no equivalent of.

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::probes::{self, Probes};
use crate::target::{Observations, Target, TargetError};

/// The target name the report prints and the errors name.
const TARGET: &str = "mutter51";
/// The environment variable that names the launcher.
pub const LAUNCHER_VARIABLE: &str = "GNOBLIN_PARITY_MUTTER51";
/// The Wayland socket name the launcher is asked to serve.
const DISPLAY: &str = "parity-0";
/// How long Mutter has to start serving its socket.
const START: Duration = Duration::from_secs(90);
/// How long Mutter has to exit after the kill before the target gives up waiting.
const STOP: Duration = Duration::from_secs(10);
/// How often the target looks again.
const POLL: Duration = Duration::from_millis(20);

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
        let directory = self.private_directory()?;
        let mut session = MutterSession::start(&self.launcher, directory.path())?;
        let socket = directory.path().join(DISPLAY);
        let mut probes = Probes::connect(TARGET, &socket).map_err(|error| session.explain(error))?;
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

impl Mutter51Target {
    /// A directory only this user can enter, as `$XDG_RUNTIME_DIR` is.
    fn private_directory(&self) -> Result<tempfile::TempDir, TargetError> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("gnoblin-parity-");
        let directory = match &self.runtime_base {
            Some(base) => builder.tempdir_in(base),
            None => builder.tempdir(),
        }
        .map_err(|error| live(format!("cannot create the runtime directory: {error}")))?;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|error| live(format!("cannot make the runtime directory private: {error}")))?;
        Ok(directory)
    }
}

/// The launcher process and the log it writes.
struct MutterSession {
    child: Child,
    log: PathBuf,
    /// Set once the launcher is gone, so a second `stop` never signals a process group that may belong to someone else.
    stopped: bool,
}

impl MutterSession {
    /// Starts the launcher in a process group of its own and waits until its Wayland socket accepts.
    fn start(launcher: &OsString, directory: &Path) -> Result<Self, TargetError> {
        let log = directory.join("mutter.log");
        let log_file = std::fs::File::create(&log).map_err(|error| live(format!("cannot create the log: {error}")))?;
        let log_error = log_file
            .try_clone()
            .map_err(|error| live(format!("cannot share the log: {error}")))?;
        let child = Command::new(launcher)
            .arg(directory)
            .arg(DISPLAY)
            .env("XDG_RUNTIME_DIR", directory)
            .stdin(Stdio::null())
            .stdout(log_file)
            .stderr(log_error)
            .process_group(0)
            .spawn()
            .map_err(|error| live(format!("cannot start the launcher {launcher:?}: {error}")))?;
        let mut session = Self {
            child,
            log,
            stopped: false,
        };
        session.wait_until_serving(&directory.join(DISPLAY))?;
        Ok(session)
    }

    fn wait_until_serving(&mut self, socket: &Path) -> Result<(), TargetError> {
        let deadline = Instant::now() + START;
        loop {
            if UnixStream::connect(socket).is_ok() {
                return Ok(());
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                // The launcher is reaped, so its process group id is no longer ours to signal.
                self.stopped = true;
                return Err(live(format!(
                    "the launcher exited with {status} before it served {}\n{}",
                    socket.display(),
                    self.tail()
                )));
            }
            if Instant::now() >= deadline {
                let tail = self.tail();
                self.stop();
                return Err(live(format!(
                    "{} never accepted a connection\n{tail}",
                    socket.display()
                )));
            }
            std::thread::sleep(POLL);
        }
    }

    /// The last lines Mutter logged, for an error message.
    fn tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(20);
        format!("mutter log (last lines):\n{}", lines[start..].join("\n"))
    }

    /// Adds the Mutter log to an error from a probe, so a refusal is explained by what the compositor said.
    fn explain(&self, error: TargetError) -> TargetError {
        match error {
            TargetError::Live { target, detail } => TargetError::Live {
                target,
                detail: format!("{detail}\n{}", self.tail()),
            },
            other => other,
        }
    }

    /// Kills the launcher's process group and waits for the launcher to be gone.
    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        if let Some(pid) = i32::try_from(self.child.id()).ok().and_then(Pid::from_raw) {
            let _ = kill_process_group(pid, Signal::TERM);
        }
        let deadline = Instant::now() + STOP;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(POLL);
        }
        if let Some(pid) = i32::try_from(self.child.id()).ok().and_then(Pid::from_raw) {
            let _ = kill_process_group(pid, Signal::KILL);
        }
        let _ = self.child.wait();
    }
}

impl Drop for MutterSession {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
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
