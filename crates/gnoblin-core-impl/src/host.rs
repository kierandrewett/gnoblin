//! The host's side of the core: what the compositor, the loader and the tests call, as Rust.
//!
//! A shell sees the C table. The code that hosts shells (the loader, and later the compositor edge that publishes events)
//! sees this module: load a module, activate or retire a generation, publish a signal into the shell's mailbox, read the
//! requests a shell made (reload, exit), and lend owned buffers back to a shell.

use std::sync::{LazyLock, Mutex, PoisonError};

use std::path::Path;

use gnoblin_compositor::inproc_shell::ShellReports;
use gnoblin_core_abi::{GnoblinApi, GnoblinCtx};
use gnoblin_shell_host::native::NativeLoader;
pub use gnoblin_shell_host::native::{Copying, LoadError, UnloadRefused};
pub use gnoblin_shell_host::reload::{
    BuildFailure, BuildStep, Built, ModuleWatch, ReloadController, ReloadError, ReloadFailure, ReloadRecord,
    ReloadStatus, ReloadTrigger, Reloaded,
};
pub use gnoblin_shell_host::retire::{RetainedStats, RetirePolicy};

use crate::catalog;
use crate::registry;

pub use crate::memory::{lend_bytes, lend_text};
pub use crate::registry::{GenerationState, OwnedValue, ShellEvent, SignalDelivery};
pub use crate::session::{AttachMode, TreeLinkProvider};

/// One loaded generation of a native shell, in this core's host.
pub type Module = gnoblin_shell_host::native::NativeModule<ShellEvent>;

/// The native shell of this core: a running generation and the reload protocol over it.
pub type NativeShell = gnoblin_shell_host::reload::NativeShell<ShellEvent>;

/// The one loader of the process: it loads native shell modules into the host this core owns, initialises them with this
/// core's function table, and keeps what it cannot unmap on the bounded retire list.
static LOADER: NativeLoader<ShellEvent> = NativeLoader::new(&registry::HOST, &crate::table::API);

/// Loads a native shell module into a staged generation and runs its `init`. See [`NativeLoader::load`].
///
/// # Errors
///
/// [`LoadError`]: the retire list is full, the copy or `dlopen` failed, the file is not a shell, or `init` failed.
pub fn load(path: &Path, copying: Copying, reload_count: u32) -> Result<Module, LoadError> {
    LOADER.load(path, copying, reload_count)
}

/// Gives the shell thread a way to reach the compositor's tree: what `tree_attach` opens. Call it on the shell thread (the thread
/// that loads the shell) before the shell attaches. The provider is not `Send`, so a link over a non-`Send` handle is welcome; the
/// in-process compositor edge provides one over its tree queue, and the tests one over the fake compositor.
pub fn set_tree_link_provider(provider: Box<dyn TreeLinkProvider>) {
    crate::session::with(|session| session.set_provider(provider));
}

/// Gives the shell thread a way to reach the compositor's verbs (`call` and `subscribe` of the table): what opens the first time a
/// shell calls a verb or subscribes to a signal of the `gnoblin` namespace. Call it on the shell thread, as
/// [`set_tree_link_provider`]. The in-process compositor edge provides the owner socket ([`crate::verbs::OwnerSocket`]); a test
/// provides a link that answers from a script.
pub fn set_verb_link_provider(provider: Box<dyn crate::verbs::VerbLinkProvider>) {
    crate::session::with(|session| session.set_verb_provider(provider));
}

/// Gives the shell thread a way to reach the compositor's configuration document: what a shell without a runtime of its own commits the
/// settings it wrote through ([`commit_document_changes`]). Call it on the shell thread, as [`set_tree_link_provider`]. The in-process
/// compositor edge provides the owner socket ([`crate::verbs::OwnerSocket`]); a test provides a link that records the commits.
pub fn set_document_link_provider(provider: Box<dyn crate::document::DocumentLinkProvider>) {
    crate::document::set_provider(provider);
}

/// Commits to the compositor what the shell of the generation `ctx` wrote to the settings of the devices and of the compositor since the
/// last call, if it wrote something and no runtime of its own commits for it (see [`crate::document`]). `ctx` is the generation that serves.
/// The shell host calls it after the start of the shell and after each turn of its dispatch loop; a shell calls nothing.
pub fn commit_document_changes(ctx: GnoblinCtx) {
    crate::document::commit_changes(ctx);
}

/// Gives the shell thread a way to reach the compositor's dialog slot: what opens the first time a shell wants to observe the
/// compositor's own dialogs (`gnoblin.dialogs`). The in-process compositor edge provides the owner socket
/// ([`crate::slots::OwnerSlots`]); a test provides a link that offers requests from a script. The provider makes the link on the
/// connection's own thread.
pub fn set_slot_link_provider(provider: std::sync::Arc<dyn crate::slots::SlotLinkProvider>) {
    crate::session::with(|session| session.set_slot_provider(provider));
}

/// Sets how long a dialog request waits for the shell's verdict before the slot connection refuses it, and how long a claim stands while no
/// generation observes the slot. The defaults are 150 ms and 10 s. Tests shorten them.
pub fn set_slot_timings(timings: crate::slots::SlotTimings) {
    crate::session::with(|session| session.set_slot_timings(timings));
}

/// Replaces the clock this thread's timers read. Tests give a manual one so that no assertion rests on how fast the machine is.
pub fn set_clock(clock: std::sync::Arc<dyn gnoblin_shell_host::supervisor::Clock>) {
    crate::session::with(|session| session.set_clock(clock));
}

/// Ends the shell instance of this thread: the tree is detached and everything that belonged to it is dropped (the compositor
/// releases a connection's claims by itself). The provider and the clock stay. For a host that restarts its shell and for tests.
pub fn reset_session() {
    crate::session::with(crate::session::Session::reset);
    crate::budget::reset();
    crate::document::reset();
}

/// What the callback budget has seen: the limits, the strikes, whether the shell is degraded, and what ran past its limit last.
/// The summary of `gnoblinctl shell status` carries the same text. See [`crate::budget`].
pub fn budget_status() -> crate::budget::BudgetStatus {
    crate::budget::status()
}

/// Replaces the callback budget's limits (the design's are 2 s for a callback and 5 s for an `init`): for the tests and the scenario
/// driver, which want a budget that a test can pass in milliseconds. A session always has the design's.
pub fn set_budget_limits(limits: crate::budget::BudgetLimits) {
    crate::budget::set_limits(limits);
}

/// Opens an active generation of this thread's shell instance, for a host that runs a shell's engine in its own process and so has no
/// module to load: the tests of the JavaScript engine, which drive it over this core and the std library. `label` names it in the log.
///
/// # Errors
///
/// The generation could not be activated.
pub fn open_generation(label: &str, language: u32) -> Result<GnoblinCtx, String> {
    let ctx = crate::registry::new_context(label, language, 0, false);
    crate::registry::activate(ctx)?;
    Ok(ctx)
}

/// Opens a generation that is only staged, as the loader does for a reload while the old one still serves: its claims are kept and sent
/// when [`take_over`] makes it the active one. `label` names it in the log.
pub fn stage_generation(label: &str, language: u32) -> GnoblinCtx {
    crate::registry::new_context(label, language, 1, false)
}

/// A staged generation takes over from the active one: the claims of the old generation are released and the new one's go out behind
/// them in the same transaction. The old generation is ended with [`close_generation`] afterwards, as a reload does.
///
/// # Errors
///
/// The generation is not staged.
pub fn take_over(ctx: GnoblinCtx) -> Result<(), String> {
    crate::registry::activate(ctx)
}

/// Ends a generation [`open_generation`] made: its shutdown hooks run and everything it registered is removed. Returns how many
/// registrations were removed.
pub fn close_generation(ctx: GnoblinCtx) -> usize {
    crate::registry::run_shutdown_hooks(ctx);
    crate::registry::finish_retire(ctx)
}

/// How many module libraries are kept mapped after retirement, abandonment or a failed load.
pub fn retired_libraries() -> usize {
    LOADER.retained_libraries()
}

/// The numbers `gnoblinctl status` shows about retention: the count, the mapped size and the limit.
pub fn retained_stats() -> RetainedStats {
    LOADER.retained_stats()
}

/// The retention policy in force (default: keep at most 32 libraries, never unload).
pub fn retire_policy() -> RetirePolicy {
    LOADER.retire_list().policy()
}

/// Replaces the retention policy. A reload is refused once `max_retained` libraries are kept; `allow_unload` lets a
/// module that proves it is unreachable be unmapped (off by default).
pub fn set_retire_policy(policy: RetirePolicy) {
    LOADER.retire_list().set_policy(policy);
}

/// The function table this core hands to shells.
pub fn api() -> &'static GnoblinApi {
    &crate::table::API
}

/// A hash of the schema catalog this core was built with.
pub fn catalog_hash() -> u64 {
    catalog::CATALOG_HASH
}

/// Queues a signal for the shell, by catalog class and signal id. Any thread; wakes the shell's dispatch loop.
pub fn publish(delivery: SignalDelivery) {
    registry::publish(delivery);
}

/// Queues a signal for the shell by name (`("lock", "requested")`), for a caller that has no ids.
///
/// # Errors
///
/// The class or signal is not in the catalog.
pub fn publish_named(class: &str, signal: &str, node: u64, values: Vec<OwnedValue>) -> Result<(), String> {
    let class_id = catalog::class_id(class).ok_or_else(|| format!("no class `{class}` in the catalog"))?;
    let signal_id =
        catalog::signal_id(class_id, signal).ok_or_else(|| format!("class `{class}` has no signal `{signal}`"))?;
    publish(SignalDelivery {
        class: class_id,
        signal: signal_id,
        node,
        values,
    });
    Ok(())
}

/// How many published signals no `dispatch` has taken yet.
pub fn queued_events() -> usize {
    registry::queued_events()
}

static EXIT_REQUEST: Mutex<Option<i32>> = Mutex::new(None);

/// What every request for a reload goes through: a shell's `request_reload`, `gnoblinctl shell reload`, the file watcher. A
/// handle on the one controller of this core; clones share it. See [`ReloadController`].
pub fn reload_controller() -> ReloadController {
    static CONTROLLER: LazyLock<ReloadController> = LazyLock::new(ReloadController::new);
    CONTROLLER.clone()
}

/// The reload request nobody has served yet, once: what asked for it. The host reloads between dispatches
/// ([`NativeShell::serve_requests`] does both). A request is made with [`reload_controller`], by the shell through the table
/// (`request_reload`), and by the watcher.
pub fn take_reload_request() -> Option<ReloadTrigger> {
    reload_controller().take_request()
}

/// A native shell that loads `path` into this core's host and reloads through this core's controller: the shell thread's
/// handle on the running generation. Call [`NativeShell::start`] on the shell thread, after
/// [`set_tree_link_provider`], and [`NativeShell::serve_requests`] between dispatches.
pub fn native_shell(path: impl Into<std::path::PathBuf>) -> NativeShell {
    NativeShell::new(&LOADER, path, reload_controller())
}

/// The exit code a shell asked for, once.
pub fn take_exit_request() -> Option<i32> {
    EXIT_REQUEST.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// `request_reload`.
pub(crate) fn request_reload(ctx: GnoblinCtx) -> i32 {
    if let Err(code) = check_live(ctx, "request_reload") {
        return code;
    }
    reload_controller().request(ReloadTrigger::Shell);
    gnoblin_core_abi::E_OK
}

/// `request_exit`.
pub(crate) fn request_exit(ctx: GnoblinCtx, code: i32) -> i32 {
    if let Err(rc) = check_live(ctx, "request_exit") {
        return rc;
    }
    *EXIT_REQUEST.lock().unwrap_or_else(PoisonError::into_inner) = Some(code);
    gnoblin_core_abi::E_OK
}

/// Gives the shell thread where its reports to the guardian go: `report_config` and `report_config_rejected` of the table, and the
/// host's own `ready`. Call it on the shell thread, as [`set_tree_link_provider`]. A shell run with no guardian to tell reports
/// nothing.
pub fn set_reports(reports: std::sync::Arc<dyn ShellReports>) {
    crate::session::with(|session| session.set_reports(reports));
}

/// The configuration revision the shell of this thread reported accepted (`report_config`), if it reported one. A shell that has no
/// configuration document of its own reports none.
pub fn reported_revision() -> Option<u64> {
    crate::session::with(|session| session.reported_revision())
}

/// `report_config`: the shell committed `document` as configuration `revision`, and the guardian is told.
pub(crate) fn report_config(ctx: GnoblinCtx, revision: u64, document: &[u8]) -> i32 {
    if let Err(code) = check_live(ctx, "report_config") {
        return code;
    }
    // The report is written after the borrow of the session ends: the sink writes to a descriptor and may call anything.
    let reports = crate::session::with(|session| {
        session.note_reported(revision);
        session.reports()
    });
    if let Some(reports) = reports {
        reports.config_committed(revision, document);
    }
    gnoblin_core_abi::E_OK
}

/// `report_config_rejected`: the shell refused a configuration revision and the last accepted one stays in force.
pub(crate) fn report_config_rejected(ctx: GnoblinCtx, revision: u64, reason: &str) -> i32 {
    if let Err(code) = check_live(ctx, "report_config_rejected") {
        return code;
    }
    if let Some(reports) = crate::session::with(|session| session.reports()) {
        reports.config_rejected(revision, reason);
    }
    gnoblin_core_abi::E_OK
}

/// A request is only honoured from a generation that has not retired; a retired one is counted as a stale call.
fn check_live(ctx: GnoblinCtx, what: &str) -> Result<(), i32> {
    registry::check_ctx(ctx, what, false).map(|_| ())
}
