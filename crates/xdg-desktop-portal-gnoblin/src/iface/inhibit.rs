//! `org.freedesktop.impl.portal.Inhibit`.
//!
//! The one interface here whose *whole* lifetime is real without a compositor call:
//! `Inhibit` exports a request handle and the inhibition lasts until the frontend closes
//! it, and `CreateMonitor` creates a session the frontend closes. Only the inhibition
//! itself is the compositor's, and [`crate::backends::InhibitBackend`] is where it goes.
//!
//! # Why the default refuses instead of forwarding
//!
//! The plan's instruction for this interface is exact: forward it "to the compositor's
//! inhibit state as IPC calls if the schema has them, else document and refuse". The object
//! schema has no inhibit member — the `settings` object added for D15 is the only new
//! object this change adds — so there is nothing to call, and the default backend refuses
//! with a sentence naming `inhibit`. A session-management dialogue that waits for an
//! inhibition nobody holds would keep the session open, which is the failure that matters,
//! so a refusal is the safe answer.

use std::collections::BTreeMap;

use zbus::OwnedVariant;
use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::capture::{Options, sandboxed};
use super::{Handle, Portal, export_session, refused, remove, sender, string_of, unsupported};
use crate::consent::{Interface, NewSession, State};

use crate::log;

/// The version of `Inhibit` this build serves.
const VERSION: u32 = 1;

/// One served `Inhibit` object.
#[derive(Debug)]
pub struct Inhibit {
    portal: Portal,
}

impl Inhibit {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> Inhibit {
        Inhibit {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Inhibit")]
impl Inhibit {
    /// Hold the session's state changes until the handle is closed.
    ///
    /// The method returns nothing by design: the interface's answer is the handle, and the
    /// inhibition ends when `org.freedesktop.impl.portal.Request.Close` reaches it.
    async fn inhibit(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
        flags: u32,
        options: Options,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = handle.as_str().to_owned();
        let _ = caller;
        let reason = options.get("reason").and_then(string_of).unwrap_or_default();
        let held = {
            let mut shared = self.portal.lock();
            shared.inhibit.inhibit(&app_id, flags, &reason)
        };
        match held {
            Ok(handle) => {
                super::export_request(&self.portal, &path).await?;
                self.portal.handle(&path, Handle::Inhibition(handle));
                log::info!(app = %app_id, flags, "an inhibition was recorded");
                Ok(())
            }
            Err(missing) => {
                let _ = remove(&self.portal.connection, &path).await;
                Err(unsupported(missing))
            }
        }
    }

    /// Create a monitoring session.
    ///
    /// The session is real: it exists until the frontend closes it, and
    /// `QueryEndResponse` is accepted for it. `StateChanged` has no source yet — the
    /// compositor reports a session state change through no member — so the signal is
    /// declared for a client that subscribes and never raised, which is the honest form of
    /// "the interface has this signal and this build has nothing to say on it".
    async fn create_monitor(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _window: String,
    ) -> fdo::Result<u32> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller,
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&Options::new()),
                    interface: Interface::Inhibit,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        export_session(&self.portal, &path).await?;
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        log::info!(app = %app_id, session = %path, "an inhibit monitor was created");
        Ok(0)
    }

    /// Acknowledge a query-end signal.
    async fn query_end_response(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
    ) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        let shared = self.portal.lock();
        let session = shared
            .guard
            .session(&caller, &path)
            .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
        if session.state == State::Created && session.interface != Interface::Inhibit {
            return Err(refused(crate::consent::Refusal::WrongState {
                path,
                state: session.state,
            }));
        }
        Ok(())
    }

    /// The session state changed.
    #[zbus(signal)]
    async fn state_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        session_handle: ObjectPath<'_>,
        state: BTreeMap<String, OwnedVariant>,
    ) -> zbus::Result<()>;

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        VERSION
    }
}
