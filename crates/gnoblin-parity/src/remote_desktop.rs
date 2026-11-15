//! The EIS door of a Mutter session: `org.gnome.Mutter.RemoteDesktop`.
//!
//! Stock Mutter and Gnoblin on the patched Mutter serve input emulation the same way. A client creates a remote desktop
//! session on the session bus, asks the session for an EIS socket with `ConnectToEIS`, and starts the session. The
//! launcher starts a private bus at `<runtime-dir>/bus` ([`crate::launcher::BUS_SOCKET`]) so this module can reach it
//! from outside the process tree the launcher made. `ConnectToEIS` comes before `Start`: Mutter refuses it on a
//! session that already started, because the device set is fixed then.
//!
//! The session lives as long as the bus connection that created it, so the door keeps the connection until it is
//! dropped; closing it ends the session and Mutter removes the devices.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use futures_lite::future::block_on;
use zbus::connection::Builder;
use zbus::zvariant::Value;
use zbus::{Connection, OwnedFd, OwnedObjectPath, Proxy};

use crate::ei_peer::EisDoor;
use crate::target::TargetError;

const DESTINATION: &str = "org.gnome.Mutter.RemoteDesktop";
const MANAGER_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const MANAGER: &str = "org.gnome.Mutter.RemoteDesktop";
const SESSION: &str = "org.gnome.Mutter.RemoteDesktop.Session";

/// Opens EIS sockets from the remote desktop service of the compositor whose private bus is at `bus`.
pub(crate) struct RemoteDesktopDoor {
    target: &'static str,
    bus: PathBuf,
    /// The connection the session belongs to, kept so the session outlives the call that created it.
    connection: Option<Connection>,
}

impl RemoteDesktopDoor {
    /// A door that has not connected to the bus yet.
    pub(crate) fn new(target: &'static str, bus: PathBuf) -> Self {
        Self {
            target,
            bus,
            connection: None,
        }
    }

    fn live(&self, detail: impl Into<String>) -> TargetError {
        TargetError::Live {
            target: self.target.to_owned(),
            detail: detail.into(),
        }
    }
}

impl EisDoor for RemoteDesktopDoor {
    fn open(&mut self) -> Result<UnixStream, TargetError> {
        let address = format!("unix:path={}", self.bus.display());
        let described = |what: &str, error: zbus::Error| {
            self.live(format!("{what} on the bus at {} failed: {error}", self.bus.display()))
        };
        let connection = block_on(async { Builder::address(address.as_str()).build().await })
            .map_err(|error| described("connecting", error))?;
        let fd = block_on(async {
            let manager = Proxy::new(&connection, DESTINATION, MANAGER_PATH, MANAGER).await?;
            let path: OwnedObjectPath = manager.call("CreateSession", &()).await?;
            let session = Proxy::new(&connection, DESTINATION, path.as_str(), SESSION).await?;
            let options: HashMap<&str, Value<'_>> = HashMap::new();
            let fd: OwnedFd = session.call("ConnectToEIS", &options).await?;
            session.call::<_, _, ()>("Start", &()).await?;
            Ok::<_, zbus::Error>(fd)
        })
        .map_err(|error| described("the remote desktop session", error))?;
        self.connection = Some(connection);
        Ok(UnixStream::from(std::os::fd::OwnedFd::from(fd)))
    }
}
