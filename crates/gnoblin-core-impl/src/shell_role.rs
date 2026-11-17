//! The shell host in a process of its own: `gnoblin --role shell PATH` (`--isolate`).
//!
//! An isolated shell is the same shell host as the in-process one, with two differences. Its tree link is the compositor's
//! socket instead of the in-process queue, and a crash in its code ends this process instead of the compositor's. It is a
//! privileged child of the guardian: the guardian hands it its end of a stream the compositor serves as the owner's
//! ([`gnoblin_proto::privilege`]) and a report descriptor it says `ready` on, as the script runtime role did.
//!
//! What ends it: the guardian's `SIGTERM` (the kernel's default action, no handler), the link ending (the compositor went away,
//! or detached the shell for not draining its mailbox) or the shell asking to exit. A failure is an exit code 1 that the
//! guardian restarts under its backoff, as it does the services role.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use gnoblin_client::{Client, Inherited, Reporter, UnixTransport, inherited_stream, privilege};
use gnoblin_compositor::inproc_shell::{GuardianReports, ShellReports as _};
use gnoblin_log::ProcessRole;
use gnoblin_log::init::{LogOptions, LogSettings, init};
use gnoblin_proto::Version;
use gnoblin_tree_client::tree_link::SocketTreeLink;
use gnoblin_tree_client::tree_mirror::TreeLink;

use crate::native_shell::{OneLink, serve};
use crate::role::{EXIT_FAILED, ShellModule};
use crate::{host, log};

/// Serves the shell role and returns the process's exit code.
pub(crate) fn serve_isolated(module: ShellModule) -> i32 {
    let _log = init(
        ProcessRole::Runtime,
        &LogOptions::from_environment(&LogSettings::default()),
    );
    let link = match connect() {
        Ok(link) => link,
        Err(message) => {
            log::error!("the isolated shell cannot connect to the compositor: {message}");
            return EXIT_FAILED;
        }
    };
    let reports = GuardianReports::new(Arc::new(Mutex::new(Reporter::new(|name| std::env::var(name).ok()))));
    host::reset_session();
    host::set_tree_link_provider(Box::new(OneLink(RefCell::new(Some(link)))));
    let outcome = serve(&module, "in its own process", &|| false, &reports);
    host::reset_session();
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            log::error!("the isolated shell ended: {error}");
            reports.config_rejected(0, &error.to_string());
            EXIT_FAILED
        }
    }
}

/// The tree link over the descriptor the guardian inherited this process: the stream is the owner's, so the compositor lets it
/// attach the tree after the version handshake.
fn connect() -> Result<Box<dyn TreeLink>, String> {
    let stream = inherited_stream(privilege::FD_VARIABLE, |name| std::env::var(name).ok())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| Inherited::Absent.to_string())?;
    let transport = UnixTransport::from_stream(stream).map_err(|error| error.to_string())?;
    let version = {
        let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
        Version::new(major, minor, patch)
    };
    let client = Client::connect(transport, version).map_err(|error| error.to_string())?;
    let stream = client.into_inner().into_stream().map_err(|error| error.to_string())?;
    let link = SocketTreeLink::new(stream).map_err(|error| error.to_string())?;
    Ok(Box::new(link))
}
