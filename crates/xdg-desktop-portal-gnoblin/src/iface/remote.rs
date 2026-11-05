//! `org.freedesktop.impl.portal.RemoteDesktop` and `InputCapture`.
//!
//! Both inject input, so both ask for [`PortalCapability::Capture`] and
//! [`PortalCapability::InputInjection`], and both deliver the injected input over EIS: the app
//! connects a libei session with `ConnectToEIS` and speaks the protocol on the descriptor,
//! so the older `Notify*` members are refused with a sentence naming EIS rather than
//! implemented as a second path that would disagree with the first.
//!
//! # What is real, and what is not
//!
//! Real: the session lifecycle (`CreateSession`, `SelectDevices`, `Start`), both consent
//! questions in order, the EIS enable/disable/release state, `Close` ending the session,
//! and the refusal of `Notify*` for a session that does not exist or was never started.
//!
//! Not real: the descriptor. [`crate::backends::EisBackend`] is the compositor's call, and
//! the default refuses with a sentence naming the missing member. A remote desktop
//! therefore asks the user and then answers `NotSupported`, which is the honest order: a
//! grant nobody can use is not created, because the grant is only recorded when the
//! question is answered and the call it enables is attempted straight after.

use std::collections::BTreeMap;

use gnoblin_types::OutputId;
use zbus::OwnedVariant;
use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::capture::{CURSOR_EMBEDDED, Options, recorded_uint, sandboxed, uint};
use super::{Portal, Reply, await_consent, export_request, export_session, refused, remove, sender, unsupported};
use crate::consent::{ConsentOutcome, Interface, NewSession, PortalCapability, State};

/// `types` bit: a keyboard.
pub const DEVICE_KEYBOARD: u32 = 1;
/// `types` bit: a pointer.
pub const DEVICE_POINTER: u32 = 2;
/// `types` bit: a touch screen.
pub const DEVICE_TOUCHSCREEN: u32 = 4;

/// The version of `RemoteDesktop` this build serves.
const REMOTE_DESKTOP_VERSION: u32 = 2;
/// The version of `InputCapture` this build serves.
const INPUT_CAPTURE_VERSION: u32 = 2;

/// `org.freedesktop.impl.portal.RemoteDesktop`.
#[derive(Debug)]
pub struct RemoteDesktop {
    portal: Portal,
}

impl RemoteDesktop {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> RemoteDesktop {
        RemoteDesktop {
            portal: Portal { shared, connection },
        }
    }

    /// Refuse one of the `Notify*` members.
    ///
    /// The check is the part that is real: the session must exist, belong to the caller
    /// and be started. Only then does the answer say why the member itself is not served.
    fn notifying(&self, caller: &str, path: &str) -> fdo::Result<()> {
        let shared = self.portal.lock();
        let session = shared
            .guard
            .session(caller, path)
            .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.to_owned() }))?;
        if session.state != State::Started {
            return Err(refused(crate::consent::Refusal::WrongState {
                path: path.to_owned(),
                state: session.state,
            }));
        }
        Err(not_supported())
    }
}

/// Why a `Notify*` member is not served.
fn not_supported() -> fdo::Error {
    fdo::Error::NotSupported(
        "remote desktop input travels over EIS in this build: call ConnectToEIS and speak libei on the descriptor"
            .to_owned(),
    )
}

#[zbus::interface(name = "org.freedesktop.impl.portal.RemoteDesktop")]
impl RemoteDesktop {
    /// Create a remote desktop session.
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller,
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&options),
                    interface: Interface::RemoteDesktop,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        export_session(&self.portal, &path).await?;
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let mut results = BTreeMap::new();
        results.insert(
            "session_id".to_owned(),
            zbus::OwnedVariant::try_from(zbus::Variant::from(path.as_str())).expect("a string is a value"),
        );
        tracing::info!(app = %app_id, session = %path, "a remote desktop session was created");
        Ok((0, results))
    }

    /// Record which devices the caller wants.
    async fn select_devices(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            let session = shared
                .guard
                .session(&caller, &path)
                .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?
                .clone();
            let selection = narrowed_devices(session.owner.narrowed(), &options);
            shared.guard.configure(&caller, &path, selection).map_err(refused)?;
        }
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        tracing::info!(app = %app_id, session = %path, "a remote desktop device selection was recorded");
        Ok((0, BTreeMap::new()))
    }

    /// Ask the user, then connect the EIS session.
    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        let _ = parent_window;
        let _ = options;
        // Both capabilities, capture first: a user who allows control but not viewing has
        // not answered the question that matters, and the second question is the narrower
        // one.
        let mut outcome =
            await_consent(&self.portal, handle.as_str(), &caller, &path, PortalCapability::Capture).await?;
        if outcome.granted() {
            outcome = await_consent(
                &self.portal,
                handle.as_str(),
                &caller,
                &path,
                PortalCapability::InputInjection,
            )
            .await?;
        }
        if !outcome.granted() {
            let _ = remove(&self.portal.connection, handle.as_str()).await;
            return Ok((outcome.response(), BTreeMap::new()));
        }
        let request = {
            let shared = self.portal.lock();
            let session = shared
                .guard
                .session(&caller, &path)
                .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
            crate::backends::StreamRequest {
                target: gnoblin_capture::Target::Output(OutputId::new(u64::from(
                    recorded_uint(&session.options, "output").unwrap_or(1),
                ))),
                cursor_mode: CURSOR_EMBEDDED,
                multiple: false,
            }
        };
        let opened = {
            let mut shared = self.portal.lock();
            let opened = shared.capture.open_stream(&request);
            if let Ok(stream) = &opened {
                let _ = shared.guard.record_stream(&path, stream.id);
            }
            opened
        };
        let stream = opened.map_err(unsupported)?;
        {
            let mut shared = self.portal.lock();
            shared.guard.start(&caller, &path).map_err(refused)?;
        }
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let mut properties = BTreeMap::new();
        for (name, value) in &stream.properties {
            if let Some(value) = super::variant(value) {
                properties.insert(name.clone(), value);
            }
        }
        let mut results = BTreeMap::new();
        results.insert(
            "streams".to_owned(),
            super::capture::streams(&[(stream.node, properties)]),
        );
        tracing::info!(app = %app_id, session = %path, "a remote desktop session started");
        Ok((0, results))
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_pointer_motion(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _dx: f64,
        _dy: f64,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_pointer_motion_absolute(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _stream: u32,
        _x: f64,
        _y: f64,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_pointer_button(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _button: i32,
        _state: u32,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_pointer_axis(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _dx: f64,
        _dy: f64,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_pointer_axis_discrete(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _axis: u32,
        _steps: i32,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_keyboard_keycode(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _keycode: i32,
        _state: u32,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_keyboard_keysym(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _keysym: i32,
        _state: u32,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    // The argument list is the D-Bus method's signature, fixed by the spec.
    #[allow(clippy::too_many_arguments)]
    async fn notify_touch_down(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _stream: u32,
        _slot: u32,
        _x: f64,
        _y: f64,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    // The argument list is the D-Bus method's signature, fixed by the spec.
    #[allow(clippy::too_many_arguments)]
    async fn notify_touch_motion(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _stream: u32,
        _slot: u32,
        _x: f64,
        _y: f64,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Refused: this build delivers input over EIS.
    async fn notify_touch_up(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _options: Options,
        _slot: u32,
    ) -> fdo::Result<()> {
        self.notifying(&sender(&header)?, session_handle.as_str())?;
        Ok(())
    }

    /// Connect an EIS session and hand the caller its descriptor.
    ///
    /// The grant is checked again here rather than trusted from `Start`: the descriptor is
    /// the thing that injects, and a caller that reached this member has to hold the grant
    /// now, not at some earlier call.
    async fn connect_to_eis(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: Options,
    ) -> fdo::Result<zbus::OwnedFd> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        let opened = {
            let mut shared = self.portal.lock();
            grant_live(&mut shared, &caller, &path)?;
            shared.eis.connect(&path, &app_id)
        };
        let fd = opened.map_err(unsupported)?;
        Ok(zbus::OwnedFd::from(fd))
    }

    /// The devices this build can offer.
    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        DEVICE_KEYBOARD | DEVICE_POINTER
    }

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        REMOTE_DESKTOP_VERSION
    }
}

/// Require that the input-injection grant is live for this caller and session.
///
/// A session with no live grant is refused rather than prompted: `Start` is the member
/// that asks, and a descriptor handed out from here would be a grant nobody gave.
fn grant_live(shared: &mut super::Shared, caller: &str, path: &str) -> fdo::Result<()> {
    let now = shared.now();
    match shared.guard.ask(now, caller, path, PortalCapability::InputInjection) {
        Ok(crate::consent::Pending::AlreadyGranted) => Ok(()),
        Ok(crate::consent::Pending::Asked { answered, .. }) => {
            // The waiting entry is dropped with the receiver; its deadline expires it.
            drop(answered);
            Err(fdo::Error::Failed(
                "the input-injection grant is not live; start the session first".to_owned(),
            ))
        }
        Err(refusal) => Err(refused(refusal)),
    }
}

/// Narrow a device selection to what this backend will record for this caller.
///
/// A sandboxed caller may not use the touchscreen: absolute input on the user's own screen
/// is the widest injection there is, and a touch screen is the device it needs.
#[must_use]
pub fn narrowed_devices(narrowed: bool, options: &Options) -> BTreeMap<String, gnoblin_model::Value> {
    let requested = uint(options, "types").unwrap_or(DEVICE_KEYBOARD | DEVICE_POINTER);
    let allowed = if narrowed {
        DEVICE_KEYBOARD | DEVICE_POINTER
    } else {
        DEVICE_KEYBOARD | DEVICE_POINTER | DEVICE_TOUCHSCREEN
    };
    let mut selection = BTreeMap::new();
    selection.insert(
        "types".to_owned(),
        gnoblin_model::Value::Uint(u64::from(requested & allowed)),
    );
    selection
}

/// `org.freedesktop.impl.portal.InputCapture`.
#[derive(Debug)]
pub struct InputCapture {
    portal: Portal,
}

impl InputCapture {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> InputCapture {
        InputCapture {
            portal: Portal { shared, connection },
        }
    }

    /// Check a session is started and owned, then perform `step`.
    fn started<T>(
        &self,
        caller: &str,
        path: &str,
        step: impl FnOnce() -> Result<T, crate::backends::Unavailable>,
    ) -> fdo::Result<T> {
        {
            let shared = self.portal.lock();
            let session = shared
                .guard
                .session(caller, path)
                .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.to_owned() }))?;
            if session.state != State::Started {
                return Err(refused(crate::consent::Refusal::WrongState {
                    path: path.to_owned(),
                    state: session.state,
                }));
            }
        }
        step().map_err(unsupported)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.InputCapture")]
impl InputCapture {
    /// Create an input capture session.
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller,
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&options),
                    interface: Interface::InputCapture,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        export_session(&self.portal, &path).await?;
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let mut results = BTreeMap::new();
        results.insert(
            "session_id".to_owned(),
            zbus::OwnedVariant::try_from(zbus::Variant::from(path.as_str())).expect("a string is a value"),
        );
        tracing::info!(app = %app_id, session = %path, "an input capture session was created");
        Ok((0, results))
    }

    /// Start the session: ask for input injection, then open the barrier state.
    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        let outcome = await_consent(
            &self.portal,
            handle.as_str(),
            &caller,
            &path,
            PortalCapability::InputInjection,
        )
        .await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let request = {
                    let shared = self.portal.lock();
                    let session = shared
                        .guard
                        .session(&caller, &path)
                        .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
                    crate::backends::StreamRequest {
                        target: gnoblin_capture::Target::Output(OutputId::new(u64::from(
                            recorded_uint(&session.options, "output").unwrap_or(1),
                        ))),
                        cursor_mode: CURSOR_EMBEDDED,
                        multiple: false,
                    }
                };
                let opened = {
                    let mut shared = self.portal.lock();
                    let opened = shared.capture.open_stream(&request);
                    if let Ok(stream) = &opened {
                        let _ = shared.guard.record_stream(&path, stream.id);
                    }
                    opened
                };
                let stream = opened.map_err(unsupported)?;
                {
                    let mut shared = self.portal.lock();
                    shared.guard.start(&caller, &path).map_err(refused)?;
                }
                let mut properties = BTreeMap::new();
                for (name, value) in &stream.properties {
                    if let Some(value) = super::variant(value) {
                        properties.insert(name.clone(), value);
                    }
                }
                let mut results = BTreeMap::new();
                results.insert(
                    "streams".to_owned(),
                    super::capture::streams(&[(stream.node, properties)]),
                );
                (0, results)
            }
            Ok(outcome) => (outcome.response(), BTreeMap::new()),
            Err(refusal) => return Err(refusal),
        };
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        tracing::info!(app = %app_id, session = %path, response = reply.0, "an input capture start finished");
        Ok(reply)
    }

    /// The zones a pointer may cross.
    ///
    /// Refused: zones are the compositor's layout state, and the schema has no member for
    /// it. An empty zone list would tell the app it may never capture, which is worse than
    /// saying the call is not served.
    async fn get_zones(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        self.started(&caller, &path, || {
            Err(crate::backends::Unavailable::missing("input-capture-zones"))
        })?;
        Ok((0, BTreeMap::new()))
    }

    /// Record the barriers the app wants.
    // The argument list is the D-Bus method's signature, fixed by the spec.
    #[allow(clippy::too_many_arguments)]
    async fn set_pointer_barriers(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: Options,
        _barriers: Vec<BTreeMap<String, OwnedVariant>>,
        _zone_set: u32,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        self.started(&caller, &path, || {
            Err(crate::backends::Unavailable::missing("input-capture-barriers"))
        })?;
        Ok((0, BTreeMap::new()))
    }

    /// Let the app capture input.
    ///
    /// The grant must be live, and enabling is a state change on the session rather than a
    /// compositor call: what captures is the EIS session, and this is the flag a barrier
    /// crossing turns on.
    async fn enable(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        {
            let mut shared = self.portal.lock();
            let now = shared.now();
            match shared.guard.ask(now, &caller, &path, PortalCapability::InputInjection) {
                Ok(crate::consent::Pending::AlreadyGranted) => {}
                Ok(crate::consent::Pending::Asked { .. }) => return Ok((2, BTreeMap::new())),
                Err(refusal) => return Err(refused(refusal)),
            }
        }
        Ok((0, BTreeMap::new()))
    }

    /// Stop capturing.
    async fn disable(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        {
            let shared = self.portal.lock();
            let session = shared.guard.session(&caller, session_handle.as_str()).ok_or_else(|| {
                refused(crate::consent::Refusal::UnknownSession {
                    path: session_handle.as_str().to_owned(),
                })
            })?;
            if session.state != State::Started {
                return Err(refused(crate::consent::Refusal::WrongState {
                    path: session_handle.as_str().to_owned(),
                    state: session.state,
                }));
            }
        }
        Ok((0, BTreeMap::new()))
    }

    /// Release the barriers.
    async fn release(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        _app_id: String,
        _options: Options,
    ) -> fdo::Result<Reply> {
        self.disable(header, session_handle, String::new(), BTreeMap::new())
            .await
    }

    /// Connect an EIS session and hand the caller its descriptor.
    async fn connect_to_eis(
        &self,
        #[zbus(header)] header: Header<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: Options,
    ) -> fdo::Result<zbus::OwnedFd> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        let opened = {
            let mut shared = self.portal.lock();
            grant_live(&mut shared, &caller, &path)?;
            shared.eis.connect(&path, &app_id)
        };
        let fd = opened.map_err(unsupported)?;
        Ok(zbus::OwnedFd::from(fd))
    }

    /// What this build's capture can tell a client.
    #[zbus(property)]
    fn supported_capabilities(&self) -> u32 {
        // Bit 1 is "EIS barriers are supported", bit 2 "Linux input events are reported".
        // Neither is, until the compositor serves a member for them, so the property is
        // zero rather than a promise.
        0
    }

    /// The interface version.
    #[zbus(property)]
    fn version(&self) -> u32 {
        INPUT_CAPTURE_VERSION
    }
}
