//! The `org.gnome.Mutter.IdleMonitor` adapter, linked into the services role.
//!
//! Screensavers and `gsd-power` need this name, so the adapter is on by default. This module
//! is the composition the `gnoblin` binary makes: it opens its own compositor subscription,
//! serves `org.gnome.Mutter.IdleMonitor` on the session bus, republishes the idle counter when
//! the compositor's object graph changes, and fires the watches callers registered.
//!
//! The gate is built from the mode the role is given. A session bus, a taken name or a
//! compositor that is not there is logged and skipped.

use futures_lite::future::block_on;
use gnoblin_compat_core::{CallerContext, Gate, LockState, Mode};
use gnoblin_compat_mutter_idlemonitor::{IdleLink, IdleMonitorService, export, watch_callers};
use gnoblin_log::{debug, info, warn};
use gnoblin_services::link::{
    Client, Host, LinkedAgent, RequestId, RequestOutcome, Tick, UnixTransport, Version, socket_path,
};
use zbus::Connection;

/// The version this adapter's compositor subscription speaks.
const CLIENT_VERSION: Version = Version::new(0, 2, 0);

/// The compositor subscription the adapter reads the idle counter from.
fn connect_link() -> Result<IdleLink<UnixTransport>, String> {
    let path = socket_path(|name| std::env::var(name).ok())?;
    let transport = UnixTransport::connect(&path).map_err(|error| error.to_string())?;
    let client = Client::connect(transport, CLIENT_VERSION).map_err(|error| error.to_string())?;
    IdleLink::connect(client).map_err(|error| error.to_string())
}

/// The idle monitor adapter as a linked agent of the services role.
pub struct MutterIdleMonitor {
    mode: Mode,
}

impl MutterIdleMonitor {
    /// An adapter that has not started, gated by `mode`: it serves nothing until the role
    /// starts it.
    pub fn new(mode: Mode) -> MutterIdleMonitor {
        MutterIdleMonitor { mode }
    }
}

impl LinkedAgent for MutterIdleMonitor {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        let connection = match block_on(Connection::session()) {
            Ok(connection) => connection,
            Err(error) => {
                info!(%error, "[services] no session bus, so no idle monitor adapter");
                return false;
            }
        };
        let mut link = match connect_link() {
            Ok(link) => link,
            Err(error) => {
                warn!(%error, "[services] the idle monitor adapter has no compositor link");
                return false;
            }
        };
        let gate = Gate::new(self.mode, CallerContext::UNSANDBOXED, LockState::Unlocked);
        let service = match IdleMonitorService::with_gate(gate) {
            Ok(service) => service,
            Err(error) => {
                warn!(%error, "[services] the idle monitor adapter refused its support matrix");
                return false;
            }
        };
        if let Err(error) = block_on(export(&connection, &service)) {
            warn!(%error, "[services] the idle monitor name is unavailable");
            return false;
        }
        if let Err(error) = block_on(service.publish(link.state())) {
            warn!(%error, "[services] the idle monitor could not publish its counter");
        }
        block_on(watch_callers(connection.clone(), service.clone()));
        let handle = std::thread::Builder::new()
            .name("gnoblin-idlemonitor".to_owned())
            .spawn(move || {
                // The connection lives as long as the loop: dropping it releases the name.
                let _connection = connection;
                loop {
                    match link.advance() {
                        Ok(true) => {
                            if let Err(error) = block_on(service.publish(link.state())) {
                                debug!(%error, "[services] the idle monitor did not publish");
                            }
                        }
                        Ok(false) => {
                            info!("[services] the compositor link ended; idle monitor stops");
                            break;
                        }
                        Err(error) => {
                            warn!(%error, "[services] the idle monitor link failed");
                            break;
                        }
                    }
                }
            });
        match handle {
            Ok(_) => true,
            Err(error) => {
                warn!(%error, "[services] the idle monitor loop could not be started");
                false
            }
        }
    }

    fn answer(&self, _request: RequestId, _outcome: &RequestOutcome) {
        // This adapter offers no requests, so no outcome is its own.
    }
}
