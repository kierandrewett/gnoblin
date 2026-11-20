//! Staging, committing and ending a generation of the script.
//!
//! `start` evaluates the script in a fresh namespace beside the generation that serves ([`crate::loader`] does the isolation) and
//! leaves the generation staged; `activate` makes it the one that serves; `abandon` throws it away. A failed evaluation changes
//! nothing that was running: the loader puts the previous modules back, this module puts the previous generation back as the one the
//! module `_gnoblin_core` acts on, and the reason comes back to the host as the text of the failure (`FILE:LINE: Type: message`), with
//! the traceback in the log.

use std::path::Path;

use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};
use pyo3::prelude::*;

use crate::callback::traceback_text;
use crate::generation::{self, init, with_state};
use crate::{interp, loader};

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
    let outcome = interp::boot().and_then(|()| {
        Python::attach(|py| {
            loader::load(py, &script.display().to_string()).map_err(|error| internal(py, "load", &error))
        })
    });
    let (reason, watch) = match outcome {
        Ok(loaded) if loaded.ok => {
            with_state(|state| state.pending_watch = Some(loaded.watch));
            return Ok(());
        }
        Ok(loaded) => {
            generation::log_error(&format!("python: {} did not load\n{}", script.display(), loaded.detail));
            (loaded.reason, Some(loaded.watch))
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
/// `ctx` is not the staged generation.
pub(crate) fn activate(ctx: GnoblinCtx) -> Result<(), String> {
    if with_state(|state| state.staged) != Some(Some(ctx)) {
        return Err(format!("generation {ctx:#x} is not the staged one"));
    }
    loader::activate();
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
        let undone = Python::attach(|py| loader::abandon(py).map_err(|error| internal(py, "abandon", &error)));
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
    let ended = Python::attach(|py| loader::end(py).map_err(|error| internal(py, "end", &error)));
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
