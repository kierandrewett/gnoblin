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
//! 4. the thread turns the dispatch loop until the compositor asks it to stop, the shell asks to exit, or the tree link
//!    ends. Its end is the shell's end and nothing more: the compositor keeps serving with its fallback window manager.
//!
//! A shell has no configuration document, so the revision it reports is `0`: the guardian records an accepted revision, which
//! is what lets it note that the shell came up, and no autostart entry, which is what a shell that reads no document has.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gnoblin_compositor::inproc_shell::{InProcContext, InProcShellSpec, ShellError};
use gnoblin_tree_client::tree_mirror::{LinkError, TreeLink};

use crate::host::{self, AttachMode, Copying, Module, TreeLinkProvider};
use crate::{guard, log};

/// How long one turn of the dispatch loop waits for something to deliver. A stop is noticed within this, and a link that
/// failed is noticed at once (the turn returns).
const TURN_MS: u32 = 100;

/// The configuration revision an in-process shell reports: it has no document.
const NO_DOCUMENT: u64 = 0;

/// The spec the compositor runs on the shell thread for the module at `path`.
pub(crate) fn spec(path: PathBuf) -> InProcShellSpec {
    let name = path
        .file_stem()
        .map_or_else(|| "shell".to_owned(), |stem| stem.to_string_lossy().into_owned());
    InProcShellSpec::new(name, move |link, context| run(&path, link, &context))
}

/// What `tree_attach` opens: the one link the compositor made for this shell. A second attach (after the link ended) has
/// nothing to open.
struct OneLink(RefCell<Option<Box<dyn TreeLink>>>);

impl TreeLinkProvider for OneLink {
    fn open(&self, _mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError> {
        self.0
            .borrow_mut()
            .take()
            .ok_or_else(|| LinkError("the in-process tree link was used already".to_owned()))
    }
}

/// Runs on the shell thread: load, activate, report, serve, retire.
fn run(path: &Path, link: Box<dyn TreeLink>, context: &InProcContext) -> Result<(), ShellError> {
    host::reset_session();
    host::set_tree_link_provider(Box::new(OneLink(RefCell::new(Some(link)))));
    let outcome = serve(path, context);
    // The tree is detached by the compositor when this thread returns; what belongs to the shell instance goes with it.
    host::reset_session();
    outcome
}

fn serve(path: &Path, context: &InProcContext) -> Result<(), ShellError> {
    let module = host::load(path, Copying::Unique, 0).map_err(|error| error.to_string())?;
    module.activate()?;
    log::info!(
        shell = module.label(),
        "the native shell {} serves in process (generation {})",
        path.display(),
        module.generation()
    );
    context.reports.config_accepted(NO_DOCUMENT);
    context.reports.ready(NO_DOCUMENT);
    let outcome = turn_until_done(&module, context);
    // The shutdown hooks run, the registrations are removed and the library is kept mapped (D16), whatever ended the loop.
    module.retire();
    outcome
}

/// Turns the dispatch loop. `Ok` is a clean end (the compositor stopped, or the shell asked to exit with code zero).
fn turn_until_done(module: &Module, context: &InProcContext) -> Result<(), ShellError> {
    loop {
        if context.stop.requested() {
            return Ok(());
        }
        let turn = module.dispatch(TURN_MS);
        if let Some(code) = host::take_exit_request() {
            return if code == 0 {
                Ok(())
            } else {
                Err(format!("the shell asked to exit with code {code}").into())
            };
        }
        if host::take_reload_request() {
            // Reloading a native shell in place is the reload protocol's work (design section 8.1), which this build does not
            // have. It says so rather than ignoring the request.
            log::warn!("the shell asked for a reload; native hot reload is not available yet, so it keeps running");
        }
        if turn < 0 {
            if context.stop.requested() {
                return Ok(());
            }
            // The core's last-error text for this thread says why: the link ended (the compositor detached the shell for
            // not draining its mailbox, or went away) or the generation was retired under the loop.
            return Err(format!("the shell's dispatch loop ended: {}", guard::last_error()).into());
        }
    }
}
