//! `gnoblin.process` on the Rust side: the programs a script starts, and how each one ends.
//!
//! The runtime role owns the children, as the earlier runtime host did: nothing but this process can reap them, so a
//! process is local to the shell and never crosses the owner socket. Each child gets one waiter thread that
//! owns it, polls it, enforces the timeout and reads what a captured stream holds. The shell drains the
//! waiters' channel once a frame ([`Processes::drain`]), so the script's state is only ever touched on
//! the shell's thread.
//!
//! A process leaves [`Processes`] when its end is drained. A kill of a process that is not there answers
//! `not-found` at once, so nothing waits on an end that can no longer arrive.
//!
//! Each process has an owner, the generation that started or adopted it, and a process the config's top level starts
//! (`gnoblin.spawn` while the entry loads) is the config's: a stage defers it and starts it only when its generation
//! takes over, so a stage that fails starts nothing and a reload that starts the same program again keeps the process it
//! has instead of doubling it. What a reload's generation no longer starts is stopped; a process a handler started is the
//! user's and a reload leaves it running.
//!
//! The tests are in `tests.rs`.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, mpsc};
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
/// The longest `timeout`, in milliseconds: a day. The script API counts milliseconds everywhere.
const MAX_TIMEOUT_MS: f64 = 86_400_000.0;
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

/// The codes of `docs/shell-api/errors.md` a process call can fail with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessCode {
    /// A limit or an option is wrong.
    InvalidArgument,
    /// The program or the process is not there.
    NotFound,
    /// Too many processes run.
    Busy,
    /// The system refused for another reason.
    Internal,
}

impl ProcessCode {
    /// The spelling a script sees.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::NotFound => "not_found",
            Self::Busy => "busy",
            Self::Internal => "internal",
        }
    }
}

/// Why a spawn or a kill was refused. The code is what the script's error carries.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProcessError {
    /// `invalid_argument`, `not_found`, `busy` or `internal`, the codes `docs/shell-api/errors.md` lists.
    pub code: ProcessCode,
    /// What the script's author reads.
    pub message: String,
}

impl ProcessError {
    fn new(code: ProcessCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// What one standard stream does with the program's output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    /// Writes to the runtime's own output, which the session log records.
    #[serde(rename = "inherit")]
    #[default]
    Log,
    /// Drops the output.
    #[serde(rename = "discard")]
    Drop,
    /// Keeps the output, up to `outputLimit` bytes, for the end event.
    #[serde(rename = "capture")]
    Keep,
}

impl StreamMode {
    fn stdio(self) -> Stdio {
        match self {
            Self::Log => Stdio::inherit(),
            Self::Drop => Stdio::null(),
            Self::Keep => Stdio::piped(),
        }
    }
}

/// The options of a spawn as the script's module hands them over, checked against the limits above.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnOptions {
    /// The working directory, the shell's own when `None`.
    pub cwd: Option<PathBuf>,
    /// Variables added to the session's environment.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Text written to standard input, which is then closed.
    pub stdin: Option<String>,
    /// What standard output does.
    #[serde(default)]
    pub stdout: StreamMode,
    /// What standard error does.
    #[serde(default)]
    pub stderr: StreamMode,
    /// The most bytes of a captured stream kept.
    pub output_limit: Option<u64>,
    /// Milliseconds before the group is terminated.
    pub timeout: Option<f64>,
}

/// How a process ended.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessExit {
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
pub struct Started {
    /// The identifier the script holds.
    pub id: u64,
    /// The operating system's process id.
    pub pid: u32,
}

/// A program and the options it starts with: what two spawns have to share for one to stand in for the other.
#[derive(Clone, Debug, PartialEq)]
struct Spawn {
    argv: Vec<String>,
    options: SpawnOptions,
}

struct LiveProcess {
    pid: u32,
    /// True once the waiter has reaped the child, under the lock the waiter reaps with, so a signal is never
    /// sent to a process id the kernel may already have handed to something else.
    reaped: Arc<Mutex<bool>>,
    /// The generation that started it, or took it over at a reload.
    owner: u64,
    /// What the config's top level started it with. `None` for a process a handler started.
    declared: Option<Spawn>,
}

impl LiveProcess {
    /// Sends `signal` to the process group unless the waiter has reaped it. False when it has: nobody is left to signal.
    fn signal(&self, signal: Signal) -> bool {
        let Ok(reaped) = self.reaped.lock() else {
            return false;
        };
        if *reaped {
            return false;
        }
        signal_group(self.pid, signal);
        true
    }
}

/// What a spawn made while a generation was staged will do when it takes over.
enum Staged {
    /// A running process of the generation being replaced that the stage started again: it stays, and is the stage's.
    Adopt(u64),
    /// A program that has not started: it starts when the generation takes over, under the id the script was given.
    Start { id: u64, spawn: Spawn },
}

/// The processes a shell started and has not seen end.
pub struct Processes {
    next_id: u64,
    running: BTreeMap<u64, LiveProcess>,
    ends: Receiver<(u64, ProcessExit)>,
    sender: Sender<(u64, ProcessExit)>,
    /// The generation that runs, to which a process started now belongs.
    owner: u64,
    /// The spawns of the generation being staged, `None` when none is.
    stage: Option<Vec<Staged>>,
}

impl Default for Processes {
    fn default() -> Self {
        let (sender, ends) = mpsc::channel();
        Self {
            next_id: 1,
            running: BTreeMap::new(),
            ends,
            sender,
            owner: 0,
            stage: None,
        }
    }
}

/// The program, its arguments and the options, refused with the name of the thing that is wrong.
fn checked(argv: &[String], options: &SpawnOptions) -> Result<(), ProcessError> {
    let invalid = |message: String| Err(ProcessError::new(ProcessCode::InvalidArgument, message));
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
    if let Some(ms) = options.timeout
        && !(ms > 0.0 && ms <= MAX_TIMEOUT_MS)
    {
        return invalid(format!(
            "timeout is more than 0 and at most {MAX_TIMEOUT_MS} milliseconds"
        ));
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
fn wait_for(mut child: Child, reaped: &Mutex<bool>, timeout: Option<Duration>, readers: PipeReaders) -> ProcessExit {
    let started = Instant::now();
    let pid = child.id();
    let mut timed_out = false;
    let mut kill_at: Option<Instant> = None;
    let status = loop {
        {
            let Ok(mut done) = reaped.lock() else {
                return ProcessExit::default();
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
    ProcessExit {
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
struct PipeReaders {
    stdout: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>,
    stderr: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>,
}

impl Processes {
    /// How many processes are running.
    pub fn running(&self) -> usize {
        self.running.len()
    }

    /// The operating system's process id of process `id`, or `None` when it is not running: it has ended, or it is a
    /// spawn of a staged generation that has not taken over, which has not started.
    pub fn pid(&self, id: u64) -> Option<u32> {
        self.running.get(&id).map(|running| running.pid)
    }

    /// Whether a generation is staged: its spawns wait for the takeover.
    pub fn is_staging(&self) -> bool {
        self.stage.is_some()
    }

    /// The generation that owns process `id`, or `None` when it is not running.
    pub fn owner(&self, id: u64) -> Option<u64> {
        self.running.get(&id).map(|running| running.owner)
    }

    /// Starts `argv` in a process group of its own, with `options` over the session's environment.
    ///
    /// While a generation is staged ([`begin_stage`](Self::begin_stage)) nothing starts: the spawn is checked, given the
    /// id its handle will carry and kept, and the answer's `pid` is 0 until the generation takes over. A program the
    /// running generation's config started with the same arguments and options is not started again: the staged
    /// generation takes that process over, and the answer is its id and pid.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for a limit or a malformed option, `busy` at 64 running processes, and `not_found` or
    /// `internal` when the operating system refuses to start the program, with its reason.
    pub fn spawn(&mut self, argv: &[String], options: &SpawnOptions) -> Result<Started, ProcessError> {
        checked(argv, options)?;
        let spawn = Spawn {
            argv: argv.to_vec(),
            options: options.clone(),
        };
        if self.stage.is_some() {
            return self.stage_spawn(spawn);
        }
        let id = self.next_id;
        self.next_id += 1;
        let pid = self.start(id, spawn, false)?;
        Ok(Started { id, pid })
    }

    /// Keeps a spawn of the staged generation: it adopts the running process of the generation being replaced that the
    /// config declared with the same arguments, if one is not adopted already, and otherwise waits to start.
    fn stage_spawn(&mut self, spawn: Spawn) -> Result<Started, ProcessError> {
        let staged = self.stage.get_or_insert_with(Vec::new);
        let adopted = |id: &u64| {
            staged
                .iter()
                .any(|kept| matches!(kept, Staged::Adopt(taken) if taken == id))
        };
        let same = self
            .running
            .iter()
            .find(|(id, running)| {
                running.owner == self.owner && running.declared.as_ref() == Some(&spawn) && !adopted(id)
            })
            .map(|(id, running)| (*id, running.pid));
        if let Some((id, pid)) = same {
            staged.push(Staged::Adopt(id));
            return Ok(Started { id, pid });
        }
        let waiting = staged
            .iter()
            .filter(|kept| matches!(kept, Staged::Start { .. }))
            .count();
        if self.running.len() + waiting >= MAX_RUNNING {
            return Err(ProcessError::new(
                ProcessCode::Busy,
                format!("{MAX_RUNNING} processes are already running"),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        staged.push(Staged::Start { id, spawn });
        Ok(Started { id, pid: 0 })
    }

    /// Starts `spawn` as process `id`, owned by the running generation, and returns its operating system id.
    /// `declared` says the config's top level asked for it.
    fn start(&mut self, id: u64, spawn: Spawn, declared: bool) -> Result<u32, ProcessError> {
        let Spawn { argv, options } = &spawn;
        if self.running.len() >= MAX_RUNNING {
            return Err(ProcessError::new(
                ProcessCode::Busy,
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
                ProcessCode::NotFound
            } else {
                ProcessCode::Internal
            };
            ProcessError::new(code, format!("cannot start {}: {error}", argv[0]))
        })?;
        if let (Some(text), Some(mut stdin)) = (options.stdin.clone(), child.stdin.take()) {
            // A program that never reads must not hold the shell: the bytes go from a thread, then stdin closes.
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        let limit = usize::try_from(options.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT)).unwrap_or(usize::MAX);
        let readers = PipeReaders {
            stdout: child
                .stdout
                .take()
                .map(|stream| std::thread::spawn(move || capture(stream, limit))),
            stderr: child
                .stderr
                .take()
                .map(|stream| std::thread::spawn(move || capture(stream, limit))),
        };
        let pid = child.id();
        let reaped = Arc::new(Mutex::new(false));
        let waiter_reaped = Arc::clone(&reaped);
        let sender = self.sender.clone();
        let timeout = options.timeout.map(|ms| Duration::from_secs_f64(ms / 1000.0));
        std::thread::Builder::new()
            .name(format!("process-{id}"))
            .spawn(move || {
                let exit = wait_for(child, &waiter_reaped, timeout, readers);
                // The shell may be gone: nobody is left to tell.
                let _ = sender.send((id, exit));
            })
            .map_err(|error| ProcessError::new(ProcessCode::Internal, format!("cannot watch the process: {error}")))?;
        self.running.insert(
            id,
            LiveProcess {
                pid,
                reaped,
                owner: self.owner,
                declared: declared.then_some(spawn),
            },
        );
        Ok(pid)
    }

    /// A generation is about to be staged: what its config spawns is kept, not started, until it takes over.
    pub fn begin_stage(&mut self) {
        self.stage = Some(Vec::new());
    }

    /// Another generation is staged inside the same stage (a restage after the first was interrupted): the spawns the
    /// first one kept are dropped, or a program both spawn would start twice. Nothing happens when no stage is open.
    pub fn restage(&mut self) {
        if let Some(kept) = self.stage.as_mut() {
            kept.clear();
        }
    }

    /// The stage failed: the spawns it kept are dropped, and nothing that runs was touched.
    pub fn abandon_stage(&mut self) {
        self.stage = None;
    }

    /// The staged generation `owner` takes over. The config's processes it started again stay and become its own, the
    /// ones it did not are stopped (SIGTERM to the group), and the programs it spawned for the first time start now.
    ///
    /// Returns the spawns that could not start, as `(id, why)`. Each has been ended with exit status 127 and its reason
    /// as standard error, so the handle the script holds is not left running for ever.
    pub fn end_stage(&mut self, owner: u64) -> Vec<(u64, ProcessError)> {
        let Some(staged) = self.stage.take() else {
            return Vec::new();
        };
        let previous = std::mem::replace(&mut self.owner, owner);
        let adopted: Vec<u64> = staged
            .iter()
            .filter_map(|kept| match kept {
                Staged::Adopt(id) => Some(*id),
                Staged::Start { .. } => None,
            })
            .collect();
        for (id, running) in &mut self.running {
            if running.owner != previous || running.declared.is_none() {
                continue;
            }
            if adopted.contains(id) {
                running.owner = owner;
            } else {
                // The config no longer starts it: it is not the config's any more, and it is told to stop.
                running.declared = None;
                running.signal(Signal::TERM);
            }
        }
        let mut failed = Vec::new();
        for kept in staged {
            if let Staged::Start { id, spawn } = kept
                && let Err(error) = self.start(id, spawn, true)
            {
                let exit = ProcessExit {
                    exit_code: Some(127),
                    stderr: Some(error.message.clone()),
                    ..ProcessExit::default()
                };
                // The shell may be gone: nobody is left to tell.
                let _ = self.sender.send((id, exit));
                failed.push((id, error));
            }
        }
        failed
    }

    /// Sends the named signal (`term`, `kill`, `int`, `hup`, `usr1`, `usr2`, `stop`, `cont`) to the process group. A
    /// process a staged generation has not started yet is not started, and ends with that signal.
    ///
    /// # Errors
    ///
    /// `invalid_argument` for another name, `not_found` once the process has ended or when this shell never
    /// started it.
    pub fn kill(&mut self, id: u64, signal: &str) -> Result<(), ProcessError> {
        let Some((name, signal)) = SIGNALS.iter().find(|(name, _)| *name == signal) else {
            let names: Vec<&str> = SIGNALS.iter().map(|(name, _)| *name).collect();
            return Err(ProcessError::new(
                ProcessCode::InvalidArgument,
                format!("no signal named {signal:?}; the signals are {}", names.join(", ")),
            ));
        };
        let not_found = || {
            ProcessError::new(
                ProcessCode::NotFound,
                format!("process {id} has ended or was never started"),
            )
        };
        if let Some(staged) = self.stage.as_mut()
            && let Some(at) = staged
                .iter()
                .position(|kept| matches!(kept, Staged::Start { id: waiting, .. } if *waiting == id))
        {
            staged.remove(at);
            let exit = ProcessExit {
                signal: Some((*name).to_owned()),
                ..ProcessExit::default()
            };
            // The shell may be gone: nobody is left to tell.
            let _ = self.sender.send((id, exit));
            return Ok(());
        }
        let running = self.running.get(&id).ok_or_else(not_found)?;
        if running.signal(*signal) {
            Ok(())
        } else {
            Err(not_found())
        }
    }

    /// The ends the waiters have reported since the last call, oldest first. A process is forgotten here.
    pub fn drain(&mut self) -> Vec<(u64, ProcessExit)> {
        let ended: Vec<(u64, ProcessExit)> = self.ends.try_iter().collect();
        for (id, _) in &ended {
            self.running.remove(id);
        }
        ended
    }
}

impl Drop for Processes {
    /// A shell that goes away takes its running programs with it: SIGTERM to each group, as the earlier runtime host did.
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
mod tests;
