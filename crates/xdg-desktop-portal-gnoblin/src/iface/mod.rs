//! The D-Bus half: one object per interface at the backend's path, and the two objects
//! the frontend differentiates.
//!
//! Everything here is transport. A method reads its arguments, asks [`Shared`] for the
//! one decision it needs, and turns the result into the `Response`/`a{sv}` pair the
//! `org.freedesktop.impl.portal` interfaces return. No interface keeps state of its own:
//! the sessions are in [`crate::consent::Guard`], the settings values in
//! [`crate::settings::Table`], and the compositor calls behind the traits in
//! [`crate::backends`].
//!
//! # Why the methods block
//!
//! A consent prompt is answered by a shell, which is another process, so the D-Bus call
//! that needs the answer waits for it. The wait is a one-shot channel, and `Close` on the
//! request handle resolves it: the frontend cancels a request by calling
//! `org.freedesktop.impl.portal.Request.Close`, and the handler must be released rather
//! than held until its own deadline. That is why [`RequestObject`] exists at all, and why
//! the handle paths in [`Shared::handles`] are recorded before the wait begins.
//!
//! # Object lifetime
//!
//! A session handle and a request handle are paths the frontend chose, and both are an
//! object on this connection for exactly as long as they mean something: the session
//! object ends when the session does, the request object when the method it belongs to
//! returns. A stale handle is therefore an error the frontend can act on instead of a
//! call that reaches a session nobody owns.

pub mod capture;
pub mod inhibit;
pub mod remote;
pub mod settings;
pub mod shortcuts;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use futures_lite::StreamExt as _;

use gnoblin_types::{RequestId, Tick};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, fdo};

use crate::backends::{
    Backends, CaptureBackend, ClipboardBackend, EisBackend, InhibitBackend, InhibitHandle, ShortcutsBackend,
};
use crate::consent::{ConsentOutcome, Guard, Host, Refusal};
use crate::settings::Table;

/// The name the backend owns, as the `.portal` file and the frontend's routing name it.
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.gnoblin";

/// The path every interface is served at, which is the one the frontend calls.
pub const PATH: &str = "/org/freedesktop/portal/desktop";

/// How often the service loop fails the prompts that are past their deadline.
pub const TICK: std::time::Duration = std::time::Duration::from_millis(200);

/// One request handle's live meaning, so `Request.Close` can end what the handler waits
/// for.
pub enum Handle {
    /// A method is waiting for this consent prompt's answer.
    Consent(RequestId),
    /// An inhibition the frontend ends with `Close`.
    Inhibition(InhibitHandle),
}

/// Everything the served objects share.
pub struct Shared {
    /// The sessions and the grants behind them.
    pub guard: Guard,
    /// The values `Settings` publishes.
    pub settings: Table,
    /// The capture call.
    pub capture: Box<dyn CaptureBackend>,
    /// The input call.
    pub eis: Box<dyn EisBackend>,
    /// The inhibit call.
    pub inhibit: Box<dyn InhibitBackend>,
    /// The shortcut call.
    pub shortcuts: Box<dyn ShortcutsBackend>,
    /// The clipboard call.
    pub clipboard: Box<dyn ClipboardBackend>,
    /// Live request handles, keyed by the path the frontend chose.
    pub handles: BTreeMap<String, Handle>,
    /// The instant deadlines are measured from.
    epoch: Instant,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Shared")
            .field("guard", &self.guard)
            .field("settings", &self.settings)
            .field("handles", &self.handles.len())
            .finish_non_exhaustive()
    }
}

impl Shared {
    /// The compositor's tick now, in milliseconds since this process started.
    ///
    /// A consent deadline is the one place time is the input, so it needs a clock. The
    /// compositor supplies ticks to every rule it owns; this process is not the
    /// compositor, and its deadline is its own, so the clock is too.
    #[must_use]
    pub fn now(&self) -> Tick {
        Tick::new(u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// One served backend: the shared state and the connection it answers on.
#[derive(Clone, Debug)]
pub struct Portal {
    shared: Arc<Mutex<Shared>>,
    connection: Connection,
}

impl Portal {
    /// A portal with `host` offering its consent prompts and `backends` performing the
    /// compositor calls.
    ///
    /// `lifetime` is how long one prompt stays open, in the same milliseconds
    /// [`Shared::now`] counts.
    #[must_use]
    pub fn new(connection: Connection, host: Box<dyn Host>, backends: Backends, lifetime: u64) -> Portal {
        Portal {
            shared: Arc::new(Mutex::new(Shared {
                guard: Guard::new(host, lifetime),
                settings: Table::new(),
                capture: backends.capture,
                eis: backends.eis,
                inhibit: backends.inhibit,
                shortcuts: backends.shortcuts,
                clipboard: backends.clipboard,
                handles: BTreeMap::new(),
                epoch: Instant::now(),
            })),
            connection,
        }
    }

    /// The shared state, for the loop that feeds the graph's observations and for tests.
    #[must_use]
    pub fn shared(&self) -> &Arc<Mutex<Shared>> {
        &self.shared
    }

    /// The lock every handler takes, recovering from a handler that panicked.
    ///
    /// A poisoned lock is a panic inside a method that already replied or dropped its
    /// caller; refusing every later call as well would take the whole backend down for
    /// one bad request.
    pub fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Export every interface and own the bus name.
    ///
    /// The objects are exported before the name is requested, because a caller that
    /// resolves the name must find an object behind it: a name without an object is a
    /// method call that never gets a reply.
    pub async fn export(&self) -> Result<(), fdo::Error> {
        let object_server = self.connection.object_server();
        object_server
            .at(
                PATH,
                settings::Settings::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                capture::Screenshot::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                capture::ScreenCast::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                remote::RemoteDesktop::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                remote::InputCapture::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                shortcuts::GlobalShortcuts::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                shortcuts::Clipboard::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        object_server
            .at(
                PATH,
                inhibit::Inhibit::new(self.shared.clone(), self.connection.clone()),
            )
            .await
            .map_err(bus)?;
        Ok(())
    }

    /// Serve until the connection ends: expire prompts, and end what a gone caller owned.
    ///
    /// The name-owner watch is what makes "a grant ends with its requester" true across
    /// processes: the frontend connection disappearing is the only fact this backend gets
    /// about a caller that crashed, and a session that outlived its creator is the case
    /// the plan names.
    pub async fn run(self) {
        let mut owners = match bus::Owners::watch(&self.connection).await {
            Ok(owners) => owners,
            Err(error) => {
                tracing::warn!(%error, "the name-owner watch cannot start; grants will outlive a crashed caller");
                return self.expire_only().await;
            }
        };
        let mut ticker = async_io::Timer::interval(TICK);
        loop {
            let due = {
                let mut shared = self.lock();
                let now = shared.now();
                shared.guard.tick(now)
            };
            for request in due {
                tracing::debug!(request = request.get(), "a consent prompt passed its deadline");
            }
            // Wake on whichever comes first: a caller left the bus, or the next tick. The
            // tick expires prompts nobody answers, so it runs even on a quiet bus.
            let departed = futures_lite::future::or(async { Some(owners.next().await) }, async {
                ticker.next().await;
                None
            })
            .await;
            if let Some(name) = departed {
                let mut shared = self.lock();
                let now = shared.now();
                let sessions = shared.guard.requester_gone(now, &name);
                for session in sessions {
                    let streams = session.streams.clone();
                    for stream in streams {
                        shared.capture.close_stream(stream);
                    }
                }
                tracing::info!(%name, "a caller left; its grants and sessions ended");
            }
        }
    }

    /// Serve only the deadline, for a session where the bus watch failed.
    async fn expire_only(self) {
        let mut ticker = async_io::Timer::interval(TICK);
        loop {
            {
                let mut shared = self.lock();
                let now = shared.now();
                shared.guard.tick(now);
            }
            ticker.next().await;
        }
    }

    /// Close every stream of a session, then forget it.
    ///
    /// The two steps are one because a stream that outlives its session is the plan's
    /// first capture failure mode: the session is the consent, and the stream is what the
    /// consent allows.
    pub fn close_session(&self, bus_name: &str, path: &str) -> Result<(), Refusal> {
        let mut shared = self.lock();
        let session = shared.guard.close(bus_name, path)?;
        for stream in session.streams {
            shared.capture.close_stream(stream);
        }
        Ok(())
    }

    /// Record a request handle and return the slot its meaning goes in.
    pub fn handle(&self, path: &str, handle: Handle) {
        self.lock().handles.insert(path.to_owned(), handle);
    }

    /// Forget a request handle.
    pub fn release_handle(&self, path: &str) {
        self.lock().handles.remove(path);
    }

    /// Apply the settings object's observation and emit its changes.
    pub async fn observe(&self, observation: &gnoblin_model::Observation) {
        let changes = self.lock().settings.apply(observation);
        let emitter = match SignalEmitter::new(&self.connection, PATH) {
            Ok(emitter) => emitter,
            Err(error) => {
                tracing::warn!(%error, "the settings signal has no emitter");
                return;
            }
        };
        for change in changes {
            if let Err(error) = settings::Settings::emit(&emitter, change).await {
                tracing::warn!(%error, "a settings change could not be announced");
            }
        }
    }
}

/// Wait for a consent answer, mapping the channel's own failure.
pub async fn wait(pending: crate::consent::Pending) -> ConsentOutcome {
    match pending {
        crate::consent::Pending::AlreadyGranted => ConsentOutcome::Granted,
        crate::consent::Pending::Asked { answered, .. } => answered.await.unwrap_or(ConsentOutcome::Left),
    }
}

/// Export a session object at the handle the frontend chose.
pub async fn export_session(portal: &Portal, path: &str) -> Result<(), fdo::Error> {
    let object = SessionObject {
        shared: portal.shared.clone(),
        connection: portal.connection.clone(),
        path: path.to_owned(),
    };
    let added = portal.connection.object_server().at(path, object).await.map_err(bus)?;
    exported(added.is_none(), path)
}

/// Export a request object at the handle the frontend chose, for a method that has
/// something to cancel.
pub async fn export_request(portal: &Portal, path: &str) -> Result<(), fdo::Error> {
    let object = RequestObject {
        shared: portal.shared.clone(),
        connection: portal.connection.clone(),
        path: path.to_owned(),
    };
    let added = portal.connection.object_server().at(path, object).await.map_err(bus)?;
    exported(added.is_none(), path)
}

/// Whether `at` added the object: a path already exported is the frontend's own collision.
fn exported(added: bool, path: &str) -> Result<(), fdo::Error> {
    if added {
        Ok(())
    } else {
        Err(fdo::Error::FileExists(format!("`{path}` is already exported")))
    }
}

/// Remove an object this portal exported, ignoring one that was already gone.
pub async fn remove(connection: &Connection, path: &str) -> Result<(), fdo::Error> {
    let renamed = zbus::ObjectPath::try_from(path)
        .map_err(|error| fdo::Error::InvalidArgs(format!("`{path}` is not an object path: {error}")))?;
    let server = connection.object_server();
    // Both types may sit at a path while a method returns: a request handle and the
    // session it created are the frontend's choice and are usually different paths, but a
    // frontend may reuse one for both, and a removal that fails on the other type is not
    // an error.
    let _ = server.remove::<RequestObject, _>(&renamed).await;
    let _ = server.remove::<SessionObject, _>(&renamed).await;
    Ok(())
}

/// The lock a handler takes, with the poison check turned into the one error a caller can
/// act on.
pub fn locked(shared: &Arc<Mutex<Shared>>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The caller's unique bus name.
pub fn sender(header: &zbus::message::Header<'_>) -> Result<String, fdo::Error> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| fdo::Error::Failed("the call carried no sender".to_owned()))
}

/// One `Response`/`a{sv}` pair.
pub type Reply = (u32, BTreeMap<String, zbus::OwnedVariant>);

/// A refusal, as the error the frontend forwards.
///
/// D14 is a `Failed` whose sentence names `portal.ui`: the frontend shows the app its own
/// error, and the sentence is what a person reads in a log when a share does not appear.
pub fn refused(refusal: Refusal) -> fdo::Error {
    match refusal {
        Refusal::UnknownSession { .. } | Refusal::NotOwner { .. } | Refusal::WrongState { .. } => {
            fdo::Error::InvalidArgs(refusal.to_string())
        }
        Refusal::NoProvider { .. }
        | Refusal::Pending { .. }
        | Refusal::Refused
        | Refusal::Locked
        | Refusal::NotServed { .. }
        | Refusal::UnknownRequest { .. } => fdo::Error::Failed(refusal.to_string()),
    }
}

/// A compositor call this build cannot make, as an error naming what is missing.
pub fn unsupported(backend: crate::backends::Unavailable) -> fdo::Error {
    fdo::Error::NotSupported(backend.reason)
}

/// A bus failure, as the error the caller sees.
fn bus(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

/// One portal session, as the frontend closes it.
#[derive(Debug)]
pub struct SessionObject {
    shared: Arc<Mutex<Shared>>,
    connection: Connection,
    path: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl SessionObject {
    /// End the session.
    ///
    /// The frontend calls this when the app's own session object closes, so this is the
    /// one place a session ends other than its creator's connection going away.
    async fn close(&self, #[zbus(header)] header: zbus::message::Header<'_>) -> fdo::Result<()> {
        let caller = sender(&header)?;
        let portal = Portal {
            shared: self.shared.clone(),
            connection: self.connection.clone(),
        };
        portal.close_session(&caller, &self.path).map_err(refused)?;
        let emitter = SignalEmitter::new(&self.connection, self.path.as_str()).map_err(bus)?;
        SessionObject::closed(&emitter).await.map_err(bus)?;
        let _ = remove(&self.connection, &self.path).await;
        Ok(())
    }

    /// The session ended, for whatever reason.
    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// One request handle, as the frontend cancels it.
#[derive(Debug)]
pub struct RequestObject {
    shared: Arc<Mutex<Shared>>,
    connection: Connection,
    path: String,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestObject {
    /// Cancel whatever the handle stands for.
    ///
    /// An inhibition is released; a method waiting on consent is answered with a refusal,
    /// which is what makes `Close` reach the handler instead of leaving it until its
    /// deadline. A handle with nothing behind it is not an error: the method returned
    /// while the frontend was cancelling it, and the cancellation then has nothing to do.
    async fn close(&self) -> fdo::Result<()> {
        let handle = locked(&self.shared).handles.remove(&self.path);
        match handle {
            Some(Handle::Inhibition(id)) => {
                locked(&self.shared).inhibit.release(id);
            }
            Some(Handle::Consent(request)) => {
                let mut shared = locked(&self.shared);
                let now = shared.now();
                match shared.guard.answer(now, request, None) {
                    Ok(_) | Err(Refusal::UnknownRequest { .. }) => {}
                    Err(refusal) => return Err(refused(refusal)),
                }
            }
            None => {}
        }
        let _ = remove(&self.connection, &self.path).await;
        Ok(())
    }
}

/// The `org.freedesktop.DBus` name-owner watch.
mod bus {
    use zbus::{Connection, fdo};

    /// The unique names that have disappeared, from the bus's own `NameOwnerChanged`.
    pub struct Owners {
        stream: fdo::NameOwnerChangedStream,
    }

    impl Owners {
        /// Start watching the bus for names going away.
        pub async fn watch(connection: &Connection) -> Result<Owners, fdo::Error> {
            let proxy = fdo::DBusProxy::new(connection).await?;
            Ok(Owners {
                stream: proxy.receive_name_owner_changed().await?,
            })
        }

        /// The next unique name that left. Waits forever if the watch itself ends, so the
        /// caller's tick still runs.
        pub async fn next(&mut self) -> String {
            use futures_lite::StreamExt as _;
            loop {
                let Some(signal) = self.stream.next().await else {
                    tracing::warn!("the name-owner watch ended; grants will outlive a crashed caller");
                    return std::future::pending().await;
                };
                let Ok(args) = signal.args() else {
                    continue;
                };
                // Only a unique name that has gone ends anything: a well-known name
                // changing owner is two connections doing what they like, and the name a
                // grant is keyed by is the unique one.
                if args.name().starts_with(':') && args.new_owner().is_none() {
                    return args.name().to_string();
                }
            }
        }
    }
}

/// One method's wait for consent, with its handle recorded so `Close` can reach it.
///
/// The handle is recorded once the offer exists, because the request id is what `Close`
/// answers; a frontend that closes before the offer stands has nothing to cancel yet and
/// the method's own reply says so.
pub async fn await_consent(
    portal: &Portal,
    handle: &str,
    bus_name: &str,
    path: &str,
    capability: gnoblin_authority::PortalCapability,
) -> Result<ConsentOutcome, fdo::Error> {
    let pending = {
        let mut shared = portal.lock();
        let now = shared.now();
        shared.guard.ask(now, bus_name, path, capability).map_err(refused)?
    };
    match pending {
        crate::consent::Pending::AlreadyGranted => Ok(ConsentOutcome::Granted),
        crate::consent::Pending::Asked { request, answered } => {
            portal.handle(handle, Handle::Consent(request));
            let outcome = answered.await.unwrap_or(ConsentOutcome::Left);
            portal.release_handle(handle);
            Ok(outcome)
        }
    }
}

/// The string a variant carries, if it carries one.
pub(crate) fn string_of(value: &zbus::OwnedVariant) -> Option<String> {
    match &**value {
        zbus::Variant::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// One value of the object model, as the variant a result carries.
pub fn variant(value: &gnoblin_model::Value) -> Option<zbus::OwnedVariant> {
    use zbus::{OwnedVariant, Variant};
    let value = match value {
        gnoblin_model::Value::Bool(raw) => Variant::from(*raw),
        gnoblin_model::Value::Int(raw) => Variant::from(*raw),
        gnoblin_model::Value::Uint(raw) => Variant::from(*raw),
        gnoblin_model::Value::Double(raw) => Variant::from(*raw),
        gnoblin_model::Value::Text(raw) => Variant::from(raw.as_str()),
        // A bytes value, an object reference and a list have no meaning in a portal
        // options dictionary, so a backend that returns one is dropping a field rather
        // than inventing an encoding for it.
        gnoblin_model::Value::Bytes(_) | gnoblin_model::Value::Object(_) | gnoblin_model::Value::List(_) => {
            return None;
        }
    };
    OwnedVariant::try_from(value).ok()
}
