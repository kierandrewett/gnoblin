//! The JavaScript shell this thread runs, and what each entry point of the runtime does to it.
//!
//! One shell instance serves for the life of the process: the engine's [`JsRole`] keeps the tree mirror, the nodes the shell
//! made, the workspaces, the claims and the watcher. A generation is one evaluation of the script. The first `start` boots the
//! role over the core's link and evaluates the script, and every later `start` evaluates it in a staging generation beside the
//! one that serves and swaps it in only if it evaluated cleanly, which is the engine's own reload (`JsRole::reload_now`). The
//! core's contexts follow: the context of the generation that serves is the one the link and the log use, and it moves to the new
//! one when the swap commits.
//!
//! A change to a file the script loaded is seen by the role's own watcher, which knows every module the generation imported.
//! The role does not reload itself ([`JsRoleConfig::external_reload`]): the runtime asks the core for a reload
//! (`request_reload`), and the host serves it between turns through the loader every language shares, which calls `start` with a
//! new context. Hot reload of a script is therefore the same protocol, the same counters and the same log lines as a native one.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_js::{JsBytecodeCache, JsRole, JsRoleConfig, JsRoleLinks};

use crate::link::{RelayedLink, Serving};
use crate::table::{CoreTable, set_log_context};

/// The runtime's version string, as the entry reports it and the bytecode cache keys on it.
pub(crate) const VERSION: &str = concat!("gnoblin-runtime-js ", env!("CARGO_PKG_VERSION"));

/// The longest one turn waits when the engine has no timer due: what has no descriptor to wait on (the watcher, the console, the
/// session thread's events) is looked at this often. One display frame at 60 Hz.
const FRAME_WAIT: Duration = Duration::from_millis(16);

/// The shortest wait of a turn when a frame is due now, so a shell that keeps asking for one is not spun on.
const MIN_WAIT: Duration = Duration::from_millis(1);

/// The shell this thread runs.
struct Instance {
    table: CoreTable,
    role: JsRole,
    /// The context of the generation that serves.
    serving: Serving,
}

thread_local! {
    static INSTANCE: RefCell<Option<Instance>> = const { RefCell::new(None) };
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The engine's configuration for the script at `script`: imports resolve under its directory, the cache is the runtime's, and the
/// watcher asks the host to reload.
fn role_config(script: &Path) -> JsRoleConfig {
    let env = &environment;
    let dir = script.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let stock = env("GNOBLIN_JS_STOCK_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from);
    let mut config = JsRoleConfig::new(dir, stock);
    config.entry = script.to_path_buf();
    config.watch = env("GNOBLIN_JS_WATCH").as_deref() != Some("0");
    config.external_reload = true;
    config.shell.xr_declared = env("GNOBLIN_XR").as_deref() == Some("1");
    config.shell.js.cache = JsBytecodeCache::from_environment_for(VERSION, &|name| env(name));
    config.resolve_console(&|name| env(name));
    config
}

/// `start`: the first generation boots the shell, a later one is a reload.
///
/// # Errors
///
/// The text to give the core as the last error: the link could not be opened, the script could not be found, or a reload did not
/// evaluate (the running generation stays).
pub(crate) fn start(table: CoreTable, ctx: GnoblinCtx, script: &Path) -> Result<(), String> {
    crate::table::remember(table);
    // Every line the engine logs from here is tagged with the generation being started.
    let previous = crate::table::log_context();
    set_log_context(ctx);
    let outcome = INSTANCE.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.as_mut() {
            None => boot(table, ctx, script).map(|instance| {
                *slot = Some(instance);
            }),
            Some(instance) => reload(instance, ctx),
        }
    });
    if outcome.is_err() {
        set_log_context(previous);
    }
    outcome
}

fn boot(table: CoreTable, ctx: GnoblinCtx, script: &Path) -> Result<Instance, String> {
    let script = std::fs::canonicalize(script).map_err(|error| format!("cannot read {}: {error}", script.display()))?;
    let serving: Serving = Rc::new(std::cell::Cell::new(ctx));
    let link = RelayedLink::open(table, Rc::clone(&serving))?;
    let role = JsRole::over_link(
        Box::new(link),
        JsRoleLinks::from_environment(&environment),
        role_config(&script),
    )
    .map_err(|error| format!("the shell could not start: {error}"))?;
    Ok(Instance { table, role, serving })
}

/// A reload: the engine evaluates the script in a staging generation and swaps it in when it was clean. The core's context moves to
/// the new generation only then, so a failed reload leaves the link, the log and the serving generation as they were.
fn reload(instance: &mut Instance, ctx: GnoblinCtx) -> Result<(), String> {
    let report = instance.role.reload_now();
    if !report.ok {
        let mut reasons: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
        reasons.extend(report.warnings.iter().cloned());
        return Err(if reasons.is_empty() {
            "the script did not evaluate".to_owned()
        } else {
            reasons.join("; ")
        });
    }
    instance.serving.set(ctx);
    Ok(())
}

/// `shutdown`: the generation that serves ends the shell. An older one, already replaced by a reload, has nothing left to end.
pub(crate) fn shutdown(ctx: GnoblinCtx) {
    INSTANCE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.as_ref().is_some_and(|instance| instance.serving.get() == ctx) {
            *slot = None;
        }
    });
}

/// `turn`: waits for the engine's connections or its next timer (at most `timeout_ms`), runs one frame, and asks the host for a
/// reload when the watcher saw a change. Returns the number of callbacks run, which the engine does not count, so zero.
///
/// # Errors
///
/// The shell ended: its link failed or the compositor sent something it cannot take. The shell is dropped.
pub(crate) fn turn(ctx: GnoblinCtx, timeout_ms: u32) -> Result<i32, String> {
    INSTANCE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(instance) = slot.as_mut() else {
            return Err("no JavaScript shell is running on this thread".to_owned());
        };
        let limit = Duration::from_millis(u64::from(timeout_ms));
        let wait = instance
            .role
            .next_wake()
            .map_or(FRAME_WAIT, |due| due.clamp(MIN_WAIT, FRAME_WAIT))
            .min(limit);
        let outcome = instance
            .role
            .wait_for_input(None, wait)
            .map(drop)
            .and_then(|()| instance.role.tick());
        if let Err(error) = outcome {
            let text = format!("the shell ended: {error}");
            *slot = None;
            return Err(text);
        }
        if instance.role.take_reload_wanted() {
            let code = instance.table.request_reload(instance.serving.get());
            if code != abi::E_OK {
                instance.table.log(
                    ctx,
                    abi::LOG_WARN,
                    &format!("the reload the watcher asked for was refused ({code})"),
                );
            }
        }
        Ok(0)
    })
}

/// Says why a turn ended the shell: the core's last error on this thread is what the host's loop reads, and the log has the line.
pub(crate) fn report_ended(ctx: GnoblinCtx, message: &str) {
    if let Some(table) = crate::table::remembered() {
        table.set_last_error(ctx, message);
        table.log(ctx, abi::LOG_ERROR, message);
    }
}
