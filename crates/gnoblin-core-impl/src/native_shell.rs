//! A native shell served in the compositor process: the body of the in-process shell thread.
//!
//! `gnoblin ./libshell.so` makes the compositor start a thread (`gnoblin_compositor::inproc_shell`) and run [`run`] on it.
//! That thread is the shell thread of design section 5.4: it is the only place the shell's code runs, and the compositor's
//! own thread never waits for it. The order is the one design section 19 gives the compositor edge:
//!
//! 1. the thread gives the core the tree link the compositor made for it ([`host::set_tree_link_provider`]), so the shell's
//!    first read of the tree (`tree_attach`, made by the SDK on the first call that needs the tree) opens it;
//! 2. the module is loaded ([`host::load`], which copies it, `dlopen`s it and runs its `init`) and then activated;
//! 3. the guardian is told the shell accepted its (empty) configuration and is ready ([`ShellReports`]);
//! 4. the thread turns the dispatch loop, serving reload requests between turns, until the compositor asks it to stop, the shell
//!    asks to exit, or the tree link ends. Its end is the shell's end and nothing more: the compositor keeps serving with its fallback window manager.
//!
//! A shell has no configuration document, so the revision it reports is `0`: the guardian records an accepted revision, which
//! is what lets it note that the shell came up, and no autostart entry, which is what a shell that reads no document has.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gnoblin_client::wire::ShellStatus;
use gnoblin_compositor::inproc_shell::{InProcContext, InProcShellSpec, ShellControl, ShellError, ShellReports};
use gnoblin_shell_host::reload::{ModuleWatch, ReloadController, ReloadStatus, ReloadTrigger};
use gnoblin_tree_client::tree_mirror::{LinkError, TreeLink};

use crate::host::{self, AttachMode, TreeLinkProvider};
use crate::role::ShellModule;
use crate::{guard, log};

/// How long one turn of the dispatch loop waits for something to deliver. A stop is noticed within this, and a link that
/// failed is noticed at once (the turn returns).
const TURN_MS: u32 = 100;

/// The configuration revision an in-process shell reports: it has no document.
const NO_DOCUMENT: u64 = 0;

/// The quiet after the last event of a rebuild before the watcher asks for a reload: the shell host's own constant.
const DEBOUNCE: Duration = gnoblin_shell_host::watch::ConfigWatcher::DEFAULT_DEBOUNCE;

/// How long the watcher thread waits for a change before it looks at whether the shell has ended.
const WATCH_TURN: Duration = Duration::from_millis(200);

/// The spec the compositor runs on the shell thread for `module`.
pub(crate) fn spec(module: ShellModule) -> InProcShellSpec {
    let name = module
        .path
        .file_stem()
        .map_or_else(|| "shell".to_owned(), |stem| stem.to_string_lossy().into_owned());
    InProcShellSpec::new(name, move |link, context| run(&module, link, &context)).with_control(Arc::new(ReloadControl))
}

/// What `shell.reload` and `shell.status` on the session's socket reach: this core's one reload controller.
struct ReloadControl;

impl ShellControl for ReloadControl {
    fn reload(&self) -> ShellStatus {
        let controller = host::reload_controller();
        controller.request(ReloadTrigger::User);
        status_of(&controller.status())
    }

    fn status(&self) -> ShellStatus {
        status_of(&host::reload_controller().status())
    }
}

/// The wire form of a controller's status.
pub(crate) fn status_of(status: &ReloadStatus) -> ShellStatus {
    ShellStatus {
        generation: status.generation,
        reloads: status.reloads,
        failures: status.failures,
        build_failures: status.build_failures,
        pending: status.pending.is_some(),
        last: status.last.as_ref().map(|record| record.outcome.clone()),
        failed: status.failed.clone(),
        summary: format!("{status}{}", crate::budget::status()),
    }
}

/// Applies the callback budget's rule to the request that waits for the shell thread's next turn: while the shell is degraded, a
/// reload the shell asked for itself is refused (the last good generation keeps running), and a reload somebody asked for (the
/// user, or the watcher seeing a rebuilt library) is a retry that clears the strikes.
fn gate_reload(controller: &ReloadController) {
    match controller.status().pending {
        Some(ReloadTrigger::Shell) if crate::budget::degraded() => {
            controller.take_request();
            log::warn!(
                "the shell asked for a reload while it is degraded by the callback budget: refused, the running generation \
                 keeps serving (`gnoblinctl shell reload` is a deliberate retry)"
            );
        }
        Some(ReloadTrigger::User | ReloadTrigger::Files(_)) => crate::budget::user_reload(),
        _ => {}
    }
}

/// What `tree_attach` opens: the one link the compositor made for this shell. A second attach (after the link ended) has
/// nothing to open.
pub(crate) struct OneLink(pub(crate) RefCell<Option<Box<dyn TreeLink>>>);

impl TreeLinkProvider for OneLink {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        self.0
            .borrow_mut()
            .take()
            .ok_or_else(|| LinkError("the tree link was used already".to_owned()))
    }
}

/// Runs on the shell thread: load, activate, report, serve, retire.
fn run(module: &ShellModule, link: Box<dyn TreeLink>, context: &InProcContext) -> Result<(), ShellError> {
    host::reset_session();
    host::set_tree_link_provider(Box::new(OneLink(RefCell::new(Some(link)))));
    host::set_verb_link_provider(Box::new(crate::verbs::OwnerSocket::from_environment()));
    host::set_slot_link_provider(std::sync::Arc::new(crate::slots::OwnerSlots::from_environment()));
    let outcome = serve(
        module,
        "in process",
        &|| context.stop.requested(),
        Arc::clone(&context.reports),
        Some(&|| context.relink()),
    );
    // The tree is detached by the compositor when this thread returns; what belongs to the shell instance goes with it.
    host::reset_session();
    outcome
}

/// Whether the module is a native library, which is the kind the host watches itself: a script's runtime watches the files
/// its generation imported and asks for the reload.
fn is_native(module: &ShellModule) -> bool {
    match module.language.as_deref() {
        Some(language) => language == "native",
        None => {
            let elf = std::fs::File::open(&module.path)
                .and_then(|mut file| {
                    let mut magic = [0_u8; 4];
                    std::io::Read::read_exact(&mut file, &mut magic).map(|()| magic)
                })
                .is_ok_and(|magic| magic == *b"\x7fELF");
            elf || module.path.extension().is_some_and(|extension| extension == "so")
        }
    }
}

/// The watcher of a native module (`--watch`): a thread that turns a rebuilt library into a reload request, until the shell
/// ends. The compositor never runs cargo; the developer loop (`cargo xtask watch-shell`, or `cargo build`) writes the library
/// and this sees it.
struct Watching {
    ended: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watching {
    fn start(module: &ShellModule, controller: ReloadController) -> Option<Watching> {
        if !module.watch || !is_native(module) {
            return None;
        }
        let watch = match ModuleWatch::new(std::slice::from_ref(&module.path), DEBOUNCE) {
            Ok(watch) => watch,
            Err(error) => {
                log::error!(
                    "cannot watch {}: {error}; the shell will not reload on its own",
                    module.path.display()
                );
                return None;
            }
        };
        log::info!("watching {} for a rebuild", module.path.display());
        let ended = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ended);
        let thread = std::thread::Builder::new()
            .name("gnoblin-shell-watch".to_owned())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    watch.forward_to(&controller, WATCH_TURN);
                }
            })
            .map_err(|error| log::error!("cannot start the watcher thread: {error}"))
            .ok()?;
        Some(Watching {
            ended,
            thread: Some(thread),
        })
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How to get a link to the tree that attached nothing yet, for the next start after a failed one: the compositor's own
/// in-process link is detached and replaced; a shell with no way to try again (the isolated shell, whose process the guardian
/// restarts) has none.
pub(crate) type Relink<'a> = &'a dyn Fn() -> Option<Box<dyn TreeLink>>;

/// The shell thread's whole life, shared by the compositor's thread and the isolated shell's main thread: start the module,
/// report it, turn the dispatch loop until something ends it, retire the generation.
///
/// A start that fails (a `shell.ts` with a syntax error, an `init` that fails) ends the shell when `relink` is `None`. With
/// one, the shell stays in a *failed* state instead: the tree is let go, so the compositor's own window management places
/// windows, the error is logged, reported to the guardian and shown by `shell status`, and the thread waits for a reload
/// (`gnoblinctl shell reload`, or the watcher of `--watch` and of scripts) to try the file again, on the same terms as the
/// first start.
pub(crate) fn serve(
    module: &ShellModule,
    placed: &str,
    stopped: &dyn Fn() -> bool,
    reports: Arc<dyn ShellReports>,
    relink: Option<Relink<'_>>,
) -> Result<(), ShellError> {
    // `report_config` and `report_config_rejected` of a shell with a document of its own reach the guardian through these.
    host::set_reports(Arc::clone(&reports));
    let mut shell = host::native_shell(&module.path);
    if let Some(language) = &module.language {
        shell = shell.with_language(language);
    }
    let generation = match shell.start() {
        Ok(generation) => generation,
        Err(error) => {
            let Some(relink) = relink else {
                return Err(error.to_string().into());
            };
            match await_fix(&mut shell, module, &error.to_string(), stopped, &reports, relink) {
                Waited::Fixed(generation) => generation,
                Waited::Stopped => {
                    shell.stop();
                    return Ok(());
                }
                Waited::Gave(why) => return Err(why.into()),
            }
        }
    };
    log::info!(
        "the shell {} serves {placed} (generation {generation})",
        module.path.display()
    );
    // A shell with a configuration document of its own told the guardian each revision it committed while it started
    // (`report_config`); the guardian then has the revision in force, and only the readiness is left. A shell with none is
    // revision zero.
    match host::reported_revision() {
        Some(revision) => reports.ready(revision),
        None => {
            reports.config_accepted(NO_DOCUMENT);
            reports.ready(NO_DOCUMENT);
        }
    }
    let watching = Watching::start(module, host::reload_controller());
    let outcome = turn_until_done(&mut shell, stopped);
    drop(watching);
    // The shutdown hooks run, the registrations are removed and the library is kept mapped (D16), whatever ended the loop.
    shell.stop();
    outcome
}

/// How a failed start ended.
enum Waited {
    /// A reload loaded the file: this generation runs.
    Fixed(gnoblin_shell_host::generation::GenerationId),
    /// The compositor stopped while the shell waited.
    Stopped,
    /// The shell cannot wait: the compositor has no tree link for another start. The text says why.
    Gave(String),
}

/// How long the failed shell sleeps between looks at the controller when nothing watches the file.
const FAILED_TURN: Duration = Duration::from_millis(100);

/// The watcher a failed start waits with: the module file, for a shell that was asked to watch and for a script (a script
/// that is running reloads on its own when its files change, so a script that never started does too). `GNOBLIN_JS_WATCH=0`
/// turns the script's off, as it does a running one's.
fn watch_failed(module: &ShellModule) -> Option<ModuleWatch> {
    let script = !is_native(module) && std::env::var("GNOBLIN_JS_WATCH").as_deref() != Ok("0");
    if !module.watch && !script {
        return None;
    }
    match ModuleWatch::new(std::slice::from_ref(&module.path), DEBOUNCE) {
        Ok(watch) => Some(watch),
        Err(error) => {
            log::error!(
                "cannot watch {}: {error}; fix it and run `gnoblinctl shell reload`",
                module.path.display()
            );
            None
        }
    }
}

/// The failed state of a shell that did not start: the tree is let go and a fresh link is ready for the next try, the error is
/// logged, reported to the guardian and kept for `shell status`, and the thread waits for a reload request (a user's, or the
/// watcher's) to run it through the loader. A reload that fails leaves it here with the new error.
fn await_fix(
    shell: &mut host::NativeShell,
    module: &ShellModule,
    first_error: &str,
    stopped: &dyn Fn() -> bool,
    reports: &Arc<dyn ShellReports>,
    relink: Relink<'_>,
) -> Waited {
    let controller = host::reload_controller();
    let watch = watch_failed(module);
    let mut error = first_error.to_owned();
    loop {
        // What the failed attempt attached is let go (the session forgets its tree and its link; the compositor detaches the
        // shell, so its own window management has the windows) and the next attempt gets a link of its own.
        host::reset_session();
        let Some(link) = relink() else {
            return if stopped() {
                Waited::Stopped
            } else {
                Waited::Gave(format!("{error}; and the compositor has no link for another start"))
            };
        };
        host::set_tree_link_provider(Box::new(OneLink(RefCell::new(Some(link)))));
        controller.record_start_failed(error.clone());
        let path = module.path.display();
        log::error!(
            "{path}: the shell did not start, so the compositor's own window management runs until it does; fix the file{}",
            if watch.is_some() {
                " (it is watched) or run `gnoblinctl shell reload`"
            } else {
                " and run `gnoblinctl shell reload`"
            }
        );
        reports.config_rejected(
            0,
            &format!(
                "the shell failed to start: {error}; the compositor's own window management runs until the file is fixed and the shell reloaded (`gnoblinctl shell reload`)"
            ),
        );
        // Wait for the request.
        let trigger = loop {
            if stopped() {
                return Waited::Stopped;
            }
            if let Some(watch) = &watch {
                watch.forward_to(&controller, FAILED_TURN);
            } else {
                std::thread::sleep(FAILED_TURN);
            }
            if let Some(trigger) = controller.take_request() {
                break trigger;
            }
        };
        match shell.reload(trigger) {
            Ok(reloaded) => return Waited::Fixed(reloaded.generation),
            Err(failure) => {
                error = failure.to_string();
                log::warn!("the shell still does not start: {failure}");
            }
        }
    }
}

/// Turns the dispatch loop. `Ok` is a clean end (the compositor stopped, or the shell asked to exit with code zero).
fn turn_until_done(shell: &mut host::NativeShell, stopped: &dyn Fn() -> bool) -> Result<(), ShellError> {
    loop {
        if stopped() {
            return Ok(());
        }
        let turn = shell.dispatch(TURN_MS);
        if let Some(code) = host::take_exit_request() {
            return if code == 0 {
                Ok(())
            } else {
                Err(format!("the shell asked to exit with code {code}").into())
            };
        }
        // A reload the shell asked for (or the watcher, or `gnoblinctl shell reload`): the controller stages, commits or abandons it
        // and logs the outcome; a failed one leaves the running generation as it was.
        gate_reload(&host::reload_controller());
        if let Some(reloaded) = shell.serve_requests() {
            match reloaded {
                Ok(reloaded) => log::info!("the shell reloaded: {reloaded:?}"),
                Err(failure) => log::warn!("the shell did not reload and keeps running: {failure}"),
            }
        }
        if turn.is_none_or(|turn| turn < 0) {
            if stopped() {
                return Ok(());
            }
            // The core's last-error text for this thread says why: the link ended (the compositor detached the shell for
            // not draining its mailbox, or went away) or the generation was retired under the loop.
            return Err(format!("the shell's dispatch loop ended: {}", guard::last_error()).into());
        }
    }
}
