//! Starts, watches, restarts and stops the runtime service child process.
//!
//! [`RuntimeManager`] owns at most one child. It creates the child through a
//! [`Spawner`] and observes it through a [`ChildProcess`]. Tests substitute a
//! fake; [`SystemSpawner`] is the real `std::process` implementation.
//!
//! Restart policy: a child that exits with failure is restarted, at most
//! [`MAX_RESTARTS`] times per [`RuntimeManager::start`]. A clean exit is final.
//! A spawn error during a restart is not returned from [`RuntimeManager::poll`];
//! it leaves the state at [`ChildState::Restarting`], and later polls retry it
//! against the same limit. Shutdown: [`RuntimeManager::stop`] asks the child to
//! terminate, waits for the grace period, then kills it. Dropping the manager
//! stops the child.

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::Runtime;

/// Restarts allowed per start before the manager gives up.
pub const MAX_RESTARTS: u32 = 3;

/// How long a child has to exit after a terminate request before it is killed.
const DEFAULT_GRACE: Duration = Duration::from_secs(5);

/// How often [`RuntimeManager::stop`] checks whether the child has exited.
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Creates runtime service child processes.
pub trait Spawner {
    /// The handle type returned for a started child.
    type Child: ChildProcess;

    /// Starts `program` with no arguments.
    fn spawn(&mut self, program: &Path) -> io::Result<Self::Child>;
}

/// A running child process, observed and signalled by [`RuntimeManager`].
pub trait ChildProcess {
    /// The operating-system process id.
    fn pid(&self) -> u32;

    /// Reaps the child if it has exited, returning how it exited.
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;

    /// Asks the child to exit (SIGTERM on Unix).
    fn terminate(&mut self) -> io::Result<()>;

    /// Forces the child to exit and reaps it (SIGKILL on Unix).
    fn kill(&mut self) -> io::Result<()>;
}

/// How a child process exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// Exited with status zero.
    Clean,
    /// Exited with a non-zero status, or was killed by a signal.
    Failed,
}

/// What the manager knows about its child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildState {
    /// No child has been started, or the last one was stopped.
    Idle,
    /// A child is running with this process id.
    Running { pid: u32 },
    /// A failed child is being replaced, and the last spawn attempt failed.
    Restarting { attempt: u32 },
    /// The child exited on its own. `clean` is true for a zero exit status.
    Exited { clean: bool },
    /// The child failed more often than [`MAX_RESTARTS`] allows.
    GaveUp { restarts: u32 },
}

/// Whether a usable OpenXR runtime is available to supervise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No OpenXR runtime was found. This is the failure mode the owner reports.
    NoRuntime,
    /// A runtime was found and can be used.
    Ready,
}

/// Owns at most one runtime service child and applies the restart policy.
pub struct RuntimeManager<S: Spawner> {
    spawner: S,
    program: PathBuf,
    grace: Duration,
    state: ChildState,
    child: Option<S::Child>,
    restarts: u32,
}

impl<S: Spawner> RuntimeManager<S> {
    /// A manager that will start `program` through `spawner`.
    pub fn new(spawner: S, program: PathBuf) -> Self {
        Self {
            spawner,
            program,
            grace: DEFAULT_GRACE,
            state: ChildState::Idle,
            child: None,
            restarts: 0,
        }
    }

    /// Sets how long [`RuntimeManager::stop`] waits after a terminate request.
    pub fn set_grace(&mut self, grace: Duration) {
        self.grace = grace;
    }

    /// The current child state.
    pub fn state(&self) -> ChildState {
        self.state
    }

    /// Starts the child. Does nothing if a child is already running or being restarted.
    pub fn start(&mut self) -> io::Result<()> {
        if matches!(self.state, ChildState::Running { .. } | ChildState::Restarting { .. }) {
            return Ok(());
        }
        self.restarts = 0;
        let child = self.spawner.spawn(&self.program)?;
        self.state = ChildState::Running { pid: child.pid() };
        self.child = Some(child);
        Ok(())
    }

    /// Checks the child once and applies the restart policy.
    pub fn poll(&mut self) -> io::Result<()> {
        match self.state {
            ChildState::Running { .. } => {
                let child = self.child.as_mut().expect("a running state always has a child");
                match child.try_wait()? {
                    None => {}
                    Some(ExitStatus::Clean) => {
                        self.child = None;
                        self.state = ChildState::Exited { clean: true };
                    }
                    Some(ExitStatus::Failed) => {
                        self.child = None;
                        self.restart_or_give_up();
                    }
                }
            }
            ChildState::Restarting { .. } => self.restart_or_give_up(),
            ChildState::Idle | ChildState::Exited { .. } | ChildState::GaveUp { .. } => {}
        }
        Ok(())
    }

    /// Stops the child: terminate, wait up to the grace period, then kill.
    pub fn stop(&mut self) -> io::Result<()> {
        self.state = ChildState::Idle;
        match self.child.take() {
            Some(mut child) => shut_down(&mut child, self.grace),
            None => Ok(()),
        }
    }

    /// Reports whether a runtime was found. Takes the runtime from lookup, which
    /// is the source of truth; the child's own state is not consulted.
    pub fn status(&self, runtime: Option<&Runtime>) -> Status {
        match runtime {
            Some(_) => Status::Ready,
            None => Status::NoRuntime,
        }
    }

    fn restart_or_give_up(&mut self) {
        if self.restarts >= MAX_RESTARTS {
            self.state = ChildState::GaveUp {
                restarts: self.restarts,
            };
            return;
        }
        self.restarts += 1;
        match self.spawner.spawn(&self.program) {
            Ok(child) => {
                self.state = ChildState::Running { pid: child.pid() };
                self.child = Some(child);
            }
            Err(_) => self.state = ChildState::Restarting { attempt: self.restarts },
        }
    }
}

impl<S: Spawner> Drop for RuntimeManager<S> {
    fn drop(&mut self) {
        // Drop cannot report an error. The child is still signalled and killed where possible.
        let _ = self.stop();
    }
}

/// Terminates `child`, waiting up to `grace` for it to exit, then kills it.
/// Returns the terminate error if there was one, after killing the child.
fn shut_down<C: ChildProcess>(child: &mut C, grace: Duration) -> io::Result<()> {
    let asked = child.terminate();
    let deadline = Instant::now() + grace;
    while asked.is_ok() {
        match child.try_wait() {
            Ok(Some(_)) => return asked,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }
    let killed = child.kill();
    asked.and(killed)
}

/// The real spawner: starts the program with `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSpawner;

/// A child started by [`SystemSpawner`].
#[derive(Debug)]
pub struct SystemChild {
    child: std::process::Child,
}

impl Spawner for SystemSpawner {
    type Child = SystemChild;

    fn spawn(&mut self, program: &Path) -> io::Result<SystemChild> {
        std::process::Command::new(program)
            .spawn()
            .map(|child| SystemChild { child })
    }
}

impl ChildProcess for SystemChild {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?.map(|status| {
            if status.success() {
                ExitStatus::Clean
            } else {
                ExitStatus::Failed
            }
        }))
    }

    fn terminate(&mut self) -> io::Result<()> {
        use rustix::process::{Pid, Signal, kill_process};
        let raw = i32::try_from(self.child.id()).map_err(io::Error::other)?;
        let pid = Pid::from_raw(raw).ok_or_else(|| io::Error::other("child pid is zero"))?;
        kill_process(pid, Signal::TERM).map_err(io::Error::from)
    }

    fn kill(&mut self) -> io::Result<()> {
        self.child.kill()?;
        self.child.wait().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    /// Shared script the fake spawner and fake children read from.
    #[derive(Default)]
    struct Script {
        /// Each spawn pops the next entry: `Ok(pid)` or `Err(message)`.
        spawns: Vec<Result<u32, String>>,
        /// Each poll pops the next entry: `None` (still running) or `Some(status)`.
        polls: Vec<Option<ExitStatus>>,
        spawn_count: usize,
        terminated: Vec<u32>,
        killed: Vec<u32>,
        /// When true, `terminate` does not end the child, so `kill` is needed.
        ignore_terminate: bool,
    }

    #[derive(Clone, Default)]
    struct FakeSpawner {
        script: Rc<RefCell<Script>>,
    }

    struct FakeChild {
        pid: u32,
        script: Rc<RefCell<Script>>,
        exited: Option<ExitStatus>,
    }

    impl Spawner for FakeSpawner {
        type Child = FakeChild;

        fn spawn(&mut self, _program: &Path) -> io::Result<FakeChild> {
            let mut script = self.script.borrow_mut();
            script.spawn_count += 1;
            match script.spawns.remove(0) {
                Ok(pid) => Ok(FakeChild {
                    pid,
                    script: Rc::clone(&self.script),
                    exited: None,
                }),
                Err(message) => Err(io::Error::other(message)),
            }
        }
    }

    impl ChildProcess for FakeChild {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            if let Some(status) = self.exited {
                return Ok(Some(status));
            }
            let mut script = self.script.borrow_mut();
            if script.polls.is_empty() {
                return Ok(None);
            }
            let next = script.polls.remove(0);
            self.exited = next;
            Ok(next)
        }

        fn terminate(&mut self) -> io::Result<()> {
            let mut script = self.script.borrow_mut();
            script.terminated.push(self.pid);
            if !script.ignore_terminate {
                self.exited.get_or_insert(ExitStatus::Clean);
            }
            Ok(())
        }

        fn kill(&mut self) -> io::Result<()> {
            let mut script = self.script.borrow_mut();
            script.killed.push(self.pid);
            self.exited.get_or_insert(ExitStatus::Failed);
            Ok(())
        }
    }

    fn manager(script: Script) -> (RuntimeManager<FakeSpawner>, Rc<RefCell<Script>>) {
        let script = Rc::new(RefCell::new(script));
        let spawner = FakeSpawner {
            script: Rc::clone(&script),
        };
        let mut manager = RuntimeManager::new(spawner, PathBuf::from("/usr/bin/monado-service"));
        manager.set_grace(Duration::ZERO);
        (manager, script)
    }

    fn runtime(root: &TempDir) -> crate::Runtime {
        crate::Runtime {
            name: None,
            library_path: PathBuf::from("/opt/libopenxr.so"),
            source: crate::Source::Global,
            manifest_path: root.join("active_runtime.json"),
        }
    }

    #[test]
    fn a_new_manager_is_idle() {
        let (manager, _) = manager(Script::default());

        assert_eq!(manager.state(), ChildState::Idle);
    }

    #[test]
    fn start_spawns_once_and_reports_running_with_the_pid() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(4242)],
            ..Script::default()
        });

        manager.start().expect("start");

        assert_eq!(manager.state(), ChildState::Running { pid: 4242 });
        assert_eq!(script.borrow().spawn_count, 1);
    }

    #[test]
    fn starting_a_running_manager_does_not_spawn_again() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(1)],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.start().expect("second start is a no-op");

        assert_eq!(script.borrow().spawn_count, 1);
    }

    #[test]
    fn clean_exit_is_reported_and_not_restarted() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(10)],
            polls: vec![Some(ExitStatus::Clean)],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.poll().expect("poll");

        assert_eq!(manager.state(), ChildState::Exited { clean: true });
        assert_eq!(script.borrow().spawn_count, 1);
    }

    #[test]
    fn failed_exit_is_restarted_at_most_three_times_then_gives_up() {
        // One start, then each failed child is replaced: three restarts, then the
        // fourth failure gives up without another spawn.
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(1), Ok(2), Ok(3), Ok(4)],
            polls: vec![
                Some(ExitStatus::Failed),
                Some(ExitStatus::Failed),
                Some(ExitStatus::Failed),
                Some(ExitStatus::Failed),
            ],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.poll().expect("poll 1");
        assert_eq!(manager.state(), ChildState::Running { pid: 2 });
        manager.poll().expect("poll 2");
        assert_eq!(manager.state(), ChildState::Running { pid: 3 });
        manager.poll().expect("poll 3");
        assert_eq!(manager.state(), ChildState::Running { pid: 4 });
        manager.poll().expect("poll 4");

        assert_eq!(manager.state(), ChildState::GaveUp { restarts: 3 });
        assert_eq!(script.borrow().spawn_count, 4, "one start plus three restarts");
    }

    #[test]
    fn a_failed_spawn_during_restart_is_retried_on_later_polls_within_the_limit() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(1), Err("exec failed".to_owned())],
            polls: vec![Some(ExitStatus::Failed)],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.poll().expect("poll");

        assert_eq!(manager.state(), ChildState::Restarting { attempt: 1 });
        assert_eq!(script.borrow().spawn_count, 2);
    }

    #[test]
    fn repeated_spawn_failures_give_up_after_three_restarts() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(1), Err("e".to_owned()), Err("e".to_owned()), Err("e".to_owned())],
            polls: vec![Some(ExitStatus::Failed)],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.poll().expect("first failure restarts");
        manager.poll().expect("second");
        manager.poll().expect("third");

        assert_eq!(manager.state(), ChildState::Restarting { attempt: 3 });
        manager.poll().expect("fourth gives up");
        assert_eq!(manager.state(), ChildState::GaveUp { restarts: 3 });
        assert_eq!(script.borrow().spawn_count, 4);
    }

    #[test]
    fn stop_terminates_then_reports_idle() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(77)],
            ..Script::default()
        });
        manager.start().expect("start");

        manager.stop().expect("stop");

        assert_eq!(manager.state(), ChildState::Idle);
        assert_eq!(script.borrow().terminated, vec![77]);
        assert!(
            script.borrow().killed.is_empty(),
            "a child that exits on SIGTERM is not killed"
        );
    }

    #[test]
    fn stop_kills_a_child_that_ignores_terminate_after_the_grace_period() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(88)],
            ignore_terminate: true,
            ..Script::default()
        });
        manager.start().expect("start");

        manager.stop().expect("stop");

        assert_eq!(script.borrow().terminated, vec![88]);
        assert_eq!(script.borrow().killed, vec![88]);
        assert_eq!(manager.state(), ChildState::Idle);
    }

    #[test]
    fn dropping_a_running_manager_shuts_the_child_down() {
        let (mut manager, script) = manager(Script {
            spawns: vec![Ok(99)],
            ..Script::default()
        });
        manager.start().expect("start");

        drop(manager);

        assert_eq!(script.borrow().terminated, vec![99]);
    }

    #[test]
    fn status_is_no_runtime_when_no_runtime_was_found() {
        let (manager, _) = manager(Script::default());

        assert_eq!(manager.status(None), Status::NoRuntime);
    }

    #[test]
    fn status_is_ready_when_a_runtime_was_found() {
        let root = TempDir::new("status-ready");
        let (manager, _) = manager(Script::default());

        assert_eq!(manager.status(Some(&runtime(&root))), Status::Ready);
    }

    #[test]
    fn system_spawner_reports_a_missing_program() {
        let mut spawner = SystemSpawner;

        let result = spawner.spawn(Path::new("/nonexistent/gnoblin-xr-test-binary"));

        assert!(result.is_err());
    }
}
