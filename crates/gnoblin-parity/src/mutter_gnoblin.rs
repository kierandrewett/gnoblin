//! The `mutter-gnoblin` target: the reference session, Gnoblin's control API on a patched Mutter, headless.
//!
//! The target is two launchers the environment names, because the reference session is a Mutter build with a scripting
//! runtime that this crate neither builds nor ships:
//!
//! | Variable                            | Meaning                                                                          |
//! | ----------------------------------- | -------------------------------------------------------------------------------- |
//! | `GNOBLIN_PARITY_MUTTER_GNOBLIN`     | The session launcher: `<launcher> <runtime-dir> <display>`, as for `mutter51`.   |
//! | `GNOBLIN_PARITY_MUTTER_GNOBLIN_CTL` | The control client: `<ctl> <runtime-dir> <gnoblinctl arguments...>`. Default: `parity-gnoblinctl` beside the session launcher. |
//!
//! `scripts/parity-mutter-gnoblin` starts the reference guardian (`gnoblin --devkit --wayland-display <display>`), which
//! starts the patched Mutter on its headless backend with one virtual monitor, the scripting runtime and the control socket
//! `<runtime-dir>/compositor-v1.sock`. `scripts/parity-gnoblinctl` runs the reference `gnoblinctl` against that socket.
//! Both can run inside a root file system (`GNOBLIN_PARITY_MUTTER_ROOT`), because the build needs a distribution new
//! enough for Mutter 51.
//!
//! The target answers every probe the other live targets answer (the Wayland client is the same code and the EIS door
//! is the same `org.gnome.Mutter.RemoteDesktop`) and the probes that read Gnoblin's own API through the reference
//! `gnoblinctl`. Those observations are what the reference command prints, reduced to the same keys the candidate's
//! observation has where the two mean the same thing, and the reference command's own JSON where they do not (the
//! candidate's workspace list is an object model; the reference one is an array). A scenario records that as a divergence.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::launcher::{BUS_SOCKET, DISPLAY, LauncherSession, Names, private_directory};
use crate::probes::{self, Probes};
use crate::remote_desktop::RemoteDesktopDoor;
use crate::target::{Observations, RunEnvironment, Target, TargetError};
use crate::wayland_peer::{Toplevel, WaylandPeer};

/// The target name the report prints and the errors name.
const TARGET: &str = "mutter-gnoblin";
/// The environment variable that names the session launcher.
pub const LAUNCHER_VARIABLE: &str = "GNOBLIN_PARITY_MUTTER_GNOBLIN";
/// The environment variable that names the control client.
pub const CTL_VARIABLE: &str = "GNOBLIN_PARITY_MUTTER_GNOBLIN_CTL";
/// The control client's file name, looked for beside the session launcher when the variable is not set.
const CTL_DEFAULT: &str = "parity-gnoblinctl";
const NAMES: Names = Names {
    target: TARGET,
    log: "gnoblin",
};
/// How long a step waits for the session to reach the state it asks for.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How long the control socket has to answer after the Wayland socket is up: the scripting runtime starts after it.
const CONTROL_START: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
/// The app id and title of the toplevel the window steps create, as the candidate's steps use.
const APP_ID: &str = "org.gnoblin.parity";
const WINDOW_TITLE: &str = "parity-window";
/// A window id no session hands out, for the refusal probe.
const UNKNOWN_WINDOW: &str = "9999999";

/// The steps that read Gnoblin's own API, which only this target and the candidate answer.
const API_STEPS: &[&str] = &["workspace.list", "window.open", "window.close", "window.kill.unknown"];

/// The two programs the target runs.
#[derive(Debug, Clone)]
pub struct MutterGnoblinLaunch {
    /// Starts the session.
    pub launcher: OsString,
    /// Runs the reference `gnoblinctl` against it.
    pub ctl: OsString,
}

impl MutterGnoblinLaunch {
    /// Reads the launchers from the environment of a run. The error names the variable to set.
    pub(crate) fn from_environment(environment: &RunEnvironment) -> Result<Self, String> {
        let launcher = environment
            .mutter_gnoblin_launcher
            .clone()
            .ok_or_else(|| format!("{LAUNCHER_VARIABLE} names no launcher"))?;
        let ctl = match &environment.mutter_gnoblin_ctl {
            Some(ctl) => ctl.clone(),
            None => {
                let beside = Path::new(&launcher).with_file_name(CTL_DEFAULT);
                if beside.is_file() {
                    beside.into_os_string()
                } else {
                    return Err(format!(
                        "{CTL_VARIABLE} names no control client and {} does not exist",
                        beside.display()
                    ));
                }
            }
        };
        Ok(Self { launcher, ctl })
    }
}

/// Runs scenarios against Gnoblin on the patched Mutter, started by the launchers the environment names.
#[derive(Debug)]
pub struct MutterGnoblinTarget {
    launch: MutterGnoblinLaunch,
    runtime_base: Option<PathBuf>,
}

impl MutterGnoblinTarget {
    /// A target that runs `launch`, making its runtime directory under `runtime_base` (the system temporary directory
    /// when absent).
    pub fn new(launch: MutterGnoblinLaunch, runtime_base: Option<PathBuf>) -> Self {
        Self { launch, runtime_base }
    }
}

fn live(detail: impl Into<String>) -> TargetError {
    TargetError::Live {
        target: TARGET.to_owned(),
        detail: detail.into(),
    }
}

impl Target for MutterGnoblinTarget {
    fn name(&self) -> &str {
        TARGET
    }

    fn observe(&self, scenario: &crate::scenario::Scenario) -> Result<Observations, TargetError> {
        if let Some(step) = scenario
            .steps
            .iter()
            .find(|step| !probes::handles(&step.id) && !API_STEPS.contains(&step.id.as_str()))
        {
            return Err(live(format!("no probe for step `{}`", step.id)));
        }
        let directory = private_directory(TARGET, self.runtime_base.as_deref())?;
        let mut session = LauncherSession::start(NAMES, &self.launch.launcher, &[], directory.path())?;
        let mut run = Run {
            ctl: Ctl {
                program: self.launch.ctl.clone(),
                runtime: directory.path().to_path_buf(),
            },
            socket: directory.path().join(DISPLAY),
            shared: None,
            window: None,
            bus: directory.path().join(BUS_SOCKET),
        };
        let observations = run.steps(scenario).map_err(|error| session.explain(error));
        // The clients leave before the session is stopped, so it sees them go.
        drop(run);
        session.stop();
        observations
    }
}

/// The control client of one session.
struct Ctl {
    program: OsString,
    runtime: PathBuf,
}

/// The result of one `gnoblinctl` run.
struct CtlOutput {
    code: Option<i32>,
    out: String,
    err: String,
}

impl Ctl {
    /// Runs `gnoblinctl` with `arguments` against the session.
    fn run(&self, arguments: &[&str]) -> Result<CtlOutput, TargetError> {
        let output = Command::new(&self.program)
            .arg(&self.runtime)
            .args(arguments)
            .output()
            .map_err(|error| live(format!("cannot start the control client {:?}: {error}", self.program)))?;
        Ok(CtlOutput {
            code: output.status.code(),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Runs `gnoblinctl --json` and reads its standard output as JSON. A non-zero exit fails the step.
    fn json(&self, arguments: &[&str]) -> Result<Value, TargetError> {
        let mut full = vec!["--json"];
        full.extend_from_slice(arguments);
        let output = self.run(&full)?;
        if output.code != Some(0) {
            return Err(live(format!(
                "`gnoblinctl {}` exited {:?}: {}",
                full.join(" "),
                output.code,
                output.err.trim_end()
            )));
        }
        serde_json::from_str(output.out.trim_end()).map_err(|error| {
            live(format!(
                "`gnoblinctl {}` printed no JSON: {error}\n{}",
                full.join(" "),
                output.out
            ))
        })
    }

    /// Waits until the control socket answers `ping`, which is after the scripting runtime has started.
    fn wait_until_answering(&self) -> Result<(), TargetError> {
        let deadline = Instant::now() + CONTROL_START;
        loop {
            let output = self.run(&["ping"])?;
            if output.code == Some(0) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(live(format!(
                    "the control socket never answered `ping`; the last answer was exit {:?}: {}",
                    output.code,
                    output.err.trim_end()
                )));
            }
            std::thread::sleep(POLL);
        }
    }
}

/// One scenario's session: the clients and the toplevel the window steps keep between them.
struct Run {
    ctl: Ctl,
    socket: PathBuf,
    bus: PathBuf,
    /// The client of the shared focus, popup and input probes, connected by the first of them.
    shared: Option<Probes>,
    /// The client and toplevel of the window steps.
    window: Option<(WaylandPeer, Option<Toplevel>)>,
}

impl Run {
    fn steps(&mut self, scenario: &crate::scenario::Scenario) -> Result<Observations, TargetError> {
        self.ctl.wait_until_answering()?;
        let mut observations = Observations::new();
        for step in &scenario.steps {
            let value = self.probe(&step.id)?;
            observations.insert(step.id.clone(), value);
        }
        Ok(observations)
    }

    fn probe(&mut self, step: &str) -> Result<Value, TargetError> {
        match step {
            "workspace.list" => self.ctl.json(&["workspace", "list"]),
            "window.open" => self.open_window(),
            "window.close" => self.close_window(),
            "window.kill.unknown" => self.kill_unknown_window(),
            shared if probes::handles(shared) => {
                if self.shared.is_none() {
                    let door = RemoteDesktopDoor::new(TARGET, self.bus.clone());
                    self.shared = Some(Probes::connect(TARGET, &self.socket, Some(Box::new(door)))?);
                }
                self.shared
                    .as_mut()
                    .ok_or_else(|| live("the shared probes are not connected"))?
                    .run(shared)
            }
            other => Err(live(format!("no probe for step `{other}`"))),
        }
    }

    /// The windows `gnoblinctl window list` reports, as `(app id, title, focused)`.
    fn windows(&self) -> Result<Vec<(String, String, bool)>, TargetError> {
        let reply = self.ctl.json(&["window", "list"])?;
        let list = match &reply {
            Value::Array(list) => list.as_slice(),
            Value::Object(object) => object
                .get("windows")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .ok_or_else(|| live(format!("`window list` reply has no `windows`: {reply}")))?,
            other => return Err(live(format!("`window list` reply is not a list: {other}"))),
        };
        let text = |window: &Value, keys: &[&str]| {
            keys.iter()
                .find_map(|key| window.get(*key).and_then(Value::as_str))
                .unwrap_or_default()
                .to_owned()
        };
        Ok(list
            .iter()
            .map(|window| {
                (
                    text(window, &["app", "app_id"]),
                    text(window, &["title"]),
                    window.get("focused").and_then(Value::as_bool).unwrap_or(false),
                )
            })
            .collect())
    }

    /// Waits until the test window is listed (`present`) or gone.
    fn wait_for_window(&self, present: bool) -> Result<Vec<(String, String, bool)>, TargetError> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let windows = self.windows()?;
            if windows.iter().any(|(_, title, _)| title == WINDOW_TITLE) == present {
                return Ok(windows);
            }
            if Instant::now() >= deadline {
                let state = if present { "listed" } else { "unlisted" };
                return Err(live(format!("window `{WINDOW_TITLE}` never became {state}")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Maps a toplevel through a real Wayland client, then reports what `window list` shows for it.
    fn open_window(&mut self) -> Result<Value, TargetError> {
        let mut client = WaylandPeer::connect(TARGET, &self.socket)?;
        let window = client.map_toplevel(APP_ID, WINDOW_TITLE)?;
        self.window = Some((client, Some(window)));
        let windows = self.wait_for_window(true)?;
        let ours = windows.iter().find(|(_, title, _)| title == WINDOW_TITLE);
        Ok(json!({
            "listed": true,
            "app_id_listed": ours.is_some_and(|(app, _, _)| app == APP_ID),
            "focused": ours.is_some_and(|(_, _, focused)| *focused),
        }))
    }

    /// Destroys the toplevel the way a well-behaved client does, then reports whether the session still lists it.
    fn close_window(&mut self) -> Result<Value, TargetError> {
        let (client, window) = self
            .window
            .as_mut()
            .ok_or_else(|| live("`window.close` needs `window.open` first"))?;
        if let Some(window) = window.take() {
            client.destroy_toplevel(window)?;
        }
        let windows = self.wait_for_window(false)?;
        Ok(json!({ "listed": false, "list_empty": windows.is_empty() }))
    }

    /// Asks the session to kill a window that is not there. The refusal is the observation: the exit status and
    /// the message the command printed.
    fn kill_unknown_window(&self) -> Result<Value, TargetError> {
        let output = self.ctl.run(&["window", "kill", UNKNOWN_WINDOW])?;
        Ok(json!({ "exit": output.code, "stderr": output.err.trim_end() }))
    }
}
