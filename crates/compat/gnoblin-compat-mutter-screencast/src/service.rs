//! The served interfaces: `org.gnome.Mutter.ScreenCast` and its session and stream
//! objects.
//!
//! Every member admits its call through [`ScreenCastAdmission`] first, so a disabled
//! adapter answers nothing, a capture in `portal` mode answers
//! `capture-portal-only`, and a capture while the lock is up answers `locked-capture`.
//! The mapping after the gate is wire state plus one compositor call: a session opens
//! a capture through [`CaptureBridge`] when it starts and closes it when it ends.
//!
//! Three things are worth stating rather than implying:
//!
//! - **A session belongs to its caller.** Every session and stream member compares the
//!   sender with the unique name that created the session, so a second process that
//!   learns a path cannot stop a stream or read its parameters. The caller's bus name
//!   vanishing ends its sessions, which is the plan's "lifetimes that end with the
//!   caller".
//! - **The lock ends sessions rather than pausing them.** [`ScreenCastService::set_lock`]
//!   closes every open session, so releasing the lock does not resume a capture the user
//!   was never asked about again.
//! - **`Stream.Start` names the missing producer.** PipeWire cannot be built with the
//!   pinned toolchain (`gnoblin-capture` documents the exact failure), so the method
//!   answers `unsupported-pipewire` and no node id is invented. Mutter's proxy watches
//!   the `Parameters` property; Mutter 51.0 declares no `ParameterChanged` signal, and
//!   this adapter does not invent one either.
//!
//! # Failure modes first
//!
//! 1. **A session that started keeps capturing after its caller left.** The
//!    `NameOwnerChanged` loop calls [`ScreenCastService::forget_owner`], which closes the
//!    compositor session: `a_session_ends_with_its_caller`.
//! 2. **A session outlives the lock.** `set_lock` ends every session and stream:
//!    `locking_ends_every_session`.
//! 3. **A refused open leaves a started session with no capture.** The handle is stored
//!    only after the bridge answered: `a_refused_open_leaves_the_session_unstarted`.
//! 4. **A second caller uses another's session.** Every member checks the recorded
//!    owner: `a_foreign_caller_cannot_use_a_session`.
//! 5. **A second export replaces a live object.** `take_export` refuses it:
//!    `an_adapter_registers_once`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use gnoblin_compat_core::{Call as _, CallerWatcher, Gate, LockState, Mode, Refusal};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, ObjectPath, OwnedVariant, fdo};

use crate::bridge::CaptureBridge;
use crate::caller;
use crate::compat::{ScreenCastAdmission, ScreenCastCall, default_gate};
use crate::error::ScreenCastError;

/// The bus name Mutter owns, which this adapter takes when it serves.
pub const BUS_NAME: &str = "org.gnome.Mutter.ScreenCast";

/// The object path Mutter exports it on.
pub const PATH: &str = "/org/gnome/Mutter/ScreenCast";

/// The interface name.
pub const INTERFACE: &str = "org.gnome.Mutter.ScreenCast";

/// The session interface, one object per `CreateSession`.
pub const SESSION_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Session";

/// The stream interface, one object per `Record*`.
pub const STREAM_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";

/// The interface's version, as Mutter 51.0 reports it.
pub const VERSION: u32 = 4;

/// What one `Record*` selected.
#[derive(Clone, Debug, PartialEq, Eq)]
enum StreamTarget {
    /// A monitor, by connector.
    Monitor(String),
    /// A window, by the id the caller sent.
    Window(u64),
    /// A rectangle in stage coordinates.
    Area { x: i32, y: i32, width: i32, height: i32 },
}

/// One ScreenCast session, as wire state.
#[derive(Clone, Debug)]
struct SessionRecord {
    path: String,
    owner: String,
    remote_session: String,
    started: bool,
    capture: Option<u64>,
    streams: Vec<u32>,
}

/// One stream inside a session.
#[derive(Clone, Debug)]
struct StreamRecord {
    path: String,
    owner: String,
    target: StreamTarget,
}

/// Everything the adapter keeps between calls.
#[derive(Debug, Default)]
struct ScreenCastState {
    sessions: BTreeMap<u32, SessionRecord>,
    streams: BTreeMap<u32, StreamRecord>,
    next_session: u32,
    next_stream: u32,
}

/// The state the object server and the host share.
struct ServiceShared {
    admission: ScreenCastAdmission,
    gate: Mutex<Gate>,
    lock: Mutex<LockState>,
    bridge: Box<dyn CaptureBridge>,
    emitter: Mutex<Option<Connection>>,
    exported: Mutex<bool>,
    state: Mutex<ScreenCastState>,
}

impl std::fmt::Debug for ServiceShared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenCastShared")
            .field("admission", &self.admission)
            .field("gate", &self.gate)
            .field("lock", &self.lock)
            .field("bridge", &"<the host's capture seam>")
            .field("emitter", &self.emitter)
            .field("exported", &self.exported)
            .field("state", &self.state)
            .finish()
    }
}

/// The `ScreenCast` object: exportable, clonable and cheap.
#[derive(Clone, Debug)]
pub struct ScreenCastService {
    shared: Arc<ServiceShared>,
}

impl ScreenCastService {
    /// An adapter with the default gate and the supplied capture seam.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn new(bridge: Box<dyn CaptureBridge>) -> Result<ScreenCastService, gnoblin_compat_core::MatrixError> {
        ScreenCastService::with_gate(default_gate(), bridge)
    }

    /// An adapter with a gate the composition root chose.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn with_gate(
        gate: Gate,
        bridge: Box<dyn CaptureBridge>,
    ) -> Result<ScreenCastService, gnoblin_compat_core::MatrixError> {
        Ok(ScreenCastService {
            shared: Arc::new(ServiceShared {
                admission: ScreenCastAdmission::load()?,
                gate: Mutex::new(gate),
                lock: Mutex::new(LockState::Unlocked),
                bridge,
                emitter: Mutex::new(None),
                exported: Mutex::new(false),
                state: Mutex::new(ScreenCastState::default()),
            }),
        })
    }

    /// Set the mode the gate screens with, which is `gnoblin.configure.compat.capture`.
    pub fn set_mode(&self, mode: Mode) {
        let mut gate = self.gate_lock();
        *gate = Gate::new(mode, gate.caller(), gate.lock());
    }

    /// The mode the gate screens with.
    pub fn mode(&self) -> Mode {
        self.gate_lock().mode()
    }

    /// The lock state the gate screens with.
    pub fn lock(&self) -> LockState {
        *self.lock_lock()
    }

    /// Record the session lock and end every session when it engages.
    ///
    /// The composition root calls this from the IPC's own lock observation. Answering
    /// how many sessions ended makes the transition visible in a log line.
    pub async fn set_lock(&self, lock: LockState) -> usize {
        *self.lock_lock() = lock;
        if lock.is_engaged() { self.end_all().await } else { 0 }
    }

    /// How many sessions are live, for a host's log line or a test.
    pub fn session_count(&self) -> usize {
        self.state_lock().sessions.len()
    }

    /// How many streams are live, for a host's log line or a test.
    pub fn stream_count(&self) -> usize {
        self.state_lock().streams.len()
    }

    /// The object path of the session with `id`, when it is live.
    pub fn session_path(&self, id: u32) -> Option<String> {
        self.state_lock().sessions.get(&id).map(|record| record.path.clone())
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// # Errors
    ///
    /// [`ScreenCastError::Exported`] when this service was already exported.
    pub fn take_export(&self) -> Result<(), ScreenCastError> {
        let mut exported = self.exported_lock();
        if *exported {
            return Err(ScreenCastError::Exported);
        }
        *exported = true;
        Ok(())
    }

    /// Remember the connection, so ending a session can raise `Closed`.
    pub fn attach(&self, connection: &Connection) {
        *self.emitter_lock() = Some(connection.clone());
    }

    /// End every session owned by `owner`, answering how many ended.
    pub async fn forget_owner(&self, owner: &str) -> usize {
        let doomed: Vec<u32> = {
            let state = self.state_lock();
            state
                .sessions
                .iter()
                .filter(|(_, record)| record.owner == owner)
                .map(|(id, _)| *id)
                .collect()
        };
        let mut ended = 0;
        for id in doomed {
            if self.end_session(id).await {
                ended += 1;
            }
        }
        ended
    }

    /// End every session.
    async fn end_all(&self) -> usize {
        let doomed: Vec<u32> = self.state_lock().sessions.keys().copied().collect();
        let mut ended = 0;
        for id in doomed {
            if self.end_session(id).await {
                ended += 1;
            }
        }
        ended
    }

    /// End one session, its streams and the compositor's capture behind it.
    ///
    /// Answering `false` means the session was already gone, so a second `Stop` or a
    /// lock that raced the caller's own stop closes the bridge exactly once.
    async fn end_session(&self, id: u32) -> bool {
        let (record, connection) = {
            let mut state = self.state_lock();
            let Some(record) = state.sessions.remove(&id) else {
                return false;
            };
            for stream in &record.streams {
                state.streams.remove(stream);
            }
            (record, self.emitter_lock().clone())
        };
        if let Some(connection) = connection {
            match SignalEmitter::new(&connection, record.path.as_str()) {
                Ok(emitter) => {
                    if let Err(error) = ScreenCastSession::closed(&emitter).await {
                        gnoblin_log::debug!(session = id, %error, "the Closed signal could not be delivered");
                    }
                }
                Err(error) => gnoblin_log::debug!(session = id, %error, "the session path is not a signal path"),
            }
            for stream in &record.streams {
                if let Some(stream) = self.stream_path(*stream)
                    && let Err(error) = connection
                        .object_server()
                        .remove::<ScreenCastStream, _>(stream.as_str())
                        .await
                {
                    gnoblin_log::debug!(stream, %error, "a stream object could not be removed");
                }
            }
            if let Err(error) = connection
                .object_server()
                .remove::<ScreenCastSession, _>(record.path.as_str())
                .await
            {
                gnoblin_log::debug!(session = id, %error, "a session object could not be removed");
            }
        }
        if let Some(handle) = record.capture
            && let Err(error) = self.shared.bridge.close(crate::bridge::CaptureHandle::new(handle))
        {
            gnoblin_log::warn!(
                session = id,
                handle,
                reason = error.reason(),
                "the compositor did not confirm the capture session ended"
            );
        }
        gnoblin_log::debug!(session = id, "a screen cast session ended");
        true
    }

    fn stream_path(&self, id: u32) -> Option<String> {
        self.state_lock().streams.get(&id).map(|record| record.path.clone())
    }

    fn gate_lock(&self) -> MutexGuard<'_, Gate> {
        self.shared.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_lock(&self) -> MutexGuard<'_, LockState> {
        self.shared.lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn exported_lock(&self) -> MutexGuard<'_, bool> {
        self.shared
            .exported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn emitter_lock(&self) -> MutexGuard<'_, Option<Connection>> {
        self.shared
            .emitter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn state_lock(&self) -> MutexGuard<'_, ScreenCastState> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The gate for one call: the configured mode, the caller's sandbox and the lock.
    fn gate(&self, caller: gnoblin_compat_core::CallerContext) -> Gate {
        Gate::new(self.gate_lock().mode(), caller, *self.lock_lock())
    }

    /// Admit one call and answer the caller's bus name.
    ///
    /// A caller the bus cannot identify is refused a privileged call rather than
    /// assumed unsandboxed, which is the one place this adapter could otherwise be
    /// talked out of the sandbox rule.
    async fn admit(
        &self,
        connection: &Connection,
        header: &Header<'_>,
        call: ScreenCastCall,
    ) -> Result<String, fdo::Error> {
        let owner = header
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| fdo::Error::Failed("the call carried no sender".to_owned()))?;
        let caller = match caller::probe(connection, &owner).await {
            Ok(caller) => caller,
            Err(reason) if call.operation().is_privileged() => {
                return Err(fdo::Error::Failed(format!(
                    "the caller cannot be identified, so capture is refused: {reason}"
                )));
            }
            Err(reason) => {
                gnoblin_log::debug!(caller = %owner, %reason, "a read proceeded without identifying the caller");
                gnoblin_compat_core::CallerContext::UNSANDBOXED
            }
        };
        self.shared
            .admission
            .admit(call, self.gate(caller))
            .map_err(|refusal: Refusal| fdo::Error::Failed(refusal.to_string()))?;
        Ok(owner)
    }

    /// The matrix's own sentence for an unsupported member.
    fn unsupported(&self, call: ScreenCastCall) -> fdo::Error {
        let reason = self
            .shared
            .admission
            .matrix()
            .support(call.interface(), call.member())
            .and_then(gnoblin_compat_core::Support::detail)
            .unwrap_or("this adapter does not serve that member");
        fdo::Error::Failed(format!("{} is unsupported: {reason}", call.member()))
    }

    /// Refuse a call from anyone but the session's owner.
    fn ensure_owner(&self, id: u32, owner: &str) -> fdo::Result<()> {
        let state = self.state_lock();
        let record = state
            .sessions
            .get(&id)
            .ok_or_else(|| fdo::Error::Failed("the session has closed".to_owned()))?;
        if record.owner == owner {
            Ok(())
        } else {
            Err(fdo::Error::AccessDenied(
                "this session belongs to another caller".to_owned(),
            ))
        }
    }

    /// Refuse a stream call from anyone but the session's owner.
    fn ensure_stream_owner(&self, id: u32, owner: &str) -> fdo::Result<()> {
        let state = self.state_lock();
        let record = state
            .streams
            .get(&id)
            .ok_or_else(|| fdo::Error::Failed("the stream has closed".to_owned()))?;
        if record.owner == owner {
            Ok(())
        } else {
            Err(fdo::Error::AccessDenied(
                "this stream belongs to another caller".to_owned(),
            ))
        }
    }

    /// Allocate a stream inside a session and export its object.
    async fn add_stream(
        &self,
        connection: &Connection,
        session: u32,
        owner: &str,
        target: StreamTarget,
    ) -> fdo::Result<ObjectPath<'static>> {
        let (id, path) = {
            let mut state = self.state_lock();
            if !state.sessions.contains_key(&session) {
                return Err(fdo::Error::Failed("the session has closed".to_owned()));
            }
            state.next_stream = state.next_stream.saturating_add(1);
            let id = state.next_stream;
            let path = format!("{PATH}/Stream/u{id}");
            state.streams.insert(
                id,
                StreamRecord {
                    path: path.clone(),
                    owner: owner.to_owned(),
                    target,
                },
            );
            if let Some(record) = state.sessions.get_mut(&session) {
                record.streams.push(id);
            }
            (id, path)
        };
        let object = ScreenCastStream {
            shared: Arc::clone(&self.shared),
            id,
            session,
            path: Arc::from(path.as_str()),
        };
        if let Err(error) = connection.object_server().at(path.as_str(), object).await {
            let mut state = self.state_lock();
            state.streams.remove(&id);
            if let Some(record) = state.sessions.get_mut(&session) {
                record.streams.retain(|stream| *stream != id);
            }
            return Err(fdo::Error::Failed(format!(
                "the stream object could not be exported: {error}"
            )));
        }
        ObjectPath::try_from(path).map_err(|error| fdo::Error::Failed(error.to_string()))
    }
}

/// One `org.gnome.Mutter.ScreenCast.Session` object.
#[derive(Clone, Debug)]
struct ScreenCastSession {
    shared: Arc<ServiceShared>,
    id: u32,
}

/// One `org.gnome.Mutter.ScreenCast.Stream` object.
#[derive(Clone, Debug)]
struct ScreenCastStream {
    shared: Arc<ServiceShared>,
    id: u32,
    session: u32,
    path: Arc<str>,
}

impl ScreenCastSession {
    fn service(&self) -> ScreenCastService {
        ScreenCastService {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl ScreenCastStream {
    fn service(&self) -> ScreenCastService {
        ScreenCastService {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// The interface itself.
#[zbus::interface(name = "org.gnome.Mutter.ScreenCast")]
impl ScreenCastService {
    /// Create a session and answer its object path.
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        properties: HashMap<String, OwnedVariant>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let owner = self.admit(connection, &header, ScreenCastCall::CreateSession).await?;
        let remote_session = text(&properties, "remote-desktop-session-id").unwrap_or_default();
        if flag(&properties, "disable-animations") {
            // The session is announced without an animation switch: the object protocol
            // has no member for it, and a caller that asked gets no silent claim that it
            // was honoured.
            gnoblin_log::debug!("a screen cast session asked for animations to be disabled; the IPC has no switch");
        }
        let (id, path) = {
            let mut state = self.state_lock();
            state.next_session = state.next_session.saturating_add(1);
            let id = state.next_session;
            let path = format!("{PATH}/Session/u{id}");
            state.sessions.insert(
                id,
                SessionRecord {
                    path: path.clone(),
                    owner,
                    remote_session,
                    started: false,
                    capture: None,
                    streams: Vec::new(),
                },
            );
            (id, path)
        };
        let object = ScreenCastSession {
            shared: Arc::clone(&self.shared),
            id,
        };
        if let Err(error) = connection.object_server().at(path.as_str(), object).await {
            self.state_lock().sessions.remove(&id);
            return Err(fdo::Error::Failed(format!(
                "the session object could not be exported: {error}"
            )));
        }
        gnoblin_log::debug!(session = id, "a screen cast session was created");
        ObjectPath::try_from(path).map_err(|error| fdo::Error::Failed(error.to_string()))
    }

    /// The API version Mutter 51.0 reports.
    #[zbus(property)]
    fn version(&self) -> fdo::Result<i32> {
        self.shared
            .admission
            .admit(
                ScreenCastCall::Version,
                Gate::new(
                    self.mode(),
                    gnoblin_compat_core::CallerContext::UNSANDBOXED,
                    self.lock(),
                ),
            )
            .map_err(|refusal| fdo::Error::Failed(refusal.to_string()))?;
        Ok(VERSION as i32)
    }
}

#[zbus::interface(name = "org.gnome.Mutter.ScreenCast.Session")]
impl ScreenCastSession {
    /// Start the session: open the compositor's capture session.
    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::Start).await?;
        service.ensure_owner(self.id, &owner)?;
        let (remote_session, already) = {
            let state = service.state_lock();
            let record = state
                .sessions
                .get(&self.id)
                .ok_or_else(|| fdo::Error::Failed("the session has closed".to_owned()))?;
            (record.remote_session.clone(), record.started)
        };
        if already {
            return Err(fdo::Error::Failed("the session is already started".to_owned()));
        }
        let handle = service
            .shared
            .bridge
            .open(&owner, &remote_session)
            .map_err(|error| fdo::Error::Failed(format!("the compositor refused the capture session: {error}")))?;
        let mut state = service.state_lock();
        match state.sessions.get_mut(&self.id) {
            Some(record) if !record.started => {
                record.started = true;
                record.capture = Some(handle.get());
                Ok(())
            }
            // The session ended while the bridge was answering: the handle has no session
            // to belong to, so it is closed rather than leaked.
            _ => {
                drop(state);
                if let Err(error) = service.shared.bridge.close(handle) {
                    gnoblin_log::warn!(reason = error.reason(), "a capture session could not be closed");
                }
                Err(fdo::Error::Failed("the session closed while starting".to_owned()))
            }
        }
    }

    /// Stop the session and its streams.
    async fn stop(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::Stop).await?;
        service.ensure_owner(self.id, &owner)?;
        service.end_session(self.id).await;
        Ok(())
    }

    /// Record one monitor.
    async fn record_monitor(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        connector: String,
        _properties: HashMap<String, OwnedVariant>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let service = self.service();
        let owner = service
            .admit(connection, &header, ScreenCastCall::RecordMonitor)
            .await?;
        service.ensure_owner(self.id, &owner)?;
        service
            .add_stream(connection, self.id, &owner, StreamTarget::Monitor(connector))
            .await
    }

    /// Record one window.
    async fn record_window(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        properties: HashMap<String, OwnedVariant>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::RecordWindow).await?;
        service.ensure_owner(self.id, &owner)?;
        let window = properties
            .get("window-id")
            .and_then(|value| match &**value {
                zbus::Variant::U64(id) => Some(*id),
                _ => None,
            })
            .ok_or_else(|| fdo::Error::InvalidArgs("RecordWindow needs a `window-id` (t) property".to_owned()))?;
        service
            .add_stream(connection, self.id, &owner, StreamTarget::Window(window))
            .await
    }

    /// Record one area.
    #[allow(clippy::too_many_arguments)]
    async fn record_area(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        _properties: HashMap<String, OwnedVariant>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::RecordArea).await?;
        service.ensure_owner(self.id, &owner)?;
        if width <= 0 || height <= 0 {
            return Err(fdo::Error::InvalidArgs(
                "a recorded area needs a positive width and height".to_owned(),
            ));
        }
        service
            .add_stream(connection, self.id, &owner, StreamTarget::Area { x, y, width, height })
            .await
    }

    /// Record a virtual monitor, which this compositor does not publish.
    async fn record_virtual(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        _properties: HashMap<String, OwnedVariant>,
    ) -> fdo::Result<ObjectPath<'static>> {
        let service = self.service();
        service
            .admit(connection, &header, ScreenCastCall::RecordVirtual)
            .await?;
        Err(service.unsupported(ScreenCastCall::RecordVirtual))
    }

    /// The session closed: it was stopped, its caller left, or the lock engaged.
    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[zbus::interface(name = "org.gnome.Mutter.ScreenCast.Stream")]
impl ScreenCastStream {
    /// Start the stream, which needs a PipeWire producer this build does not have.
    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::StreamStart).await?;
        service.ensure_stream_owner(self.id, &owner)?;
        Err(service.unsupported(ScreenCastCall::StreamStart))
    }

    /// Stop the stream and remove its object.
    async fn stop(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<()> {
        let service = self.service();
        let owner = service.admit(connection, &header, ScreenCastCall::StreamStop).await?;
        service.ensure_stream_owner(self.id, &owner)?;
        {
            let mut state = service.state_lock();
            state.streams.remove(&self.id);
            if let Some(record) = state.sessions.get_mut(&self.session) {
                record.streams.retain(|stream| *stream != self.id);
            }
        }
        if let Err(error) = connection
            .object_server()
            .remove::<ScreenCastStream, _>(&*self.path)
            .await
        {
            gnoblin_log::debug!(stream = self.id, %error, "a stopped stream object could not be removed");
        }
        Ok(())
    }

    /// What the adapter knows about this stream's source.
    #[zbus(property)]
    async fn parameters(
        &self,
        #[zbus(header)] header: Option<Header<'_>>,
        #[zbus(connection)] connection: &Connection,
    ) -> fdo::Result<HashMap<String, OwnedVariant>> {
        let service = self.service();
        let header = header.ok_or_else(|| fdo::Error::Failed("the call carried no header".to_owned()))?;
        let owner = service
            .admit(connection, &header, ScreenCastCall::StreamParameters)
            .await?;
        service.ensure_stream_owner(self.id, &owner)?;
        let state = service.state_lock();
        let record = state
            .streams
            .get(&self.id)
            .ok_or_else(|| fdo::Error::Failed("the stream has closed".to_owned()))?;
        let mut parameters = HashMap::new();
        match &record.target {
            StreamTarget::Monitor(connector) => {
                parameters.insert(
                    "output-name".to_owned(),
                    OwnedVariant::from(zbus::Str::from(connector.as_str())),
                );
            }
            StreamTarget::Area { x, y, width, height } => {
                parameters.insert("position".to_owned(), pair(*x, *y));
                parameters.insert("size".to_owned(), pair(*width, *height));
            }
            StreamTarget::Window(_) => {}
        }
        Ok(parameters)
    }

    /// A PipeWire node was created for this stream, which this build cannot do.
    #[allow(dead_code)]
    #[zbus(signal)]
    async fn pipe_wire_stream_added(emitter: &SignalEmitter<'_>, node_id: u32) -> zbus::Result<()>;
}

/// A `(ii)` value inside a D-Bus property dict.
///
/// `Parameters` is a `a{sv}`, so its position and size are D-Bus structures rather than
/// tuples the value enum has a `From` for.
fn pair(first: i32, second: i32) -> OwnedVariant {
    OwnedVariant::try_from(zbus::Variant::Structure(zbus::Structure::from((first, second))))
        .expect("a pair of i32 has an owned form")
}

/// One string property, or `None` when it is absent or not a string.
fn text(properties: &HashMap<String, OwnedVariant>, name: &str) -> Option<String> {
    match properties.get(name).map(|value| &**value) {
        Some(zbus::Variant::Str(value)) => Some(value.as_str().to_owned()),
        _ => None,
    }
}

/// One boolean property, or `false` when it is absent or not a boolean.
fn flag(properties: &HashMap<String, OwnedVariant>, name: &str) -> bool {
    matches!(
        properties.get(name).map(|value| &**value),
        Some(zbus::Variant::Bool(true))
    )
}

/// End the sessions of every caller that leaves the bus, until the stream ends.
///
/// Pass a watcher that is already subscribed (see [`CallerWatcher::new`]): a caller that
/// leaves before the match rule is installed produces no signal, so subscribing inside
/// the loop would drop that departure.
pub async fn watch_callers(mut watcher: CallerWatcher, service: ScreenCastService) {
    while let Some(owner) = watcher.next().await {
        let ended = service.forget_owner(&owner).await;
        if ended > 0 {
            gnoblin_log::debug!(caller = %owner, ended, "a caller left the bus and its capture sessions ended");
        }
    }
}

/// Export the object and own the name.
///
/// The object is exported before the name is requested, because a caller that resolves
/// the name must find an object behind it: a name without an object is a method call
/// that never gets a reply.
///
/// # Errors
///
/// [`ScreenCastError::NameTaken`] when another process owns the name, which is the
/// ordinary case while Mutter runs beside this adapter.
pub async fn export(connection: &Connection, service: &ScreenCastService) -> Result<(), ScreenCastError> {
    service.take_export()?;
    connection
        .object_server()
        .at(PATH, service.clone())
        .await
        .map_err(|error| ScreenCastError::Bus {
            reason: error.to_string(),
        })?;
    service.attach(connection);
    connection
        .request_name_with_flags(BUS_NAME, fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
        .map_err(|_| ScreenCastError::NameTaken {
            name: BUS_NAME.to_owned(),
        })?;
    gnoblin_log::info!(name = BUS_NAME, path = PATH, "serving the screen cast adapter");
    Ok(())
}
