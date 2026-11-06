//! The `org.gnome.Mutter.DisplayConfig` adapter, linked into the services role.
//!
//! `gnoblin-services` owns the privileged connection and names no compat adapter. This module
//! is the composition the `gnoblin` binary makes: it opens its own compositor subscription,
//! serves `org.gnome.Mutter.DisplayConfig` on the session bus, and republishes the monitors
//! each time the compositor's object graph changes.
//!
//! The gate is built from the mode the role is given. Writes stay refused in the default
//! `portal` mode: `ApplyMonitorsConfig` reaches the applier only when the mode is `unmediated`,
//! and this module's applier refuses every write regardless. A session bus, a taken name or a
//! compositor that is not there is logged and skipped.

use futures_lite::future::block_on;
use gnoblin_compat_core::{CallerContext, Gate, LockState, Mode};
use gnoblin_compat_mutter_displayconfig::{DisplayConfigService, DisplayLink, OutputApplier, export};
use gnoblin_log::{debug_in, info_in, warn_in};
use gnoblin_services::link::{
    Client, Fields, Host, LinkedAgent, RequestId, RequestOutcome, Tick, UnixTransport, Version, socket_path,
};
use zbus::Connection;

/// The version this adapter's compositor subscription speaks.
const CLIENT_VERSION: Version = Version::new(0, 2, 0);

/// The applier the adapter draws through. It refuses every write.
struct WritesOff;

impl OutputApplier for WritesOff {
    fn apply(&self, output: &str, _fields: &Fields) -> Result<(), String> {
        Err(format!(
            "applying a configuration to {output} is refused: writes are off"
        ))
    }
}

/// The compositor subscription the adapter reads the monitors from.
fn connect_link() -> Result<DisplayLink<UnixTransport>, String> {
    let path = socket_path(|name| std::env::var(name).ok())?;
    let transport = UnixTransport::connect(&path).map_err(|error| error.to_string())?;
    let client = Client::connect(transport, CLIENT_VERSION).map_err(|error| error.to_string())?;
    DisplayLink::connect(client).map_err(|error| error.to_string())
}

/// The display configuration adapter as a linked agent of the services role.
pub struct MutterDisplayConfig {
    mode: Mode,
}

impl MutterDisplayConfig {
    /// An adapter that has not started, gated by `mode`: it serves nothing until the role
    /// starts it.
    pub fn new(mode: Mode) -> MutterDisplayConfig {
        MutterDisplayConfig { mode }
    }
}

impl LinkedAgent for MutterDisplayConfig {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        let connection = match block_on(Connection::session()) {
            Ok(connection) => connection,
            Err(error) => {
                info_in!("gnoblin", %error, "[services] no session bus, so no display configuration adapter");
                return false;
            }
        };
        let mut link = match connect_link() {
            Ok(link) => link,
            Err(error) => {
                warn_in!("gnoblin", %error, "[services] the display configuration adapter has no compositor link");
                return false;
            }
        };
        let gate = Gate::new(self.mode, CallerContext::UNSANDBOXED, LockState::Unlocked);
        let service = match DisplayConfigService::with_gate(gate, Box::new(WritesOff)) {
            Ok(service) => service,
            Err(error) => {
                warn_in!("gnoblin", %error, "[services] the display configuration adapter refused its support matrix");
                return false;
            }
        };
        if let Err(error) = block_on(export(&connection, &service)) {
            warn_in!("gnoblin", %error, "[services] the display configuration name is unavailable");
            return false;
        }
        if let Err(error) = block_on(service.publish(link.state().clone(), link.lock())) {
            warn_in!("gnoblin", %error, "[services] the display configuration could not publish its monitors");
        }
        let handle = std::thread::Builder::new()
            .name("gnoblin-displayconfig".to_owned())
            .spawn(move || {
                // The connection lives as long as the loop: dropping it releases the name.
                let _connection = connection;
                loop {
                    match link.advance() {
                        Ok(true) => {
                            if let Err(error) = block_on(service.publish(link.state().clone(), link.lock())) {
                                debug_in!("gnoblin", %error, "[services] the display configuration did not publish");
                            }
                        }
                        Ok(false) => {
                            info_in!(
                                "gnoblin",
                                "[services] the compositor link ended; display configuration stops"
                            );
                            break;
                        }
                        Err(error) => {
                            warn_in!("gnoblin", %error, "[services] the display configuration link failed");
                            break;
                        }
                    }
                }
            });
        match handle {
            Ok(_) => true,
            Err(error) => {
                warn_in!("gnoblin", %error, "[services] the display configuration loop could not be started");
                false
            }
        }
    }

    fn answer(&self, _request: RequestId, _outcome: &RequestOutcome) {
        // This adapter offers no requests, so no outcome is its own.
    }
}
