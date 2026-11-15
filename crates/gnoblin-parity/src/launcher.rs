//! A compositor the environment names a launcher for: the process, its log and its teardown.
//!
//! `mutter51` and `mutter-gnoblin` both start a compositor this crate neither builds nor ships. The environment names a
//! program, the target calls it as `<launcher> <runtime-dir> <display>` with `XDG_RUNTIME_DIR` set to the runtime
//! directory, and the launcher stays in the foreground until the target kills its process group. This module is that
//! contract, once, so the two targets differ only in what they ask of the session afterwards.
//!
//! The launcher's own files go in the runtime directory, so the runner and the compositor agree on where they are
//! without a second channel: the Wayland socket is `<runtime-dir>/<display>` and the private D-Bus bus the launcher
//! starts listens at `<runtime-dir>/bus` (see [`BUS_SOCKET`]).

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};

use crate::target::TargetError;

/// The Wayland socket name every launcher is asked to serve.
pub(crate) const DISPLAY: &str = "parity-0";
/// The private session bus the launcher starts, as a file name in the runtime directory. It is how the runner reaches
/// `org.gnome.Mutter.RemoteDesktop` for input, since the launcher's bus address is otherwise private to its process
/// tree.
pub(crate) const BUS_SOCKET: &str = "bus";
/// How long a compositor has to start serving its socket.
const START: Duration = Duration::from_secs(90);
/// How long a compositor has to exit after the kill before the target gives up waiting.
const STOP: Duration = Duration::from_secs(10);
/// How often the target looks again.
const POLL: Duration = Duration::from_millis(20);

/// What a launched session is called in errors.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Names {
    /// The target name the report prints and the errors name.
    pub target: &'static str,
    /// What the compositor's log is called, as in `mutter log (last lines)`.
    pub log: &'static str,
}

/// A directory only this user can enter, as `$XDG_RUNTIME_DIR` is, made under `base` (the system temporary directory
/// when absent).
pub(crate) fn private_directory(target: &str, base: Option<&Path>) -> Result<tempfile::TempDir, TargetError> {
    let live = |detail: String| TargetError::Live {
        target: target.to_owned(),
        detail,
    };
    let mut builder = tempfile::Builder::new();
    builder.prefix("gnoblin-parity-");
    let directory = match base {
        Some(base) => builder.tempdir_in(base),
        None => builder.tempdir(),
    }
    .map_err(|error| live(format!("cannot create the runtime directory: {error}")))?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .map_err(|error| live(format!("cannot make the runtime directory private: {error}")))?;
    Ok(directory)
}

/// The launcher process and the log it writes.
#[derive(Debug)]
pub(crate) struct LauncherSession {
    names: Names,
    child: Child,
    log: PathBuf,
    /// Set once the launcher is gone, so a second `stop` never signals a process group that may belong to someone else.
    stopped: bool,
}

impl LauncherSession {
    /// Starts `launcher` with `leading` before its two arguments, in a process group of its own, and waits until its
    /// Wayland socket accepts. `leading` is empty for a launcher script; the candidate's launcher is this binary, and
    /// `leading` selects its `serve-candidate` command.
    pub(crate) fn start(
        names: Names,
        launcher: &OsStr,
        leading: &[&str],
        directory: &Path,
    ) -> Result<Self, TargetError> {
        let live = |detail: String| TargetError::Live {
            target: names.target.to_owned(),
            detail,
        };
        let log = directory.join("launcher.log");
        let log_file = std::fs::File::create(&log).map_err(|error| live(format!("cannot create the log: {error}")))?;
        let log_error = log_file
            .try_clone()
            .map_err(|error| live(format!("cannot share the log: {error}")))?;
        let child = Command::new(launcher)
            .args(leading)
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
            names,
            child,
            log,
            stopped: false,
        };
        session.wait_until_serving(&directory.join(DISPLAY))?;
        Ok(session)
    }

    fn live(&self, detail: String) -> TargetError {
        TargetError::Live {
            target: self.names.target.to_owned(),
            detail,
        }
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
                return Err(self.live(format!(
                    "the launcher exited with {status} before it served {}\n{}",
                    socket.display(),
                    self.tail()
                )));
            }
            if Instant::now() >= deadline {
                let tail = self.tail();
                self.stop();
                return Err(self.live(format!("{} never accepted a connection\n{tail}", socket.display())));
            }
            std::thread::sleep(POLL);
        }
    }

    /// The process group id of the launcher and everything it started: the compositor, its bus and its helpers.
    pub(crate) fn group(&self) -> u32 {
        self.child.id()
    }

    /// The last lines the compositor logged, for an error message.
    pub(crate) fn tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(20);
        format!("{} log (last lines):\n{}", self.names.log, lines[start..].join("\n"))
    }

    /// Adds the compositor's log to an error from a probe, so a refusal is explained by what the compositor said.
    pub(crate) fn explain(&self, error: TargetError) -> TargetError {
        match error {
            TargetError::Live { target, detail } => TargetError::Live {
                target,
                detail: format!("{detail}\n{}", self.tail()),
            },
            other => other,
        }
    }

    /// Kills the launcher's process group and waits for the launcher to be gone.
    pub(crate) fn stop(&mut self) {
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

impl Drop for LauncherSession {
    fn drop(&mut self) {
        self.stop();
    }
}
