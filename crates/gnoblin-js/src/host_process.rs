//! `gnoblin.process` on the Rust side: the programs a script starts, and how each one ends.
//!
//! The runtime role owns the children, as the Lua host does: nothing but this process can reap them, so a
//! process is local to the shell and never crosses the owner socket. Each child gets one waiter thread that
//! owns it, polls it, enforces the timeout and reads what a captured stream holds. The shell drains the
//! waiters' channel once a frame ([`JsProcesses::drain`]), so the script's state is only ever touched on
//! the shell's thread.
//!
//! A process leaves [`JsProcesses`] when its end is drained. A kill of a process that is not there answers
//! `not-found` at once, so nothing waits on an end that can no longer arrive.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use serde::Deserialize;
use thiserror::Error;

/// Arguments the program may take, counting the program itself.
const MAX_ARGV: usize = 64;
/// Bytes the argument list may hold in total.
const MAX_ARGV_BYTES: usize = 64 * 1024;
/// Variables a spawn may add to the session environment.
const MAX_ENV: usize = 128;
/// Bytes written to standard input.
const MAX_STDIN: usize = 1024 * 1024;
/// The largest `outputLimit` a caller may ask for.
const MAX_OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
/// Bytes kept from a captured stream when `outputLimit` is not given.
const DEFAULT_OUTPUT_LIMIT: u64 = 64 * 1024;
/// The longest `timeout`, in seconds.
const MAX_TIMEOUT_SECONDS: f64 = 86_400.0;
/// Processes that may run at once. A config past this is looping, so the spawn is refused.
const MAX_RUNNING: usize = 64;
/// How long a timed-out process has after SIGTERM before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// How often a waiter polls its child.
const POLL: Duration = Duration::from_millis(5);
/// The read buffer for a captured stream.
const READ_CHUNK: usize = 8 * 1024;

/// The signal names `kill` accepts, with the signal each one sends.
const SIGNALS: &[(&str, Signal)] = &[
    ("term", Signal::TERM),
    ("kill", Signal::KILL),
    ("int", Signal::INT),
    ("hup", Signal::HUP),
    ("usr1", Signal::USR1),
    ("usr2", Signal::USR2),
    ("stop", Signal::STOP),
    ("cont", Signal::CONT),
];

/// Why a spawn or a kill was refused. The code is what the script's error carries.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct JsProcessError {
    /// `invalid_argument`, `not_found`, `busy` or `internal`, the codes `docs/shell-api/errors.md` lists.
    pub code: &'static str,
    /// What the script's author reads.
    pub message: String,
}

impl JsProcessError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// What one standard stream does with the program's output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JsStream {
    /// Writes to the runtime's own output, which the session log records.
    #[default]
    Inherit,
    /// Drops the output.
    Discard,
    /// Keeps the output, up to `outputLimit` bytes, for the end event.
    Capture,
}

impl JsStream {
    fn stdio(self) -> Stdio {
        match self {
            Self::Inherit => Stdio::inherit(),
            Self::Discard => Stdio::null(),
            Self::Capture => Stdio::piped(),
        }
    }
}

/// The options of a spawn as the script's module hands them over, checked against the limits above.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JsSpawnOptions {
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    stdin: Option<String>,
    #[serde(default)]
    stdout: JsStream,
    #[serde(default)]
    stderr: JsStream,
    output_limit: Option<u64>,
    timeout: Option<f64>,
}

/// How a process ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JsExit {
    /// The exit status after a normal exit.
    pub exit_code: Option<i32>,
    /// The name of the signal that ended it, after it was killed.
    pub signal: Option<String>,
    /// The `timeout` option ended it.
    pub timed_out: bool,
    /// What a captured standard output held, decoded as UTF-8 with replacement characters.
    pub stdout: Option<String>,
    /// What a captured standard error held.
    pub stderr: Option<String>,
    /// Standard output passed `outputLimit` and the rest was dropped.
    pub stdout_truncated: bool,
    /// Standard error passed `outputLimit` and the rest was dropped.
    pub stderr_truncated: bool,
}

/// A process that has started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JsStarted {
    /// The identifier the script holds.
    pub id: u64,
    /// The operating system's process id.
    pub pid: u32,
}

struct Running {
    pid: u32,
    /// True once the waiter has reaped the child, under the lock the waiter reaps with, so a signal is never
    /// sent to a process id the kernel may already have handed to something else.
    reaped: Arc<Mutex<bool>>,
}

/// The processes a shell started and has not seen end.
pub struct JsProcesses {
    next_id: u64,
    running: BTreeMap<u64, Running>,
    ends: Receiver<(u64, JsExit)>,
    sender: Sender<(u64, JsExit)>,
}

impl Default for JsProcesses {
    fn default() -> Self {
        let (sender, ends) = mpsc::channel();
        Self {
            next_id: 1,
            running: BTreeMap::new(),
            ends,
            sender,
        }
    }
}

/// The program, its arguments and the options, refused with the name of the thing that is wrong.
fn checked(argv: &[String], options: &JsSpawnOptions) -> Result<(), JsProcessError> {
    let invalid = |message: String| Err(JsProcessError::new("invalid_argument", message));
    if argv.is_empty() || argv.len() > MAX_ARGV {
        return invalid(format!("argv holds 1 to {MAX_ARGV} strings, not {}", argv.len()));
    }
    if argv.iter().map(String::len).sum::<usize>() > MAX_ARGV_BYTES {
        return invalid(format!("argv holds at most {MAX_ARGV_BYTES} bytes"));
    }
    if argv.iter().any(|part| part.contains('\0')) || argv[0].is_empty() {
        return invalid("argv strings are non-empty program names and arguments with no NUL".to_owned());
    }
    if let Some(cwd) = &options.cwd
        && !cwd.is_absolute()
    {
        return invalid(format!("cwd must be an absolute path, not {}", cwd.display()));
    }
    if options.env.len() > MAX_ENV {
        return invalid(format!("env holds at most {MAX_ENV} variables"));
    }
    if let Some(name) = options
        .env
        .iter()
        .flat_map(|(name, value)| [name, value])
        .find(|text| text.contains('\0'))
    {
        return invalid(format!("env holds a NUL in {name:?}"));
    }
    if let Some(name) = options.env.keys().find(|name| name.is_empty() || name.contains('=')) {
        return invalid(format!("env names are non-empty and have no `=`, not {name:?}"));
    }
    if options.stdin.as_ref().is_some_and(|text| text.len() > MAX_STDIN) {
        return invalid(format!("stdin holds at most {MAX_STDIN} bytes"));
    }
    if options.output_limit.is_some_and(|limit| limit > MAX_OUTPUT_LIMIT) {
        return invalid(format!("outputLimit is at most {MAX_OUTPUT_LIMIT}"));
    }
    if let Some(seconds) = options.timeout
        && !(seconds > 0.0 && seconds <= MAX_TIMEOUT_SECONDS)
    {
        return invalid(format!("timeout is more than 0 and at most {MAX_TIMEOUT_SECONDS} seconds"));
    }
    Ok(())
}

/// The name of the signal that ended a process, as `kill` spells it.
fn signal_name(number: i32) -> String {
    SIGNALS
        .iter()
        .find(|(_, signal)| signal.as_raw() == number)
        .map_or_else(|| format!("sig{number}"), |(name, _)| (*name).to_owned())
}

/// Reads `stream` to its end, keeping the first `limit` bytes. Output past the limit is read and dropped, so the
/// program never blocks on a full pipe.
fn capture(mut stream: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut chunk = [0_u8; READ_CHUNK];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return (kept, truncated),
            Ok(count) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..count.min(room)]);
                truncated |= count > room;
            }
        }
    }
}

/// Sends `signal` to the process group of `pid`.
fn signal_group(pid: u32, signal: Signal) {
    if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
        // The group may already be gone: a signal nobody receives is the answer the caller wanted.
        let _ = kill_process_group(pid, signal);
    }
}

/// Owns `child` until it ends: enforces the timeout, then reports how it ended.
fn wait_for(mut child: Child, reaped: &Mutex<bool>, timeout: Option<Duration>, readers: Readers) -> JsExit {
    let started = Instant::now();
    let pid = child.id();
    let mut timed_out = false;
    let mut kill_at: Option<Instant> = None;
    let status = loop {
        {
            let Ok(mut done) = reaped.lock() else {
                return JsExit::default();
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    *done = true;
                    break Some(status);
                }
                Ok(None) => {}
                Err(_) => {
                    *done = true;
                    break None;
                }
            }
        }
        let now = Instant::now();
        if !timed_out && timeout.is_some_and(|limit| now.duration_since(started) >= limit) {
            timed_out = true;
            signal_group(pid, Signal::TERM);
            kill_at = Some(now + KILL_GRACE);
        }
        if kill_at.is_some_and(|at| now >= at) {
            signal_group(pid, Signal::KILL);
            kill_at = None;
        }
        std::thread::sleep(POLL);
    };
    let (stdout, stdout_truncated) = readers.stdout.map_or((None, false), |reader| {
        let (bytes, truncated) = reader.join().unwrap_or_default();
        (Some(String::from_utf8_lossy(&bytes).into_owned()), truncated)
    });
    let (stderr, stderr_truncated) = readers.stderr.map_or((None, false), |reader| {
        let (bytes, truncated) = reader.join().unwrap_or_default();
        (Some(String::from_utf8_lossy(&bytes).into_owned()), truncated)
    });
    JsExit {
        exit_code: status.and_then(|status| status.code()),
        signal: status.and_then(|status| status.signal()).map(signal_name),
        timed_out,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    }
}

/// The threads that read a captured stream to its end.
struct Readers {
    stdout: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>,
    stderr: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>,
}

impl JsProcesses {
    /// How many processes are running.
    pub fn running(&self) -> usize {
        self.running.len()
    }

    /// Starts `argv` in a process group of its own, with `options` over the session's environment.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for a limit or a malformed option, `busy` at 64 running processes, and `not_found` or
    /// `internal` when the operating system refuses to start the program, with its reason.
    pub fn spawn(&mut self, argv: &[String], options: &JsSpawnOptions) -> Result<JsStarted, JsProcessError> {
        checked(argv, options)?;
        if self.running.len() >= MAX_RUNNING {
            return Err(JsProcessError::new(
                "busy",
                format!("{MAX_RUNNING} processes are already running"),
            ));
        }
        let mut command = Command::new(&argv[0]);
        command
            .args(&argv[1..])
            .envs(&options.env)
            .stdin(if options.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(options.stdout.stdio())
            .stderr(options.stderr.stdio())
            // A process group of its own, so a kill and a timeout reach the programs it starts too.
            .process_group(0);
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                "not_found"
            } else {
                "internal"
            };
            JsProcessError::new(code, format!("cannot start {}: {error}", argv[0]))
        })?;
        if let (Some(text), Some(mut stdin)) = (options.stdin.clone(), child.stdin.take()) {
            // A program that never reads must not hold the shell: the bytes go from a thread, then stdin closes.
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        let limit = usize::try_from(options.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT)).unwrap_or(usize::MAX);
        let readers = Readers {
            stdout: child
                .stdout
                .take()
                .map(|stream| std::thread::spawn(move || capture(stream, limit))),
            stderr: child
                .stderr
                .take()
                .map(|stream| std::thread::spawn(move || capture(stream, limit))),
        };
        let id = self.next_id;
        self.next_id += 1;
        let pid = child.id();
        let reaped = Arc::new(Mutex::new(false));
        let waiter_reaped = Arc::clone(&reaped);
        let sender = self.sender.clone();
        let timeout = options.timeout.map(Duration::from_secs_f64);
        std::thread::Builder::new()
            .name(format!("js-process-{id}"))
            .spawn(move || {
                let exit = wait_for(child, &waiter_reaped, timeout, readers);
                // The shell may be gone: nobody is left to tell.
                let _ = sender.send((id, exit));
            })
            .map_err(|error| JsProcessError::new("internal", format!("cannot watch the process: {error}")))?;
        self.running.insert(id, Running { pid, reaped });
        Ok(JsStarted { id, pid })
    }

    /// Sends the named signal (`term`, `kill`, `int`, `hup`, `usr1`, `usr2`, `stop`, `cont`) to the process group.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for another name, `not_found` once the process has ended or when this shell never
    /// started it.
    pub fn kill(&mut self, id: u64, signal: &str) -> Result<(), JsProcessError> {
        let Some((_, signal)) = SIGNALS.iter().find(|(name, _)| *name == signal) else {
            let names: Vec<&str> = SIGNALS.iter().map(|(name, _)| *name).collect();
            return Err(JsProcessError::new(
                "invalid_argument",
                format!("no signal named {signal:?}; the signals are {}", names.join(", ")),
            ));
        };
        let not_found = || JsProcessError::new("not_found", format!("process {id} has ended or was never started"));
        let running = self.running.get(&id).ok_or_else(not_found)?;
        let reaped = running.reaped.lock().map_err(|_| not_found())?;
        if *reaped {
            return Err(not_found());
        }
        signal_group(running.pid, *signal);
        Ok(())
    }

    /// The ends the waiters have reported since the last call, oldest first. A process is forgotten here.
    pub fn drain(&mut self) -> Vec<(u64, JsExit)> {
        let ended: Vec<(u64, JsExit)> = self.ends.try_iter().collect();
        for (id, _) in &ended {
            self.running.remove(id);
        }
        ended
    }
}

impl Drop for JsProcesses {
    /// A shell that goes away takes its running programs with it: SIGTERM to each group, as the Lua host does.
    fn drop(&mut self) {
        for running in self.running.values() {
            if let Ok(reaped) = running.reaped.lock()
                && !*reaped
            {
                signal_group(running.pid, Signal::TERM);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{JsProcessError, JsProcesses, JsSpawnOptions, JsStream};

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn options(json: &str) -> JsSpawnOptions {
        serde_json::from_str(json).expect("the options parse")
    }

    /// Waits for the end of process `id`, with a deadline and no sleeping beyond the poll.
    fn end_of(processes: &mut JsProcesses, id: u64) -> super::JsExit {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some((_, exit)) = processes.drain().into_iter().find(|(ended, _)| *ended == id) {
                return exit;
            }
            assert!(Instant::now() < deadline, "process {id} never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_program_ends_with_its_exit_code_and_its_captured_output() {
        let mut processes = JsProcesses::default();
        let started = processes
            .spawn(
                &argv(&["sh", "-c", "printf out; printf err >&2; exit 3"]),
                &options(r#"{"stdout":"capture","stderr":"capture"}"#),
            )
            .expect("sh starts");
        assert_eq!(started.id, 1);
        let exit = end_of(&mut processes, started.id);
        assert_eq!(exit.exit_code, Some(3));
        assert_eq!(exit.signal, None);
        assert_eq!(exit.stdout.as_deref(), Some("out"));
        assert_eq!(exit.stderr.as_deref(), Some("err"));
        assert_eq!(processes.running(), 0, "an ended process is forgotten");
    }

    #[test]
    fn env_cwd_and_stdin_reach_the_program() {
        let mut processes = JsProcesses::default();
        let started = processes
            .spawn(
                &argv(&["sh", "-c", "printf '%s|%s|' \"$GNOBLIN_TEST\" \"$(pwd)\"; cat"]),
                &options(r#"{"env":{"GNOBLIN_TEST":"v"},"cwd":"/","stdin":"fed","stdout":"capture"}"#),
            )
            .expect("sh starts");
        assert_eq!(end_of(&mut processes, started.id).stdout.as_deref(), Some("v|/|fed"));
    }

    #[test]
    fn output_past_the_limit_is_dropped_and_flagged() {
        let mut processes = JsProcesses::default();
        let started = processes
            .spawn(
                &argv(&["sh", "-c", "printf 0123456789"]),
                &options(r#"{"stdout":"capture","outputLimit":4}"#),
            )
            .expect("sh starts");
        let exit = end_of(&mut processes, started.id);
        assert_eq!(exit.stdout.as_deref(), Some("0123"));
        assert!(exit.stdout_truncated);
        assert!(!exit.stderr_truncated);
    }

    #[test]
    fn kill_ends_the_process_group_and_names_the_signal() {
        let mut processes = JsProcesses::default();
        let started = processes
            .spawn(&argv(&["sh", "-c", "sleep 60 & wait"]), &JsSpawnOptions::default())
            .expect("sh starts");
        processes.kill(started.id, "kill").expect("a running process is killed");
        let exit = end_of(&mut processes, started.id);
        assert_eq!(exit.exit_code, None);
        assert_eq!(exit.signal.as_deref(), Some("kill"));
        let error = processes.kill(started.id, "term").expect_err("an ended process has nothing to kill");
        assert_eq!(error.code, "not_found");
    }

    #[test]
    fn a_timeout_terminates_the_program() {
        let mut processes = JsProcesses::default();
        let started = processes
            .spawn(&argv(&["sleep", "60"]), &options(r#"{"timeout":0.05}"#))
            .expect("sleep starts");
        let exit = end_of(&mut processes, started.id);
        assert!(exit.timed_out);
        assert_eq!(exit.signal.as_deref(), Some("term"));
    }

    #[test]
    fn a_missing_program_is_not_found_and_limits_are_refused_by_name() {
        let mut processes = JsProcesses::default();
        let error = processes
            .spawn(&argv(&["/nonexistent/gnoblin-test"]), &JsSpawnOptions::default())
            .expect_err("no such program");
        assert_eq!(error.code, "not_found");
        for (parts, json, needle) in [
            (argv(&[]), "{}", "argv holds"),
            (argv(&["true"]), r#"{"cwd":"relative"}"#, "cwd"),
            (argv(&["true"]), r#"{"timeout":0}"#, "timeout"),
            (argv(&["true"]), r#"{"outputLimit":99999999}"#, "outputLimit"),
            (argv(&["true"]), r#"{"env":{"A=B":"x"}}"#, "env names"),
        ] {
            let JsProcessError { code, message } = processes
                .spawn(&parts, &options(json))
                .expect_err("the options are refused");
            assert_eq!(code, "invalid_argument");
            assert!(message.contains(needle), "{message} names {needle}");
        }
        assert!(processes.kill(99, "term").is_err());
        assert_eq!(
            processes.kill(1, "nope").expect_err("a bad signal").code,
            "invalid_argument"
        );
        assert_eq!(JsStream::default(), JsStream::Inherit);
    }
}
