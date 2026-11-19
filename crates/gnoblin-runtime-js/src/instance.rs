//! The JavaScript shell this thread runs, and what each entry point of the runtime does to it.
//!
//! One shell instance serves for the life of the process: the engine's [`JsRole`] keeps the tree mirror, the nodes the shell
//! made, the workspaces, the claims and the watcher. A generation is one evaluation of the script. The first `start` boots the
//! role over the core's link and evaluates the script, and every later `start` evaluates it in a staging generation beside the
//! one that serves and swaps it in only if it evaluated cleanly, which is the engine's own reload (`JsRole::reload_now`). The
//! core's contexts follow: the context of the generation that serves is the one the link and the log use, and it moves to the new
//! one when the swap commits.
//!
//! The role owns what the socket role of the guardian owned: the configuration document (`owns_document`), each revision it commits is
//! reported to the guardian through the core (`report_config`, which the core turns into the `autostart` lines and `config-accepted`
//! the guardian reads), and `gnoblinctl config reload` is answered: the compositor raises `session.reload_requested`, the role asks
//! the core for a reload, and the generation that results commits a revision (which answers the caller) or is refused
//! (`config.reject`, and `report_config_rejected` for the guardian).
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
use gnoblin_js::{JsBytecodeCache, JsCommitLog, JsRole, JsRoleConfig, JsRoleLinks, JsShellError, stock_search_path};

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
    /// The documents the shell committed that the guardian has not been told of.
    commits: JsCommitLog,
    /// Whether a reload of the configuration the compositor asked for (`gnoblinctl config reload`) is waiting for its answer: a
    /// commit, or a refusal.
    answering: bool,
    /// The core's wake descriptor, once a script has asked the core for something that is answered through its dispatch (a verb, a
    /// signal, the std library): the turn sleeps on it beside the engine's connections, so an answer from another thread starts the
    /// next frame at once and not at the end of the frame wait.
    wake: Option<std::os::fd::OwnedFd>,
}

thread_local! {
    static INSTANCE: RefCell<Option<Instance>> = const { RefCell::new(None) };
}

fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// The engine's configuration for the script at `script`: imports resolve under its directory, the stock library is the installed
/// data (`share/gnoblin/js`, found from the prefix the core and this process were installed in), the cache is the runtime's, the
/// role owns the configuration document, and the watcher asks the host to reload.
fn role_config(table: CoreTable, script: &Path, commits: &JsCommitLog) -> JsRoleConfig {
    let env = &environment;
    let dir = script.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut config = JsRoleConfig::new(dir, None);
    let anchors: Vec<PathBuf> = [table.core_file(), std::env::current_exe().ok()]
        .into_iter()
        .flatten()
        .collect();
    config.shell.stock_dirs = stock_search_path(env, &anchors);
    config.entry = script.to_path_buf();
    config.watch = env("GNOBLIN_JS_WATCH").as_deref() != Some("0");
    config.external_reload = true;
    config.owns_document = true;
    config.commits = Some(commits.clone());
    config.shell.xr_declared = env("GNOBLIN_XR").as_deref() == Some("1");
    config.shell.js.cache = JsBytecodeCache::from_environment_for(VERSION, &|name| env(name));
    config.resolve_console(&|name| env(name));
    // A host that asks for the failure of a start as data wants the failure of the entry, not the starter that would run in its place.
    if env(START_FAILURE).as_deref() == Some("json") {
        config.shell.starter = None;
    }
    // Every generation gets `__gnoblinStd`, the table of `libgnoblin-std` for the scripts that import `gnoblin-std/native`.
    config.shell.js.natives.push(crate::std_native::native());
    // And `__gnoblinCore`, the core's verbs and signals of the compositor's `gnoblin` namespace (`verbs` of `gnoblin-core`).
    config.shell.js.natives.push(crate::core_native::native());
    config
}

/// Tells the guardian the revision the shell committed last since the previous call, and returns it. `None` when the shell
/// committed nothing.
///
/// The core turns the report into the enabled `autostart` lines of the document and `config-accepted`. Only the last commit is
/// reported when there were several: the guardian reconciles what it runs with the final document, and an intermediate one would
/// start programs the next stops.
fn publish(table: CoreTable, ctx: GnoblinCtx, commits: &JsCommitLog) -> Option<u64> {
    let records = std::mem::take(&mut *commits.borrow_mut());
    let last = records.last()?;
    for notice in &last.notices {
        table.log(ctx, abi::LOG_WARN, &format!("js: not applied: {notice}"));
    }
    if let Err(message) = table.report_config(ctx, last.revision, &last.document) {
        table.log(
            ctx,
            abi::LOG_WARN,
            &format!(
                "js: the guardian could not be told of configuration revision {}: {message}",
                last.revision
            ),
        );
    }
    table.log(
        ctx,
        abi::LOG_INFO,
        &format!("js: configuration revision {} committed", last.revision),
    );
    Some(last.revision)
}

/// `start`: the first generation boots the shell, a later one is a reload.
///
/// # Errors
///
/// The text to give the core as the last error: the link could not be opened, the script could not be found, or a reload did not
/// evaluate (the running generation stays).
pub(crate) fn start(table: CoreTable, ctx: GnoblinCtx, script: &Path) -> Result<(), String> {
    crate::table::remember(table);
    crate::std_native::set_anchor(table.anchor());
    // A script that evaluates now belongs to this generation of the core, which is not the serving one until the swap commits.
    let before = crate::std_native::context();
    crate::std_native::set_context(ctx);
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
        crate::std_native::set_context(before);
    }
    outcome
}

/// The environment variable that asks for the failure of a start as data.
const START_FAILURE: &str = "GNOBLIN_START_FAILURE";

/// The words a shell that could not start is reported with. A host that sets `GNOBLIN_START_FAILURE=json` (the dry run of
/// `gnoblinctl check`) gets the failure of a script as one JSON object, `{"kind", "file", "line", "column", "message"}`, with the
/// position in the author's own coordinates, and no starter runs in the place of an entry that fails; every other host gets a sentence.
fn start_failure(error: &JsShellError) -> String {
    if let JsShellError::Js(error) = error
        && environment(START_FAILURE).as_deref() == Some("json")
    {
        let position = |value: Option<String>| value.unwrap_or_else(|| "null".to_owned());
        let at = error.location.as_ref();
        return format!(
            "{{\"kind\":{},\"file\":{},\"line\":{},\"column\":{},\"message\":{}}}",
            json_text(error.kind.word()),
            position(at.map(|at| json_text(&at.file.display().to_string()))),
            position(at.map(|at| at.line.to_string())),
            position(at.map(|at| at.column.to_string())),
            json_text(&error.message),
        );
    }
    format!("the shell could not start: {error}")
}

/// `text` as a JSON string.
fn json_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn boot(table: CoreTable, ctx: GnoblinCtx, script: &Path) -> Result<Instance, String> {
    let script = std::fs::canonicalize(script).map_err(|error| format!("cannot read {}: {error}", script.display()))?;
    let serving: Serving = Rc::new(std::cell::Cell::new(ctx));
    let link = RelayedLink::open(table, Rc::clone(&serving))?;
    let commits = JsCommitLog::default();
    let role = JsRole::over_link(
        Box::new(link),
        JsRoleLinks::from_environment(&environment),
        role_config(table, &script, &commits),
    )
    .map_err(|error| start_failure(&error))?;
    // The shell committed its document while it started. The guardian waits for the revision before it starts what reads the
    // document, and the host reports the shell ready once this returns.
    publish(table, ctx, &commits);
    Ok(Instance {
        table,
        role,
        serving,
        commits,
        answering: false,
        wake: None,
    })
}

/// A reload: the engine evaluates the script in a staging generation and swaps it in when it was clean. The core's context moves to
/// the new generation only then, so a failed reload leaves the link, the log and the serving generation as they were.
///
/// A reload that commits a revision tells the guardian. One that does not evaluate, or whose document the compositor did not
/// take, is refused: the guardian hears `config-rejected`, and the caller of `gnoblinctl config reload` (the compositor holds it
/// until a commit or a refusal answers) reads the reason. The serving generation and the last accepted document stay.
fn reload(instance: &mut Instance, ctx: GnoblinCtx) -> Result<(), String> {
    let answering = std::mem::take(&mut instance.answering);
    let seen = instance.role.diagnostics_reported();
    let report = instance.role.reload_now();
    let committed = publish(instance.table, ctx, &instance.commits);
    // A shell with no connection for its document has nothing to commit, and a reload that evaluated is a reload.
    let refused = !report.ok || (committed.is_none() && instance.role.commits_document());
    if !refused {
        instance.serving.set(ctx);
        return Ok(());
    }
    let mut reasons: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    reasons.extend(report.warnings.iter().cloned());
    let later = instance.role.diagnostics_since(seen).join("; ");
    if !later.is_empty() {
        reasons.push(later);
    }
    if reasons.is_empty() {
        reasons.push("nothing was committed".to_owned());
    }
    let reason = reasons.join("; ");
    // The staged context is abandoned by the host; the one that serves is the one that can still speak.
    let speaking = instance.serving.get();
    if let Err(message) = instance.table.report_config_rejected(speaking, 0, &reason) {
        instance.table.log(
            speaking,
            abi::LOG_WARN,
            &format!("js: the guardian could not be told of the refused reload: {message}"),
        );
    }
    if answering && let Err(error) = instance.role.reject_config_reload(&reason) {
        instance.table.log(
            speaking,
            abi::LOG_WARN,
            &format!("js: the caller of the reload could not be told why it was refused: {error}"),
        );
    }
    Err(reason)
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
        if instance.wake.is_none() && crate::std_native::used() {
            instance.wake = instance.table.wake_fd(instance.serving.get());
        }
        let limit = Duration::from_millis(u64::from(timeout_ms));
        let wait = instance
            .role
            .next_wake()
            .map_or(FRAME_WAIT, |due| due.clamp(MIN_WAIT, FRAME_WAIT))
            .min(limit);
        let outcome = instance
            .role
            .wait_for_input(instance.wake.as_ref().map(std::os::fd::AsFd::as_fd), wait)
            .map(drop)
            .and_then(|()| instance.role.tick());
        if let Err(error) = outcome {
            let text = format!("the shell ended: {error}");
            *slot = None;
            return Err(text);
        }
        // What the std library announces (a window opened, a layout is asked) is delivered by the core's dispatch, which this loop runs
        // for it; a script that never imported std has nothing queued and the call is skipped.
        if crate::std_native::used() {
            let serving = instance.serving.get();
            let enter = RoleEnter(&instance.role);
            crate::std_native::with_enter(&enter, || instance.table.dispatch(serving));
        }
        let watcher = instance.role.take_reload_wanted();
        let requested = instance.role.take_config_reload_asked();
        if watcher || requested {
            let code = instance.table.request_reload(instance.serving.get());
            if code != abi::E_OK {
                instance.table.log(
                    ctx,
                    abi::LOG_WARN,
                    &format!("the reload that was asked for was refused ({code})"),
                );
                if requested {
                    let reason = format!("the host refused the reload ({code})");
                    if let Err(error) = instance.role.reject_config_reload(&reason) {
                        instance.table.log(ctx, abi::LOG_WARN, &format!("js: {error}"));
                    }
                }
            } else if requested {
                // The caller of `gnoblinctl config reload` waits for the generation this request makes.
                instance.answering = true;
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

/// Runs `f` with the way into the generation that serves, from outside the runtime's turn: the host's own dispatch, which drains the
/// core's queue after the turn, calls the library's callbacks too. `None` when no shell is running or the turn holds it.
pub(crate) fn with_enter<R>(f: impl FnOnce(&dyn crate::std_native::Enter) -> R) -> Option<R> {
    INSTANCE.with(|slot| {
        let slot = slot.try_borrow().ok()?;
        let instance = slot.as_ref()?;
        Some(f(&RoleEnter(&instance.role)))
    })
}

/// How a callback of the std library enters the running generation: the engine's own entry, under the listener budget.
struct RoleEnter<'a>(&'a JsRole);

impl crate::std_native::Enter for RoleEnter<'_> {
    fn enter(
        &self,
        body: &mut dyn for<'js> FnMut(&gnoblin_js::rquickjs::Ctx<'js>) -> gnoblin_js::rquickjs::Result<()>,
    ) -> Result<(), String> {
        self.0.call_script(|ctx| body(ctx)).map_err(|error| error.to_string())
    }
}
