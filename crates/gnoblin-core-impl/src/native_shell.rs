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
        summary: status.to_string(),
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
    let outcome = serve(
        module,
        "in process",
        &|| context.stop.requested(),
        context.reports.as_ref(),
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

/// The shell thread's whole life, shared by the compositor's thread and the isolated shell's main thread: start the module,
/// report it, turn the dispatch loop until something ends it, retire the generation.
pub(crate) fn serve(
    module: &ShellModule,
    placed: &str,
    stopped: &dyn Fn() -> bool,
    reports: &dyn ShellReports,
) -> Result<(), ShellError> {
    let mut shell = host::native_shell(&module.path);
    if let Some(language) = &module.language {
        shell = shell.with_language(language);
    }
    let generation = shell.start().map_err(|error| error.to_string())?;
    log::info!(
        "the shell {} serves {placed} (generation {generation})",
        module.path.display()
    );
    reports.config_accepted(NO_DOCUMENT);
    reports.ready(NO_DOCUMENT);
    let watching = Watching::start(module, host::reload_controller());
    let outcome = turn_until_done(&mut shell, stopped);
    drop(watching);
    // The shutdown hooks run, the registrations are removed and the library is kept mapped (D16), whatever ended the loop.
    shell.stop();
    outcome
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
