//! Staging, committing and ending a generation of the script.
//!
//! `start` evaluates the script in a fresh namespace beside the generation that serves (`gnoblin/_loader.py` does the isolation) and
//! leaves the generation staged; `activate` makes it the one that serves; `abandon` throws it away. A failed evaluation changes
//! nothing that was running: the loader puts the previous modules back, this module puts the previous generation back as the one the
//! module `_gnoblin_core` acts on, and the reason comes back to the host as the text of the failure (`FILE:LINE: Type: message`), with
//! the traceback in the log.

use std::path::Path;

use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};
use pyo3::prelude::*;

use crate::callback::traceback_text;
use crate::generation::{self, init, with_state};
use crate::interp;

/// What the loader returns for a staged generation: whether it evaluated, why not, the traceback, and the files to watch.
type Staged = (bool, String, String, Vec<(String, i128, u64, u64)>);

/// The text of an exception of the runtime's own (a bug in the loader, not in the script).
fn internal(py: Python<'_>, what: &str, error: &PyErr) -> String {
    format!("the Python runtime failed in {what}: {}", traceback_text(py, error))
}

/// Stages a generation of `script`.
///
/// # Errors
///
/// The one-line reason the host reports: the interpreter did not start, or the script did not evaluate.
pub(crate) fn stage(api: &'static GnoblinApi, ctx: GnoblinCtx, script: &Path) -> Result<(), String> {
    init(api);
    let handle = gnoblin_core::private::gnoblin_for(ctx).map_err(|error| format!("cannot reach the core: {error}"))?;
    let previous = with_state(|state| {
        state.handles.insert(ctx, handle);
        state.staged = Some(ctx);
        std::mem::replace(&mut state.current, ctx)
    })
    .ok_or("the Python runtime has no state on this thread")?;
    // A line logged while the script runs carries the generation that is starting; once one serves, the others follow it.
    if with_state(|state| state.serving.is_none()).unwrap_or(true) {
        generation::aim_poster(ctx);
    }
    let outcome = interp::boot(api).and_then(|()| {
        Python::attach(|py| -> Result<Staged, String> {
            let loader = py
                .import("gnoblin._loader")
                .map_err(|error| internal(py, "import", &error))?;
            loader
                .call_method1("load", (script.display().to_string(),))
                .and_then(|staged| staged.extract())
                .map_err(|error| internal(py, "load", &error))
        })
    });
    let (reason, watch) = match outcome {
        Ok((true, _, _, watch)) => {
            with_state(|state| state.pending_watch = Some(watch));
            return Ok(());
        }
        Ok((false, reason, detail, watch)) => {
            generation::log_error(&format!("python: {} did not load\n{detail}", script.display()));
            (reason, Some(watch))
        }
        Err(message) => (message, None),
    };
    with_state(|state| {
        state.handles.remove(&ctx);
        state.staged = None;
        state.current = previous;
        if let (Some(watch), Some(_)) = (watch, state.serving) {
            state.watch.replace(watch);
        }
    });
    if let Some(serving) = with_state(|state| state.serving).flatten() {
        generation::aim_poster(serving);
    }
    Err(reason)
}

/// Makes the staged generation `ctx` the one that serves.
///
/// # Errors
///
/// `ctx` is not the staged generation, or the loader failed.
pub(crate) fn activate(ctx: GnoblinCtx) -> Result<(), String> {
    if with_state(|state| state.staged) != Some(Some(ctx)) {
        return Err(format!("generation {ctx:#x} is not the staged one"));
    }
    Python::attach(|py| -> Result<(), String> {
        py.import("gnoblin._loader")
            .and_then(|loader| loader.call_method0("activate"))
            .map(drop)
            .map_err(|error| internal(py, "activate", &error))
    })?;
    with_state(|state| {
        state.staged = None;
        state.serving = Some(ctx);
        state.current = ctx;
        let watch = state.pending_watch.take().unwrap_or_default();
        state.watch.replace(watch);
    });
    generation::aim_poster(ctx);
    Ok(())
}

/// Throws the staged generation `ctx` away.
pub(crate) fn abandon(ctx: GnoblinCtx) {
    if with_state(|state| state.staged) == Some(Some(ctx)) {
        let undone = Python::attach(|py| {
            py.import("gnoblin._loader")
                .and_then(|loader| loader.call_method0("abandon"))
                .map(drop)
                .map_err(|error| internal(py, "abandon", &error))
        });
        if let Err(message) = undone {
            generation::log_error(&message);
        }
    }
    let serving = with_state(|state| {
        state.staged = None;
        state.pending_watch = None;
        state.current = state.serving.unwrap_or(0);
        state.serving
    })
    .flatten();
    generation::release(ctx);
    if let Some(serving) = serving {
        generation::aim_poster(serving);
    }
}

/// Ends the generation `ctx`: the shell when it is the one that serves, else a generation a reload replaced.
pub(crate) fn end(ctx: GnoblinCtx) {
    let serving = with_state(|state| state.serving == Some(ctx)).unwrap_or(false);
    if !serving {
        if with_state(|state| state.staged) == Some(Some(ctx)) {
            abandon(ctx);
        } else {
            generation::release(ctx);
        }
        return;
    }
    let ended = Python::attach(|py| {
        py.import("gnoblin._loader")
            .and_then(|loader| loader.call_method0("end"))
            .map(drop)
            .map_err(|error| internal(py, "end", &error))
    });
    if let Err(message) = ended {
        generation::log_error(&message);
    }
    with_state(|state| {
        state.serving = None;
        state.current = 0;
        state.watch.clear();
    });
    generation::release(ctx);
    generation::clear_poster();
}
