//! Child processes started by `gnoblin.process.spawn`.
//!
//! Each process gets one waiter thread, which owns the child, polls it, forwards signal
//! requests and reports back over a channel. The pump drains that channel on the host
//! thread, so the Lua state is only ever touched there. A process is in
//! [`Processes::records`] from a successful spawn until its end is drained; a kill or a
//! wait for a process that is not there answers at once, so nothing waits on a reply that
//! can no longer arrive.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gnoblin_client::Transport;
use gnoblin_model::{OperationId, OperationOutcome};
use mlua::{Error, Lua, Table, Value};
use rustix::process::{Pid, Signal, kill_process_group};

use crate::host::{Pending, Shared};

/// Arguments the program may take, counting the program itself.
const MAX_ARGV: usize = 64;
/// Bytes the argument list may hold in total.
const MAX_ARGV_BYTES: usize = 64 * 1024;
/// Variables a spawn may add to the session environment.
const MAX_ENV: usize = 128;
/// Bytes written to standard input.
const MAX_STDIN: usize = 1024 * 1024;
/// The largest `output_limit` a caller may ask for.
const MAX_OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
/// Bytes kept from a captured stream when `output_limit` is not given.
const DEFAULT_OUTPUT_LIMIT: u64 = 64 * 1024;
/// The longest `timeout`, in seconds.
const MAX_TIMEOUT_SECONDS: f64 = 86_400.0;
/// Processes that may run at once. A config past this is looping, so the spawn is refused.
const MAX_RUNNING: usize = 64;
/// How long a timed-out process has after SIGTERM before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// How often the waiter polls its child and its request queue.
const POLL: Duration = Duration::from_millis(5);
/// The read buffer for a captured stream.
const READ_CHUNK: usize = 8 * 1024;

/// The signal names `process:kill` accepts, with the signal each one sends.
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

/// What one standard stream does with the program's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamMode {
    Inherit,
    Discard,
    Capture,
}

/// A spawn request, checked against the limits above before anything starts.
#[derive(Debug)]
pub(crate) struct SpawnSpec {
    argv: Vec<String>,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    stdin: Vec<u8>,
    stdout: StreamMode,
    stderr: StreamMode,
    output_limit: usize,
    timeout: Option<Duration>,
}

/// How a process ended, as the `wait` operation reports it.
#[derive(Debug, Default)]
pub(crate) struct Ended {
    exit_code: Option<i32>,
    signal: Option<&'static str>,
    timed_out: bool,
    stdout: Vec<u8>,
    stdout_truncated: bool,
    stderr: Vec<u8>,
    stderr_truncated: bool,
}

/// A message from a waiter thread, drained by the pump.
#[derive(Debug)]
pub(crate) enum ProcessEvent {
    /// The process ended, however it ended.
    Ended { id: u64, ended: Ended },
    /// A signal request was attempted. `failure` holds the error code when it was not sent.
    Signalled {
        operation: OperationId,
        failure: Option<(&'static str, String)>,
    },
}

/// A request from the host to a waiter thread.
#[derive(Debug)]
pub(crate) struct ProcessRequest {
    operation: OperationId,
    signal: Signal,
}

/// One process that is still running.
pub(crate) struct ProcessRecord {
    /// The `wait` operation; it finishes once the process ends.
    pub(crate) wait: OperationId,
    /// Kill operations waiting for the waiter to answer. An end answers the rest.
    pub(crate) kills: Vec<OperationId>,
    requests: Sender<ProcessRequest>,
}

/// The processes the host has started and not yet seen end.
pub(crate) struct Processes {
    pub(crate) records: BTreeMap<u64, ProcessRecord>,
    events_tx: Sender<ProcessEvent>,
    pub(crate) events: Receiver<ProcessEvent>,
}

impl Processes {
    pub(crate) fn new() -> Self {
        let (events_tx, events) = mpsc::channel();
        Self {
            records: BTreeMap::new(),
            events_tx,
            events,
        }
    }
}

/// Check the arguments of `gnoblin.process.spawn` and return the spec they describe.
///
/// Every argument is checked here, before a process starts, so a bad call raises while
/// the config loads instead of failing later on a thread.
pub(crate) fn parse(argv: Table, options: Option<Table>) -> mlua::Result<SpawnSpec> {
    let mut words = Vec::new();
    let mut bytes = 0usize;
    for index in 1..=argv.raw_len() {
        let Value::String(word) = argv.raw_get::<Value>(index)? else {
            return Err(Error::runtime(format!(
                "gnoblin.process.spawn argv entry {index} must be a string"
            )));
        };
        let word = word
            .to_str()
            .map_err(|_| Error::runtime(format!("gnoblin.process.spawn argv entry {index} is not UTF-8")))?
            .to_owned();
        bytes += word.len();
        words.push(word);
    }
    if words.is_empty() || words.len() > MAX_ARGV {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn takes 1 to {MAX_ARGV} argv strings, found {}",
            words.len()
        )));
    }
    if bytes > MAX_ARGV_BYTES {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn argv holds at most {MAX_ARGV_BYTES} bytes, found {bytes}"
        )));
    }
    let mut spec = SpawnSpec {
        argv: words,
        cwd: None,
        env: Vec::new(),
        stdin: Vec::new(),
        stdout: StreamMode::Inherit,
        stderr: StreamMode::Inherit,
        output_limit: DEFAULT_OUTPUT_LIMIT as usize,
        timeout: None,
    };
    let Some(options) = options else {
        return Ok(spec);
    };
    for pair in options.pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "cwd" => spec.cwd = Some(absolute_path(&value)?),
            "env" => spec.env = environment(&value)?,
            "stdin" => spec.stdin = stdin_bytes(&value)?,
            "stdout" => spec.stdout = mode("stdout", &value)?,
            "stderr" => spec.stderr = mode("stderr", &value)?,
            "output_limit" => spec.output_limit = output_limit(&value)?,
            "timeout" => spec.timeout = Some(timeout(&value)?),
            other => {
                return Err(Error::runtime(format!(
                    "gnoblin.process.spawn has no option named {other:?}"
                )));
            }
        }
    }
    Ok(spec)
}

fn absolute_path(value: &Value) -> mlua::Result<PathBuf> {
    let Value::String(text) = value else {
        return Err(Error::runtime("gnoblin.process.spawn option cwd must be a string"));
    };
    let path = PathBuf::from(text.to_str()?.to_owned());
    if !path.is_absolute() {
        return Err(Error::runtime(
            "gnoblin.process.spawn option cwd must be an absolute path",
        ));
    }
    Ok(path)
}

fn environment(value: &Value) -> mlua::Result<Vec<(String, String)>> {
    let Value::Table(table) = value else {
        return Err(Error::runtime(
            "gnoblin.process.spawn option env must be a table of strings",
        ));
    };
    let mut pairs = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        let (name, value) = pair?;
        let (Value::String(name), Value::String(value)) = (name, value) else {
            return Err(Error::runtime(
                "gnoblin.process.spawn option env must map string names to string values",
            ));
        };
        pairs.push((name.to_str()?.to_owned(), value.to_str()?.to_owned()));
    }
    if pairs.len() > MAX_ENV {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn option env takes at most {MAX_ENV} variables, found {}",
            pairs.len()
        )));
    }
    Ok(pairs)
}

fn stdin_bytes(value: &Value) -> mlua::Result<Vec<u8>> {
    let Value::String(text) = value else {
        return Err(Error::runtime("gnoblin.process.spawn option stdin must be a string"));
    };
    let bytes = text.as_bytes();
    if bytes.len() > MAX_STDIN {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn option stdin takes at most {MAX_STDIN} bytes, found {}",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

fn mode(option: &str, value: &Value) -> mlua::Result<StreamMode> {
    let Value::String(text) = value else {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn option {option} must be \"inherit\", \"discard\" or \"capture\""
        )));
    };
    match text.to_str()?.as_ref() {
        "inherit" => Ok(StreamMode::Inherit),
        "discard" => Ok(StreamMode::Discard),
        "capture" => Ok(StreamMode::Capture),
        other => Err(Error::runtime(format!(
            "gnoblin.process.spawn option {option} must be \"inherit\", \"discard\" or \"capture\", found {other:?}"
        ))),
    }
}

fn output_limit(value: &Value) -> mlua::Result<usize> {
    let Value::Integer(limit) = value else {
        return Err(Error::runtime(
            "gnoblin.process.spawn option output_limit must be an integer",
        ));
    };
    let limit = u64::try_from(*limit)
        .ok()
        .filter(|limit| *limit <= MAX_OUTPUT_LIMIT)
        .ok_or_else(|| {
            Error::runtime(format!(
                "gnoblin.process.spawn option output_limit takes 0 to {MAX_OUTPUT_LIMIT}, found {limit}"
            ))
        })?;
    Ok(limit as usize)
}

fn timeout(value: &Value) -> mlua::Result<Duration> {
    let seconds = match value {
        Value::Integer(seconds) => *seconds as f64,
        Value::Number(seconds) => *seconds,
        _ => {
            return Err(Error::runtime(
                "gnoblin.process.spawn option timeout must be a number of seconds",
            ));
        }
    };
    if !seconds.is_finite() || seconds <= 0.0 || seconds > MAX_TIMEOUT_SECONDS {
        return Err(Error::runtime(format!(
            "gnoblin.process.spawn option timeout takes more than 0 up to {MAX_TIMEOUT_SECONDS} seconds, found {seconds}"
        )));
    }
    Ok(Duration::from_secs_f64(seconds))
}

/// The signal a `kill` call names, or `None` for a name the surface does not accept.
pub(crate) fn signal_named(name: &str) -> Option<Signal> {
    SIGNALS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, signal)| *signal)
}

/// The surface name of a signal that ended a process, or `None` for one the surface does not name.
fn signal_name(raw: i32) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find(|(_, signal)| signal.as_raw() == raw)
        .map(|(name, _)| *name)
}

/// The error code for a process that could not start.
fn start_error_code(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "denied",
        _ => "internal",
    }
}

/// The error code for a signal that could not be sent.
fn signal_error_code(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        _ => "internal",
    }
}

/// Start a process from a checked spec and hand its waiter back.
///
/// Takes the spec by value, so the spec's buffers move into the threads that use them.
/// The waiter reports its end on `events`, tagged with `id`.
fn start(id: u64, spec: SpawnSpec, events: Sender<ProcessEvent>) -> io::Result<Sender<ProcessRequest>> {
    let mut command = Command::new(&spec.argv[0]);
    command.args(&spec.argv[1..]);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    for (name, value) in &spec.env {
        command.env(name, value);
    }
    // A process group of its own: a kill or a timeout reaches what the program starts too.
    command.process_group(0);
    command.stdin(if spec.stdin.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    });
    command.stdout(stdio(spec.stdout));
    command.stderr(stdio(spec.stderr));
    let mut child = command.spawn()?;

    if let Some(stdin) = child.stdin.take() {
        let bytes = spec.stdin;
        // Written on its own thread, so a program that does not read cannot hold the waiter.
        // Dropping the handle closes the pipe, which is what ends a program reading to EOF.
        thread::spawn(move || {
            let mut stdin = stdin;
            let _ = stdin.write_all(&bytes);
        });
    }
    let stdout = child
        .stdout
        .take()
        .filter(|_| spec.stdout == StreamMode::Capture)
        .map(|pipe| read_capped(pipe, spec.output_limit));
    let stderr = child
        .stderr
        .take()
        .filter(|_| spec.stderr == StreamMode::Capture)
        .map(|pipe| read_capped(pipe, spec.output_limit));

    let (requests, requests_rx) = mpsc::channel();
    let timeout = spec.timeout;
    thread::spawn(move || {
        let ended = wait_for(child, stdout, stderr, timeout, &requests_rx, &events);
        let _ = events.send(ProcessEvent::Ended { id, ended });
    });
    Ok(requests)
}

fn stdio(mode: StreamMode) -> Stdio {
    match mode {
        StreamMode::Inherit => Stdio::inherit(),
        StreamMode::Discard => Stdio::null(),
        StreamMode::Capture => Stdio::piped(),
    }
}

/// Read a captured stream to its end, keeping at most `limit` bytes.
///
/// The rest is still read and dropped, so the program never blocks on a full pipe. The
/// flag says whether anything was dropped.
fn read_capped<R: Read + Send + 'static>(mut pipe: R, limit: usize) -> JoinHandle<(Vec<u8>, bool)> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut truncated = false;
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let room = limit - kept.len();
                    if read > room {
                        truncated = true;
                    }
                    kept.extend_from_slice(&chunk[..read.min(room)]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        (kept, truncated)
    })
}

/// Own the child until it ends: poll it, answer signal requests and enforce the timeout.
fn wait_for(
    mut child: Child,
    stdout: Option<JoinHandle<(Vec<u8>, bool)>>,
    stderr: Option<JoinHandle<(Vec<u8>, bool)>>,
    timeout: Option<Duration>,
    requests: &Receiver<ProcessRequest>,
    events: &Sender<ProcessEvent>,
) -> Ended {
    let started = Instant::now();
    let deadline = timeout.map(|limit| started + limit);
    let group = Pid::from_raw(child.id() as i32);
    let mut timed_out = false;
    let mut force_at: Option<Instant> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        while let Ok(request) = requests.try_recv() {
            let sent = send_to_group(group, request.signal);
            let failure = sent.err().map(|error| (signal_error_code(&error), error.to_string()));
            let _ = events.send(ProcessEvent::Signalled {
                operation: request.operation,
                failure,
            });
        }
        let now = Instant::now();
        if !timed_out && deadline.is_some_and(|at| now >= at) {
            timed_out = true;
            let _ = send_to_group(group, Signal::TERM);
            force_at = Some(now + KILL_GRACE);
        }
        if force_at.is_some_and(|at| now >= at) {
            force_at = None;
            let _ = send_to_group(group, Signal::KILL);
        }
        thread::sleep(POLL);
    };
    let (stdout, stdout_truncated) = join_capture(stdout);
    let (stderr, stderr_truncated) = join_capture(stderr);
    let (exit_code, signal) = match status {
        Some(status) => (status.code(), status.signal().and_then(signal_name)),
        None => (None, None),
    };
    Ended {
        exit_code,
        signal,
        timed_out,
        stdout,
        stdout_truncated,
        stderr,
        stderr_truncated,
    }
}

fn join_capture(reader: Option<JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    reader.and_then(|reader| reader.join().ok()).unwrap_or_default()
}

/// Send a signal to the process group, so the program's own children get it too.
fn send_to_group(group: Option<Pid>, signal: Signal) -> io::Result<()> {
    let group = group.ok_or_else(|| io::Error::other("the process has no group"))?;
    kill_process_group(group, signal).map_err(io::Error::from)
}

/// Start the process and record it, so its `wait` and `kill` can find it.
///
/// A process that could not start is answered on its `wait` operation at once, with the
/// code the start error maps to, and is not recorded.
pub(crate) fn spawn<T: Transport + 'static>(
    shared: &Rc<RefCell<Shared<T>>>,
    spec: SpawnSpec,
) -> mlua::Result<(u64, OperationId)> {
    let (id, wait, events) = {
        let mut shared = shared.borrow_mut();
        if shared.processes.records.len() >= MAX_RUNNING {
            return Err(Error::runtime(format!(
                "gnoblin.process.spawn has {MAX_RUNNING} processes running; wait for one to end"
            )));
        }
        let id = shared.allocate();
        let wait = shared.allocate_operation();
        shared.operations.insert(
            wait,
            Pending {
                outcome: None,
                local_error: None,
                completions: Vec::new(),
            },
        );
        (id, wait, shared.processes.events_tx.clone())
    };
    let started = start(id, spec, events);
    let mut shared = shared.borrow_mut();
    match started {
        Ok(requests) => {
            shared.processes.records.insert(
                id,
                ProcessRecord {
                    wait,
                    kills: Vec::new(),
                    requests,
                },
            );
        }
        Err(error) => {
            if let Some(pending) = shared.operations.get_mut(&wait) {
                pending.outcome = Some(OperationOutcome::Done);
                pending.local_error = Some((start_error_code(error.kind()).to_owned(), error.to_string()));
            }
        }
    }
    Ok((id, wait))
}

/// Ask a running process for a signal. The answer is the returned operation.
///
/// A process that is no longer recorded has ended, so the answer is `not_found` at once.
pub(crate) fn kill<T: Transport + 'static>(shared: &Rc<RefCell<Shared<T>>>, id: u64, signal: Signal) -> OperationId {
    let mut shared = shared.borrow_mut();
    let operation = shared.allocate_operation();
    let mut pending = Pending {
        outcome: None,
        local_error: None,
        completions: Vec::new(),
    };
    match shared.processes.records.get_mut(&id) {
        Some(record) => {
            record.kills.push(operation);
            // A send fails only when the waiter has gone, which means the end is already on
            // the channel; the drain answers this kill with `not_found` then.
            let _ = record.requests.send(ProcessRequest { operation, signal });
        }
        None => {
            pending.outcome = Some(OperationOutcome::Done);
            pending.local_error = Some(("not_found".to_owned(), "the process has ended".to_owned()));
        }
    }
    shared.operations.insert(operation, pending);
    operation
}

/// Apply one drained message: finish the waiting operations and return the handlers they wake.
pub(crate) fn apply<T: Transport + 'static>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    event: ProcessEvent,
) -> mlua::Result<Vec<u64>> {
    match event {
        ProcessEvent::Ended { id, ended } => {
            let record = shared.borrow_mut().processes.records.remove(&id);
            let Some(record) = record else {
                return Ok(Vec::new());
            };
            let value = result_table(lua, &ended)?;
            crate::bindings::callbacks(lua)?.raw_set(format!("operation-value:{}", record.wait.get()), value)?;
            let mut shared = shared.borrow_mut();
            let mut ready = shared.finish_local(record.wait, OperationOutcome::Done, None);
            for kill in record.kills {
                ready.extend(shared.finish_local(
                    kill,
                    OperationOutcome::Done,
                    Some(("not_found".to_owned(), "the process has ended".to_owned())),
                ));
            }
            Ok(ready)
        }
        ProcessEvent::Signalled { operation, failure } => {
            let mut shared = shared.borrow_mut();
            Ok(shared.finish_local(
                operation,
                OperationOutcome::Done,
                failure.map(|(code, message)| (code.to_owned(), message)),
            ))
        }
    }
}

/// The table a `wait` operation returns for a process that ended.
fn result_table(lua: &Lua, ended: &Ended) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("exit_code", ended.exit_code)?;
    table.set("signal", ended.signal)?;
    table.set("timed_out", ended.timed_out)?;
    table.set("stdout", lua.create_string(&ended.stdout)?)?;
    table.set("stdout_truncated", ended.stdout_truncated)?;
    table.set("stderr", lua.create_string(&ended.stderr)?)?;
    table.set("stderr_truncated", ended.stderr_truncated)?;
    Ok(table)
}
