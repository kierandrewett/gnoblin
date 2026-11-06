//! The portal process: one bus name, one privileged IPC connection, one service loop.
//!
//! Design/rust-compositor.md's "Portals" card says the services role starts this backend
//! with a privileged connection. The bus can also start it on demand, through the
//! `org.freedesktop.impl.portal.desktop.gnoblin` activation file, and then no privileged
//! connection is passed. This file is the
//! composition root that does the three things such a process must: own
//! `org.freedesktop.impl.portal.desktop.gnoblin`, serve the interfaces, and read the
//! privileged connection for as long as it lives.
//!
//! # The order, and why
//!
//! 1. **The IPC connection first, if there is one.** Every interface can need it, so the
//!    shell is looked for before the name is taken. Without it the portal still starts:
//!    it serves what needs no prompt and refuses the rest (see the last section).
//! 2. **The objects, then the name.** A caller that resolves the name must find an object
//!    behind it; a name without an object is a method call that never answers.
//! 3. **The loop on its own thread, the bus loop on this one.** The IPC client is a
//!    blocking reader, and the bus connection has its own executor; one thread each keeps
//!    either from stalling the other.
//!
//! # What the connection is, in this build
//!
//! The services role passes the connection as an inherited descriptor when it starts this
//! process; otherwise the socket at `$GNOBLIN_IPC_SOCKET` (or `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`)
//! is used, which is what a developer session has.
//!
//! # A session with no shell
//!
//! When neither is reachable, the process logs that once and runs with [`NoShell`]: every
//! consent offer is refused naming `portal.ui`, so no capture, input or shortcut can start,
//! and `Settings` still answers from its (empty) table. The bus is then the only thing this
//! process waits on: it exits when the session bus goes away.

use std::process::ExitCode;

use futures_lite::future;
use gnoblin_client::{Client, UnixTransport, inherited_stream, socket_path};
use gnoblin_types::Version;
use xdg_desktop_portal_gnoblin::host::NoShell;
use xdg_desktop_portal_gnoblin::{BUS_NAME, Backends, Host, IpcHost, IpcLoop, Portal};

/// How long one consent prompt stays open, in milliseconds.
///
/// Two minutes is the frontend's own default for a response, so a portal that waits longer
/// than the frontend is a dialogue whose answer nobody can use.
const CONSENT_LIFETIME: u64 = 120_000;

fn main() -> ExitCode {
    let options = gnoblin_log_init::LogOptions::from_environment();
    let _log = gnoblin_log_init::init(gnoblin_log::Role::Portal, &options);
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            gnoblin_log::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Connect if a shell is reachable, export, own the name and serve until the bus goes.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (host, loop_) = match ipc() {
        Ok((host, loop_)) => (Box::new(host) as Box<dyn Host>, Some(loop_)),
        Err(error) => {
            gnoblin_log::warn!("no compositor IPC ({error}); capture, input and shortcuts will be refused");
            (Box::new(NoShell) as Box<dyn Host>, None)
        }
    };

    zbus::block_on(async move {
        let connection = zbus::Connection::session().await?;
        let portal = Portal::new(connection.clone(), host, Backends::default(), CONSENT_LIFETIME);
        portal.export().await?;
        // The name is requested after the objects exist, and without queueing: a second
        // portal for one session is a configuration mistake, and the one that lost says so
        // instead of waiting behind the winner.
        connection
            .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DO_NOT_QUEUE)
            .await?;
        gnoblin_log::info!(name = BUS_NAME, "serving the Gnoblin portal backend");

        if let Some(loop_) = loop_ {
            let serving = portal.clone();
            std::thread::Builder::new()
                .name("gnoblin-portal-ipc".to_owned())
                .spawn(move || loop_.run(serving))
                .map_err(|error| format!("the IPC loop could not be started: {error}"))?;
        }

        // The owner watch and the expiry loop run here. The bus closing is the end of the
        // process: a bus-activated backend has no other reason to outlive its session.
        future::or(portal.run(), connection.closed()).await;
        Ok(())
    })
}

/// The privileged connection and its IPC loop, or why there is none.
fn ipc() -> Result<(IpcHost, IpcLoop<UnixTransport>), Box<dyn std::error::Error>> {
    let transport = transport()?;
    let (major, minor, patch) = gnoblin_proto_version();
    let client = Client::connect(transport, Version::new(major, minor, patch))?;
    Ok(IpcHost::pair(client))
}

/// The privileged connection, as an inherited descriptor or the session's socket.
fn transport() -> Result<UnixTransport, Box<dyn std::error::Error>> {
    let environment = |name: &str| std::env::var(name).ok();
    if let Some(stream) = inherited_stream("GNOBLIN_IPC_FD", environment)? {
        return Ok(UnixTransport::from_stream(stream)?);
    }
    let path = socket_path(environment)?;
    Ok(UnixTransport::connect(std::path::Path::new(&path))?)
}

/// The IPC version this build advertises, from the schema both sides are built from.
fn gnoblin_proto_version() -> (u32, u32, u32) {
    gnoblin_proto::IPC_VERSION
}
