//! The guardian role: the performer of the policy's effects.
//!
//! [`crate::Guardian`] is the session's whole policy as a pure state machine — a plan of
//! children, [`Event`]s in, [`Effect`]s out — and it owns no process, socket, timer or file.
//! This module is the other half, and it is the only place in the crate that touches the
//! operating system: it spawns the children, signals and kills their process groups, reads
//! their reports, writes the recovery state and turns the session manager's own signals into
//! events.
//!
//! # What a child gets
//!
//! `gnoblin --role <role>`, started with no shell, in a process group of its own so a stop
//! reaches its helpers, with the environment [`Effect::Spawn`] names. Two descriptors beyond
//! the standard three, when the child has a use for them:
//!
//! - its end of the privileged IPC pair ([`gnoblin_proto::privilege`]), which the
//!   compositor's other end grants it a capability on;
//! - the report descriptor it writes readiness and configuration revisions on
//!   ([`gnoblin_proto::report`]).
//!
//! Both are ordinary descriptors whose numbers travel in the environment. `FD_CLOEXEC` is
//! cleared for one descriptor and one `spawn` at a time and restored immediately after,
//! which is what keeps the runtime's connection out of the services role: nothing else in
//! this process ever has an inheritable descriptor, and the parent's copy of each end is
//! closed once the child has it.
//!
//! # Why the loop ticks
//!
//! The policy is caller-driven: nothing restarts, expires or escalates without an
//! [`EventKind::Tick`]. The loop therefore waits for an event with the tick as its timeout,
//! which is also what makes an idle session cost one wake-up per tick and nothing else.
//!
//! # Failure modes first
//!
//! 1. **A child that crashes immediately is restarted forever.** The policy's ladder and
//!    circuit breaker bound it, the guardian reports the session degraded, and the loop keeps
//!    running: `tests/guardian.rs::a_crash_looping_runtime_is_cut_off_and_reported`.
//! 2. **A compositor crash leaves orphans.** Its exit begins the stop sequence, which signals
//!    and then kills every other child, and the guardian exits non-zero once they are gone:
//!    `tests/guardian.rs::a_compositor_crash_stops_every_child`.
//! 3. **A descriptor leaks into an unrelated child.** Only the descriptor a child is given is
//!    inheritable for that spawn, and the parent closes its copy afterwards, so the services
//!    role never holds the runtime's connection:
//!    `tests/guardian.rs::a_privileged_descriptor_reaches_only_its_own_role`.
//! 4. **The guardian's own `SIGTERM` is never noticed.** It becomes an event, the stop
//!    sequence runs in reverse start order, and the process exits zero once every child is
//!    gone: `tests/guardian.rs::a_termination_signal_stops_the_session_in_order`.
//! 5. **A child that cannot be started stalls the session.** The failure is reported to the
//!    policy as an exit, so its own restart policy decides instead of the session waiting for
//!    a readiness report that will never come: `tests/guardian.rs::a_child_that_cannot_start_is_reported`.

use std::collections::BTreeMap;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use gnoblin_proto::{privilege, report};
use rustix::io::FdFlags;
use rustix::process::{Pid, Signal as ProcessSignal};

use crate::effect::{Effect, Level};
use crate::environment::{EnvTarget, Environment, Inherited};
use crate::event::{Event, EventKind, ExitStatus, ShutdownReason};
use crate::guardian::{Guardian, Rules};
use crate::ids::ChildId;
use crate::plan::{Backoff, ChildKind, ChildSpec, Plan, Restart};
use crate::recovery::{ConfigStatus, Recovery};
use crate::state::{SessionState, StopReason};
use crate::time::{Tick, secs};

/// A test-only hook: shrinks every deadline so a real-process test can cross one.
///
/// The guardian's shipped numbers are the C supervisor's — a 250 ms first restart, a 4 s
/// ladder and a 2 s stop grace — because that is what a person's desktop needs. A test that
/// had to wait twenty seconds to see the circuit breaker trip would be a test nobody runs, so
/// setting this variable makes the tick, the stop grace and the restart ladders small. It is
/// read once, in [`SupervisorConfig::from_environment`], and nothing else in the workspace looks at it.
pub const TEST_FAST_VARIABLE: &str = "GNOBLIN_TEST_FAST";

/// How the guardian role runs.
pub struct SupervisorConfig {
    /// The executable to start the children from, which is this binary.
    pub program: PathBuf,
    /// The children to supervise.
    pub plan: Plan,
    /// How the guardian behaves: the stop grace, the safe-mode threshold, the environment.
    pub rules: Rules,
    /// The recovery state the last session left.
    pub recovery: Recovery,
    /// The file the recovery state is written to.
    pub recovery_path: PathBuf,
    /// How often the loop tells the policy that time passed.
    pub tick: Duration,
}

impl SupervisorConfig {
    /// The configuration the process finds: the standard plan, the C supervisor's numbers and
    /// the recovery state under `$XDG_STATE_HOME/gnoblin`.
    ///
    /// # Errors
    ///
    /// [`SupervisorError::Plan`] when the standard plan is not a valid plan, which would mean the crate
    /// and its own defaults disagree.
    pub fn from_environment(program: PathBuf) -> Result<SupervisorConfig, SupervisorError> {
        let prefix = std::env::var("GNOBLIN_PREFIX").unwrap_or_else(|_| "/usr".to_owned());
        let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
        let library_path = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
        let inherited = Inherited {
            prefix: &prefix,
            path: &path,
            library_path: &library_path,
            data_dirs: &data_dirs,
        };
        let mut plan = Plan::standard(false, Vec::new()).map_err(SupervisorError::Plan)?;
        let mut rules = Rules::standard(&inherited);
        let mut tick = Duration::from_millis(200);
        if std::env::var_os(TEST_FAST_VARIABLE).is_some() {
            plan = fast_plan(&plan)?;
            rules.stop_grace = Tick::new(200);
            tick = Duration::from_millis(10);
        }
        let recovery_path = state_home().join("gnoblin").join("recovery");
        Ok(SupervisorConfig {
            program,
            plan,
            rules,
            recovery: read_recovery(&recovery_path),
            recovery_path,
            tick,
        })
    }
}

/// Why the guardian role could not run.
#[derive(Debug)]
pub enum SupervisorError {
    /// The plan the role supervises is not a valid plan.
    Plan(crate::plan::PlanError),
    /// The session's own signals could not be observed.
    Signals(std::io::Error),
    /// A descriptor pair could not be created.
    Descriptors(std::io::Error),
}

impl std::fmt::Display for SupervisorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupervisorError::Plan(error) => write!(formatter, "the session plan is not usable: {error}"),
            SupervisorError::Signals(error) => write!(formatter, "the session's signals cannot be observed: {error}"),
            SupervisorError::Descriptors(error) => write!(formatter, "cannot create a descriptor pair: {error}"),
        }
    }
}

impl std::error::Error for SupervisorError {}

/// What the loop is waiting for.
enum LoopWake {
    /// Something the policy has to answer.
    Event(Event),
    /// The session manager asked the session to end.
    Signal,
}

/// One child the guardian has started.
struct RunningChild {
    /// The process group to signal: the child was started in a group of its own, so the
    /// group id is its pid and a helper it started goes with it.
    pid: i32,
}

/// The process supervisor.
struct Supervisor {
    config: SupervisorConfig,
    guardian: Guardian,
    /// The environment every child inherits before [`Effect::Spawn`] adds to it: this
    /// process's own, so `XDG_RUNTIME_DIR`, `HOME` and the bus address reach the session.
    base: Environment,
    /// The children that are alive, by plan slot.
    running: BTreeMap<ChildId, RunningChild>,
    /// The child ends of the privileged pairs, waiting for the child that owns each.
    pending: BTreeMap<String, OwnedFd>,
    /// The compositor's ends of those pairs, until the compositor starts.
    compositor: Vec<(String, OwnedFd)>,
    /// The events the children and the signal handler produce.
    sender: Sender<LoopWake>,
    receiver: Receiver<LoopWake>,
    /// The clock every event is stamped with.
    start: Instant,
}

impl Supervisor {
    /// The current tick, measured from the session's start.
    fn now(&self) -> Tick {
        Tick::new(u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    /// Perform one batch of effects, in order.
    fn perform(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Spawn {
                    child,
                    generation,
                    name,
                    environment,
                } => self.spawn(child, generation, &name, &environment),
                Effect::Signal { child, .. } => self.signal(child, ProcessSignal::TERM),
                Effect::Kill { child } => self.signal(child, ProcessSignal::KILL),
                Effect::Log { level, message } => log(level, &message),
                Effect::ExportEnv { target, environment } => match target {
                    EnvTarget::Session => apply(&mut self.base, &environment),
                    // The activation environment belongs to the D-Bus broker, and the
                    // guardian speaks no D-Bus. The variables still reach every child,
                    // because `EnvTarget::Session` is what a spawn applies.
                    EnvTarget::Activation => log(
                        Level::Debug,
                        &format!(
                            "[guardian] the activation environment is not exported by this build ({} variables)",
                            environment.set.len()
                        ),
                    ),
                },
                Effect::SetRecovery(recovery) => write_recovery(&self.config.recovery_path, &recovery),
            }
        }
    }

    /// Start one child, with the descriptors its role needs.
    fn spawn(&mut self, child: ChildId, generation: u64, name: &str, environment: &Environment) {
        let Some(spec) = self.guardian.plan().spec(child).cloned() else {
            log(
                Level::Warning,
                &format!(
                    "[guardian] asked to start child {} which the plan does not declare",
                    child.get()
                ),
            );
            return;
        };
        if spec.kind == ChildKind::Compositor
            && let Err(error) = self.open_pairs()
        {
            log(
                Level::Error,
                &format!("[guardian] cannot create the privileged connections: {error}"),
            );
        }
        let (reader, writer) = match readiness_pipe() {
            Ok(pipe) => pipe,
            Err(error) => {
                log(
                    Level::Error,
                    &format!("[guardian] {name}: cannot create its report descriptor: {error}"),
                );
                self.report(EventKind::Exited {
                    child,
                    generation,
                    status: ExitStatus::Exited(70),
                });
                return;
            }
        };
        let mut command = self.command(&spec);
        command
            .env_clear()
            .envs(self.base.set.iter().map(|(key, value)| (key, value)))
            .envs(environment.set.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .process_group(0);
        // The report descriptor, which every child gets.
        if let Ok(number) = inheritable(&writer) {
            command.env(report::READY_VARIABLE, number.to_string());
        }
        // The privileged connection, for the role that owns one. The pair stays open in this
        // process for the session's life: a restarted child presents a new handshake on the
        // same stream, which is the only way a child that is not the pair's creator can
        // rejoin it.
        let privileged = self.role_name(&spec).filter(|role| self.pending.contains_key(role));
        if let Some(role) = &privileged
            && let Some(fd) = self.pending.get(role)
            && let Ok(number) = inheritable(fd)
        {
            command.env(privilege::FD_VARIABLE, number.to_string());
        }
        // The compositor receives the other end of every pair, one variable per role.
        let mut owners = Vec::new();
        if spec.kind == ChildKind::Compositor {
            for (role, fd) in &self.compositor {
                if let Ok(number) = inheritable(fd) {
                    owners.push((privilege::owner_variable(role), number.to_string()));
                }
            }
            command.envs(owners);
        }

        let started = command.spawn();

        // The window in which a descriptor is inheritable is this call and nothing else.
        // The parent's copies then go entirely: an end this process kept would never report
        // the child's departure to the process at the other end of the pair.
        restore(&writer);
        drop(writer);
        if let Some(role) = &privileged
            && let Some(fd) = self.pending.get(role)
        {
            restore(fd);
        }
        if spec.kind == ChildKind::Compositor {
            for (_, fd) in self.compositor.drain(..) {
                restore(&fd);
                drop(fd);
            }
        }

        let process = match started {
            Ok(process) => process,
            Err(reason) => {
                log(Level::Error, &format!("[guardian] cannot start {name}: {reason}"));
                // The policy asked for a start and cannot have one. Reporting the exit is
                // what lets its restart policy decide, instead of the session waiting for a
                // readiness report that will never come.
                self.report(EventKind::Exited {
                    child,
                    generation,
                    status: ExitStatus::Exited(70),
                });
                return;
            }
        };
        let pid = process.id();
        log(
            Level::Info,
            &format!("[guardian] started {name} (generation {generation}, pid {pid})"),
        );
        let events = self.sender.clone();
        let start = self.start;
        let spawned = std::thread::Builder::new()
            .name(format!("gnoblin-{name}"))
            .spawn(move || supervise_child(reader, process, child, generation, events, start));
        match spawned {
            Ok(_) => {
                self.running.insert(child, RunningChild { pid: pid as i32 });
            }
            Err(error) => log(
                Level::Error,
                &format!("[guardian] {name}: its report thread could not be started: {error}"),
            ),
        }
    }

    /// The command that starts one child.
    fn command(&self, spec: &ChildSpec) -> Command {
        match spec.kind {
            ChildKind::Autostart => Command::new(&spec.name),
            ChildKind::Compositor | ChildKind::Runtime | ChildKind::Services | ChildKind::Greeter => {
                let mut command = Command::new(&self.config.program);
                let role = self.role_name(spec).unwrap_or_else(|| "guardian".to_owned());
                command.arg("--role").arg(role);
                command
            }
        }
    }

    /// The role word a planned child is started as.
    fn role_name(&self, spec: &ChildSpec) -> Option<String> {
        match spec.kind {
            ChildKind::Compositor => Some("compositor".to_owned()),
            ChildKind::Runtime => Some("runtime".to_owned()),
            ChildKind::Services => Some("services".to_owned()),
            ChildKind::Greeter => Some("greeter".to_owned()),
            ChildKind::Autostart => None,
        }
    }

    /// Create one privileged pair per privileged child, keeping both ends for now.
    fn open_pairs(&mut self) -> Result<(), SupervisorError> {
        for spec in self.config.plan.children() {
            let Some(role) = self.role_name(spec) else {
                continue;
            };
            if !privileged_role(&role) {
                continue;
            }
            let (compositor_end, child_end) = rustix::net::socketpair(
                rustix::net::AddressFamily::UNIX,
                rustix::net::SocketType::STREAM,
                rustix::net::SocketFlags::CLOEXEC,
                None,
            )
            .map_err(|error| SupervisorError::Descriptors(error.into()))?;
            self.compositor.push((role.clone(), compositor_end));
            self.pending.insert(role, child_end);
        }
        Ok(())
    }

    /// Signal one child's process group.
    fn signal(&mut self, child: ChildId, signal: ProcessSignal) {
        let Some(running) = self.running.get(&child) else {
            return;
        };
        let Some(group) = Pid::from_raw(running.pid) else {
            return;
        };
        if let Err(error) = rustix::process::kill_process_group(group, signal) {
            log(
                Level::Debug,
                &format!("[guardian] cannot signal child {}: {error}", child.get()),
            );
        }
    }

    /// Send one event to the loop's queue.
    fn report(&self, kind: EventKind) {
        let _ = self.sender.send(LoopWake::Event(Event::new(self.now(), kind)));
    }
}

/// Whether a role is one the compositor grants a capability to.
///
/// The guardian only creates a pair for a role that will be granted something, because a
/// pair created for nothing is a descriptor a child cannot use. The table itself is the
/// compositor's (`gnoblin-compositor`); this is the guardian's side of the same contract.
fn privileged_role(role: &str) -> bool {
    matches!(role, "runtime" | "services")
}

/// A plan whose restart ladders are short, for the test-only hook.
fn fast_plan(plan: &Plan) -> Result<Plan, SupervisorError> {
    let children: Vec<ChildSpec> = plan
        .children()
        .iter()
        .map(|child| {
            let mut child = child.clone();
            if matches!(child.restart, Restart::Backoff(_)) {
                child.restart = Restart::Backoff(Backoff {
                    initial: Tick::new(20),
                    factor: 2,
                    max: Tick::new(80),
                    max_restarts: 3,
                    stable_after: secs(1),
                });
            }
            child
        })
        .collect();
    Plan::new(children).map_err(SupervisorError::Plan)
}

/// Read the recovery state the last session wrote, or a fresh one.
///
/// A file this build cannot read is treated as no state: it is a hint about the last
/// session, and refusing to start over it would make a corrupt file a session that never
/// starts.
fn read_recovery(path: &Path) -> Recovery {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Recovery::default();
    };
    let mut recovery = Recovery::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "failed_starts" => recovery.failed_starts = value.parse().unwrap_or(0),
            "safe_mode" => recovery.safe_mode = value == "1",
            "last_good" => recovery.config.last_good = value.parse().ok(),
            "status" if value == "accepted" => recovery.config.status = ConfigStatus::Accepted,
            _ => {}
        }
    }
    recovery
}

/// Write the recovery state, replacing the file in one step.
fn write_recovery(path: &Path, recovery: &Recovery) {
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        log(
            Level::Warning,
            &format!("[guardian] cannot create {}: {error}", parent.display()),
        );
        return;
    }
    let status = match &recovery.config.status {
        ConfigStatus::Idle => "idle",
        ConfigStatus::Accepted => "accepted",
        ConfigStatus::Rejected { .. } => "rejected",
        ConfigStatus::Defaults { .. } => "defaults",
    };
    let text = format!(
        "failed_starts={}\nsafe_mode={}\nlast_good={}\nstatus={status}\n",
        recovery.failed_starts,
        u8::from(recovery.safe_mode),
        recovery
            .config
            .last_good
            .map(|revision| revision.to_string())
            .unwrap_or_default(),
    );
    let temporary = path.with_extension("new");
    if let Err(error) = std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, path)) {
        log(
            Level::Warning,
            &format!("[guardian] cannot write {}: {error}", path.display()),
        );
    }
}

/// `$XDG_STATE_HOME`, or `$HOME/.local/state`.
fn state_home() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state);
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".local/state"),
        None => PathBuf::from("/tmp"),
    }
}

/// The environment this process was started with.
fn base_environment() -> Environment {
    let mut environment = Environment::new();
    for (key, value) in std::env::vars() {
        // A descriptor this process hands down is named by a variable that must not be
        // inherited by a child that was not given one.
        if key == privilege::FD_VARIABLE || key == report::READY_VARIABLE || key.starts_with(privilege::OWNER_PREFIX) {
            continue;
        }
        environment.set(key, value);
    }
    environment
}

/// Apply a set of changes to an environment.
fn apply(environment: &mut Environment, changes: &Environment) {
    for (key, value) in &changes.set {
        environment.set(key.clone(), value.clone());
    }
    for key in &changes.unset {
        environment.unset(key.clone());
    }
}

/// A pipe whose write end a child reports on.
///
/// Both ends are close-on-exec: only the one spawn that hands the number to a child clears
/// it, so a descriptor never reaches a process that was not given it.
fn readiness_pipe() -> std::io::Result<(std::fs::File, OwnedFd)> {
    let (reader, writer) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
    Ok((std::fs::File::from(reader), writer))
}

/// Make one descriptor inheritable, and say which number it is.
fn inheritable(fd: &OwnedFd) -> std::io::Result<i32> {
    rustix::io::fcntl_setfd(fd, FdFlags::empty())?;
    Ok(fd.as_raw_fd())
}

/// Put a descriptor back under close-on-exec.
fn restore(fd: &OwnedFd) {
    let _ = rustix::io::fcntl_setfd(fd, FdFlags::CLOEXEC);
}

/// Read one child's report stream, then reap it.
fn supervise_child(
    reader: std::fs::File,
    mut process: Child,
    child: ChildId,
    generation: u64,
    events: Sender<LoopWake>,
    start: Instant,
) {
    let tick = move || Tick::new(u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
    let mut lines = std::io::BufRead::lines(std::io::BufReader::new(reader));
    let mut ready = false;
    let mut environment: Vec<(String, String)> = Vec::new();
    while let Some(Ok(line)) = lines.next() {
        match report::parse(&line) {
            report::Line::Ready => {
                ready = true;
                environment.clear();
            }
            report::Line::Environment { key, value } => environment.push((key, value)),
            report::Line::End => {
                if ready {
                    let _ = events.send(LoopWake::Event(Event::new(
                        tick(),
                        EventKind::Ready {
                            child,
                            generation,
                            environment: std::mem::take(&mut environment),
                        },
                    )));
                    // A child reports ready once; a second block would be a bug in the
                    // child, and the policy ignores a duplicate readiness anyway.
                    ready = false;
                }
            }
            report::Line::ConfigAccepted { revision } => {
                let _ = events.send(LoopWake::Event(Event::new(
                    tick(),
                    EventKind::ConfigAccepted { revision },
                )));
            }
            report::Line::ConfigRejected { revision, diagnostic } => {
                let _ = events.send(LoopWake::Event(Event::new(
                    tick(),
                    EventKind::ConfigRejected { revision, diagnostic },
                )));
            }
            report::Line::Unknown => {}
        }
    }
    let status = match process.wait() {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => ExitStatus::Exited(code),
            (None, Some(signal)) if signal == SIGTERM => ExitStatus::Signalled(crate::event::Signal::Term),
            (None, Some(_)) => ExitStatus::Signalled(crate::event::Signal::Kill),
            (None, None) => ExitStatus::Exited(1),
        },
        Err(_) => ExitStatus::Exited(1),
    };
    let _ = events.send(LoopWake::Event(Event::new(
        tick(),
        EventKind::Exited {
            child,
            generation,
            status,
        },
    )));
}

/// `SIGTERM`, which a child killed by a graceful stop reports as its signal.
const SIGTERM: i32 = 15;

/// One line on the guardian's log.
///
/// The guardian's log is its standard error, because that is where a session manager reads
/// it and there is no second destination to configure. The message already starts with
/// `[guardian]`, per the redaction rule in `design/startup-recovery.md`.
fn log(level: Level, message: &str) {
    let prefix = match level {
        Level::Debug => "debug",
        Level::Info => "info",
        Level::Warning => "warning",
        Level::Error => "error",
    };
    eprintln!("{prefix}: {message}");
}

/// Run the guardian role until the session ends.
///
/// Returns the exit status the process should have: zero for an orderly end, one when the
/// compositor was lost or the session never became usable.
///
/// # Errors
///
/// [`SupervisorError`] when the role cannot start at all, which is the case a session manager reports
/// rather than retries.
pub fn run(config: SupervisorConfig) -> Result<u8, SupervisorError> {
    let base = base_environment();
    let guardian = Guardian::with_recovery(config.plan.clone(), config.rules.clone(), config.recovery.clone());
    let (sender, receiver) = channel();
    let start = Instant::now();
    let mut supervisor = Supervisor {
        config,
        guardian,
        base,
        running: BTreeMap::new(),
        pending: BTreeMap::new(),
        compositor: Vec::new(),
        sender: sender.clone(),
        receiver,
        start,
    };
    watch_signals(&sender)?;
    supervisor.report(EventKind::Start);
    loop {
        // The tick interval bounds how long a shutdown request is unnoticed and how often
        // the policy is told time passed; both are this one wait.
        let wake = match supervisor.receiver.recv_timeout(supervisor.config.tick) {
            Ok(wake) => wake,
            Err(RecvTimeoutError::Timeout) => LoopWake::Event(Event::new(supervisor.now(), EventKind::Tick)),
            // The supervisor holds a sender of its own, so this cannot happen; ending is the
            // only safe answer if it did.
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let event = match wake {
            LoopWake::Event(event) => event,
            LoopWake::Signal => Event::new(
                supervisor.now(),
                EventKind::Shutdown {
                    reason: ShutdownReason::Signal,
                },
            ),
        };
        let effects = supervisor.guardian.apply(event);
        supervisor.perform(effects);
        if supervisor.guardian.session() == SessionState::Ended {
            break;
        }
    }
    // Nothing outlives the session. The stop sequence signalled and then killed every child,
    // and this is the belt to that braces: whatever is still alive is killed and its exit is
    // waited for, so the guardian's own exit leaves no process group behind.
    let survivors: Vec<i32> = supervisor.running.values().map(|running| running.pid).collect();
    for pid in &survivors {
        if let Some(group) = Pid::from_raw(*pid) {
            let _ = rustix::process::kill_process_group(group, ProcessSignal::KILL);
        }
    }
    if !survivors.is_empty() {
        let deadline = Instant::now() + Duration::from_millis(supervisor.config.rules.stop_grace.get());
        while !supervisor.running.is_empty() && Instant::now() < deadline {
            match supervisor.receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(LoopWake::Event(Event {
                    kind: EventKind::Exited { child, .. },
                    ..
                })) => {
                    supervisor.running.remove(&child);
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }
    let reason = supervisor.guardian.stop_reason();
    log(
        Level::Info,
        &format!(
            "[guardian] session ended ({}); every child stopped",
            reason.map_or("unknown", StopReason::as_str)
        ),
    );
    Ok(match reason {
        Some(StopReason::Signal | StopReason::Logout) => 0,
        _ => 1,
    })
}

/// Turn the session manager's signals into one event.
fn watch_signals(events: &Sender<LoopWake>) -> Result<(), SupervisorError> {
    use signal_hook::consts::{SIGINT, SIGTERM as TERM};
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new([TERM, SIGINT]).map_err(SupervisorError::Signals)?;
    let events = events.clone();
    std::thread::Builder::new()
        .name("gnoblin-signals".to_owned())
        .spawn(move || {
            // One signal is enough: the first stop request begins the sequence, and the
            // policy ignores the rest.
            if signals.forever().next().is_some() {
                let _ = events.send(LoopWake::Signal);
            }
        })
        .map_err(SupervisorError::Signals)?;
    Ok(())
}
