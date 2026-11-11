//! The composition root's body: one bus connection, an optional compositor connection, one
//! service loop.
//!
//! `main` decides where the bus and the compositor are and when the process is ready; this is
//! what is then done with them. It is a function of the connections it is given, so a test can
//! serve the same objects on a private bus against a live compositor rather than a copy of this
//! wiring.
//!
//! # The order, and why
//!
//! 1. **The backends from the shell, if there is one.** Every interface can need the shell, so
//!    it is built into the backends before anything is exported. Without a shell the portal
//!    still starts: it serves what needs no prompt and refuses the rest.
//! 2. **The objects, then the name.** A caller that resolves the name must find an object
//!    behind it; a name without an object is a method call that never answers.
//! 3. **The loop on its own thread, the bus loop on the caller's.** The IPC client is a
//!    blocking reader, and the bus connection has its own executor; one thread each keeps
//!    either from stalling the other.

use std::path::PathBuf;
use std::sync::Arc;

use futures_lite::future;
use gnoblin_client::Transport;
use zbus::Connection;
use zbus::fdo::RequestNameReply;

use crate::backends::{Backends, CaptureBackend, EisBackend, Missing};
use crate::capture_relay::CaptureRelay;
use crate::consent::Host;
use crate::host::{IpcHost, IpcLoop, NoShell};
use crate::iface::{BUS_NAME, Portal};
use crate::log;

/// How long one consent prompt stays open, in milliseconds.
///
/// Two minutes is the frontend's own default for a response, so a portal that waits longer
/// than the frontend is a dialogue whose answer nobody can use.
pub const CONSENT_LIFETIME: u64 = 120_000;

/// What the process was started with, besides its connections.
#[derive(Clone, Debug)]
pub struct Options {
    /// The folder screenshots are written to, if the user has one.
    pub pictures: Option<PathBuf>,
    /// Whether Gnoblin serves the Secret portal: `keyring.enabled`.
    pub secret_served: bool,
    /// How long one consent prompt stays open, in milliseconds.
    pub consent_lifetime: u64,
}

/// Export the objects, own the bus name and serve until the bus connection ends.
///
/// `shell` is the privileged compositor connection and the loop that reads it. `ready` is
/// called once the portal is serving, and also when another portal already owns the name and
/// this one is about to leave: either way the caller's start sequence may go on.
///
/// # Errors
///
/// A bus that cannot export the objects or own the name for a reason other than another
/// instance owning it, or a loop thread that could not be started.
pub async fn serve<T>(
    connection: Connection,
    shell: Option<(IpcHost, IpcLoop<T>)>,
    options: Options,
    ready: &mut dyn FnMut(),
) -> Result<(), Box<dyn std::error::Error>>
where
    T: Transport + Send + 'static,
{
    let (host, loop_, capture, eis) = match shell {
        Some((host, loop_)) => {
            let capture = Arc::new(CaptureRelay::new(host.clone(), options.pictures)) as Arc<dyn CaptureBackend>;
            let eis = Arc::new(crate::eis_relay::EisRelay::new(host.clone())) as Arc<dyn EisBackend>;
            (Box::new(host) as Box<dyn Host>, Some(loop_), capture, eis)
        }
        None => (
            Box::new(NoShell) as Box<dyn Host>,
            None,
            Arc::new(Missing) as Arc<dyn CaptureBackend>,
            Arc::new(Missing) as Arc<dyn EisBackend>,
        ),
    };
    let portal = Portal::new(
        connection.clone(),
        host,
        Backends {
            capture,
            eis,
            ..Backends::default()
        },
        options.consent_lifetime,
        options.secret_served,
    );
    portal.export().await?;
    // The name is requested after the objects exist, and without queueing: a second
    // portal for one session is a configuration mistake, and the one that lost says so
    // instead of waiting behind the winner.
    match connection
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        // Another portal serves this name. That is a second instance for the session: it
        // leaves quietly, and it reports ready so the guardian's start sequence goes on.
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            log::info!(name = BUS_NAME, "another portal serves this name; this instance exits");
            ready();
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    }
    log::info!(name = BUS_NAME, "serving the Gnoblin portal backend");

    if let Some(loop_) = loop_ {
        let serving = portal.clone();
        std::thread::Builder::new()
            .name("gnoblin-portal-ipc".to_owned())
            .spawn(move || loop_.run(serving))
            .map_err(|error| format!("the IPC loop could not be started: {error}"))?;
    }
    ready();

    // The owner watch and the expiry loop run here. The bus closing is the end of the
    // process: a bus-activated backend has no other reason to outlive its session.
    future::or(portal.run(), connection.closed()).await;
    Ok(())
}
