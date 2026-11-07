//! The org.gnome.Shell.Introspect adapter, linked into the services role.
//!
//! `gnoblin-services` owns the privileged connection and names no compat adapter. This
//! module is the composition the `gnoblin` binary makes: it asks the adapter to connect to
//! the compositor's graph, and serves `org.gnome.Shell.Introspect` on the session bus with the
//! gate the configuration chose. The adapter owns the socket and configuration reads.
//!
//! The compositor socket, the session bus or a taken name makes the adapter unavailable.
//! That is logged once and skipped: a session that cannot list windows still has a compositor.

use std::thread::JoinHandle;

use crate::{IntrospectService, export, link_from_environment};
use futures_lite::future::block_on;
use gnoblin_compat_core::{Gate, Mode};

use gnoblin_service_core::{Host, LinkedAgent, RequestId, RequestOutcome, Tick};
use zbus::Connection;

use crate::log;

/// The introspection adapter as a linked agent of the services role.
#[derive(Default)]
pub struct Introspection {
    mode: Mode,
    served: Option<(IntrospectService, Connection, JoinHandle<()>)>,
}

impl Introspection {
    /// An adapter that has not started: it serves nothing until the role starts it.
    pub fn new(mode: Mode) -> Introspection {
        Introspection { mode, served: None }
    }
}

impl LinkedAgent for Introspection {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        let mut link = match link_from_environment() {
            Ok(link) => link,
            Err(error) => {
                log::info!(%error, "[services] no compositor graph, so no introspection adapter");
                return false;
            }
        };
        let connection = match block_on(Connection::session()) {
            Ok(connection) => connection,
            Err(error) => {
                log::info!(%error, "[services] no session bus, so no introspection adapter");
                return false;
            }
        };
        let service = match IntrospectService::with_gate(Gate::new(self.mode, Default::default(), Default::default())) {
            Ok(service) => service,
            Err(error) => {
                log::warn!(%error, "[services] the introspection adapter refused its support matrix");
                return false;
            }
        };
        if let Err(error) = block_on(export(&connection, &service)) {
            log::warn!(%error, "[services] the introspection name is unavailable");
            return false;
        }
        service.attach(&connection);
        if let Err(error) = block_on(service.publish(link.state().clone())) {
            log::warn!(%error, "[services] the introspection state could not be published");
            return false;
        }
        let serving = service.clone();
        let handle = std::thread::Builder::new()
            .name("gnoblin-introspect".to_owned())
            .spawn(move || {
                loop {
                    match link.advance() {
                        Ok(true) => {
                            if let Err(error) = block_on(serving.publish(link.state().clone())) {
                                log::warn!(%error, "[services] the introspection state could not be published");
                            }
                        }
                        Ok(false) => {
                            log::debug!("[services] the compositor closed the introspection link");
                            break;
                        }
                        Err(error) => {
                            log::warn!(%error, "[services] the introspection link failed");
                            break;
                        }
                    }
                }
            })
            .map_err(|error| log::warn!(%error, "[services] the introspection loop could not be started"));
        match handle {
            Ok(handle) => {
                self.served = Some((service, connection, handle));
                true
            }
            Err(()) => false,
        }
    }

    fn answer(&self, _request: RequestId, _outcome: &RequestOutcome) {}
}
