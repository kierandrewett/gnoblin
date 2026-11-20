//! Programs a script starts.
//!
//! A program is started by the core, in a process group of its own and not through a shell. A program the script's top level starts
//! waits until the generation takes over, so a reload that fails starts nothing, and one a reload's generation starts again with
//! the same arguments is adopted instead of doubled. Those rules are the core's; this layer only passes the options.

use gnoblin_core::{ProcessExit, Signal, Stream};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::callback::{callable, fire};
use crate::errors::core_error;
use crate::generation::{current_ctx, handle, keep_process, process};

fn stream(name: &str) -> PyResult<Stream> {
    match name {
        "inherit" => Ok(Stream::Inherit),
        "discard" => Ok(Stream::Discard),
        "capture" => Ok(Stream::Capture),
        other => Err(PyValueError::new_err(format!(
            "a stream is `inherit`, `discard` or `capture`, not `{other}`"
        ))),
    }
}

pub(crate) fn signal(name: &str) -> PyResult<Signal> {
    match name {
        "term" => Ok(Signal::Term),
        "kill" => Ok(Signal::Kill),
        "int" => Ok(Signal::Int),
        "hup" => Ok(Signal::Hup),
        "usr1" => Ok(Signal::Usr1),
        "usr2" => Ok(Signal::Usr2),
        "stop" => Ok(Signal::Stop),
        "cont" => Ok(Signal::Cont),
        other => Err(PyValueError::new_err(format!(
            "a signal is term, kill, int, hup, usr1, usr2, stop or cont, not `{other}`"
        ))),
    }
}

/// Calls `callback(code, signal, timed_out, stdout, stderr, stdout_truncated, stderr_truncated)` when a program ends.
fn exited(callback: &Py<PyAny>, exit: &ProcessExit) {
    fire("a program's exit hook", callback, |_, f| {
        f.call1((
            exit.code,
            exit.signal.as_deref(),
            exit.timed_out,
            exit.stdout.as_str(),
            exit.stderr.as_str(),
            exit.stdout_truncated,
            exit.stderr_truncated,
        ))
    });
}

/// Starts `argv[0]` with the rest as its arguments; returns the number that signals it.
#[pyfunction]
#[pyo3(signature = (argv, cwd = None, env = None, stdin = None, stdout = "inherit", stderr = "inherit", output_limit = 0, timeout_ms = 0, on_exit = None))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    argv: Vec<String>,
    cwd: Option<String>,
    env: Option<Vec<(String, String)>>,
    stdin: Option<String>,
    stdout: &str,
    stderr: &str,
    output_limit: u64,
    timeout_ms: u32,
    on_exit: Option<&Bound<'_, PyAny>>,
) -> PyResult<u64> {
    let Some((program, arguments)) = argv.split_first() else {
        return Err(PyValueError::new_err("a program to start needs a name"));
    };
    let mut command = handle()?
        .command(program.as_str())
        .args(arguments.iter().cloned())
        .stdout(stream(stdout)?)
        .stderr(stream(stderr)?)
        .output_limit(output_limit)
        .timeout(timeout_ms);
    if let Some(cwd) = cwd {
        command = command.cwd(cwd);
    }
    for (name, value) in env.iter().flatten() {
        command = command.env(name, value);
    }
    if let Some(text) = stdin {
        command = command.stdin(text);
    }
    if let Some(hook) = on_exit {
        let hook = callable(hook)?;
        command = command.on_exit(move |exit| exited(&hook, exit));
    }
    let started = command.spawn().map_err(|e| core_error(&e))?;
    Ok(keep_process(current_ctx(), started))
}

/// Sends a signal to the program's process group.
#[pyfunction]
pub(crate) fn process_signal(token: u64, name: &str) -> PyResult<()> {
    process(token)?.signal(signal(name)?).map_err(|e| core_error(&e))
}
