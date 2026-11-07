//! The served interface: `org.gnome.Shell.Introspect`.
//!
//! This is the interface GNOME Shell exports at `/org/gnome/Shell/Introspect`, and the one
//! `xdg-desktop-portal-gnome`'s window picker reads: `GetWindows` for the list, the two
//! change signals so the picker can redraw, `ScreenSize` to lay itself out and
//! `AnimationsEnabled` to decide whether it animates. Every member admits its call through
//! [`crate::compat::IntrospectAdmission`] first, so a disabled adapter answers nothing and an
//! unsupported member answers with the matrix's reason.
//!
//! Two things are worth stating rather than implying:
//!
//! - **The dictionary is the window object.** Each entry carries the properties the interface
//!   documents and the `window` object publishes: title, app-id, wm-class (which is the app id
//!   for a Wayland client, as it is in Mutter), is-hidden, has-focus, width and height.
//!   `client-type` is 0 and `is-hidden` is false because the schema publishes neither, and the
//!   matrix's `partial` note says so rather than leaving a caller to discover it.
//! - **A signal is a change to the list, not to a window.** `WindowsChanged` fires when the set
//!   of identities changes and `RunningApplicationsChanged` when the set of app ids does, which
//!   is what upstream promises: a title change is not a window opening.
//!
//! # Failure modes first
//!
//! 1. **An unsupported method fails silently.** `GetRunningApplications` answers with a D-Bus
//!    error that quotes the matrix: `an_unsupported_member_is_refused_by_name`.
//! 2. **The window list is stale.** It follows the graph's diffs, and a window that opens is
//!    announced by `WindowsChanged`: `get_windows_lists_the_window_objects`.
//! 3. **A picker shows a window that is gone.** A closed window leaves the dictionary and the
//!    signal fires once: `a_closed_window_leaves_the_list`.
//! 4. **The screen size is invented.** It is the union of the enabled outputs, and refused
//!    when nothing is enabled: `screen_size_is_the_union_of_the_enabled_outputs`.
//! 5. **Two exports in one process.** The second is refused:
//!    `an_adapter_registers_once`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use gnoblin_compat_core::{Gate, LockState, Mode, Refusal};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedVariant, fdo};

use crate::compat::{IntrospectAdmission, IntrospectCall, default_gate};
use crate::error::IntrospectError;
use crate::state::{IntrospectState, WindowInfo};

use crate::log;

/// The bus name GNOME Shell owns, which this adapter takes when it serves.
pub const BUS_NAME: &str = "org.gnome.Shell";

/// The object path the interface is exported on.
pub const PATH: &str = "/org/gnome/Shell/Introspect";

/// The interface name.
pub const INTERFACE: &str = "org.gnome.Shell.Introspect";

/// The interface's version, which is also what its `version` property reports.
pub const VERSION: u32 = 3;

/// One window's property dict.
type WindowProperties = HashMap<String, OwnedVariant>;

/// What `GetWindows` answers: one dictionary entry per window, keyed by identity.
type Windows = HashMap<u64, WindowProperties>;

/// The state the object server and the host share.
struct IntrospectShared {
    admission: IntrospectAdmission,
    gate: Mutex<Gate>,
    state: Mutex<IntrospectState>,
    emitter: Mutex<Option<Connection>>,
    exported: Mutex<bool>,
}

impl std::fmt::Debug for IntrospectShared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IntrospectShared")
            .field("admission", &self.admission)
            .field("gate", &self.gate)
            .field("state", &self.state)
            .field("emitter", &self.emitter)
            .field("exported", &self.exported)
            .finish()
    }
}

/// The `Introspect` object: exportable, clonable and cheap.
#[derive(Clone, Debug)]
pub struct IntrospectService {
    shared: Arc<IntrospectShared>,
}

impl IntrospectService {
    /// An adapter with the default gate.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn new() -> Result<IntrospectService, gnoblin_compat_core::MatrixError> {
        IntrospectService::with_gate(default_gate())
    }

    /// An adapter with a gate the composition root chose.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn with_gate(gate: Gate) -> Result<IntrospectService, gnoblin_compat_core::MatrixError> {
        Ok(IntrospectService {
            shared: Arc::new(IntrospectShared {
                admission: IntrospectAdmission::load()?,
                gate: Mutex::new(gate),
                state: Mutex::new(IntrospectState::default()),
                emitter: Mutex::new(None),
                exported: Mutex::new(false),
            }),
        })
    }

    /// Set the mode the gate screens with, which is `gnoblin.configure.compat.capture`.
    pub fn set_mode(&self, mode: Mode) {
        let mut gate = self.gate_lock();
        *gate = Gate::new(mode, gate.caller(), gate.lock());
    }

    /// The state the adapter last published.
    pub fn state(&self) -> IntrospectState {
        self.state_lock().clone()
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::Exported`] when this service was already exported.
    pub fn take_export(&self) -> Result<(), IntrospectError> {
        let mut exported = self.exported_lock();
        if *exported {
            return Err(IntrospectError::Exported);
        }
        *exported = true;
        Ok(())
    }

    /// Remember the connection, so a change can raise the two signals.
    pub fn attach(&self, connection: &Connection) {
        *self
            .shared
            .emitter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection.clone());
    }

    /// Publish the state the link read, raising the signals its changes imply.
    ///
    /// # Errors
    ///
    /// The bus error from emitting a signal. The state is stored either way: the window list
    /// is not a bus transaction.
    pub async fn publish(&self, state: IntrospectState) -> Result<(), zbus::Error> {
        let (windows_changed, applications_changed) = {
            let mut stored = self.state_lock();
            let windows_changed = stored.identities() != state.identities();
            let applications_changed = stored.applications() != state.applications();
            *stored = state;
            (windows_changed, applications_changed)
        };
        if !windows_changed && !applications_changed {
            return Ok(());
        }
        let emitter = self
            .shared
            .emitter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(connection) = emitter {
            let emitter = SignalEmitter::new(&connection, PATH)?;
            if windows_changed {
                IntrospectService::windows_changed(&emitter).await?;
            }
            if applications_changed {
                IntrospectService::running_applications_changed(&emitter).await?;
            }
        }
        Ok(())
    }

    fn gate_lock(&self) -> MutexGuard<'_, Gate> {
        self.shared.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn state_lock(&self) -> MutexGuard<'_, IntrospectState> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn exported_lock(&self) -> MutexGuard<'_, bool> {
        self.shared
            .exported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The gate for one call: the configured mode, an unsandboxed caller and the lock state
    /// the graph last published.
    fn gate(&self) -> Gate {
        let mode = self.gate_lock().mode();
        let lock = self.state_lock().lock;
        Gate::new(mode, Default::default(), lock)
    }

    fn admit(&self, call: IntrospectCall) -> Result<(), fdo::Error> {
        self.shared
            .admission
            .admit(call, self.gate())
            .map_err(|refusal: Refusal| fdo::Error::Failed(refusal.to_string()))
    }
}

/// The interface itself.
///
/// The methods are private on purpose: the object server calls them, and the state is the
/// whole of what this adapter does with a call.
#[zbus::interface(name = "org.gnome.Shell.Introspect")]
impl IntrospectService {
    /// Every window the desktop holds, keyed by identity.
    fn get_windows(&self) -> fdo::Result<Windows> {
        self.admit(IntrospectCall::GetWindows)?;
        let state = self.state_lock();
        Ok(state.windows.iter().map(window_entry).collect())
    }

    /// The application list, which this adapter does not serve.
    fn get_running_applications(&self) -> fdo::Result<HashMap<String, WindowProperties>> {
        self.admit(IntrospectCall::GetRunningApplications)?;
        Err(fdo::Error::Failed(
            "GetRunningApplications is unsupported: the schema publishes no application object".to_owned(),
        ))
    }

    /// Whether the shell's animations are enabled.
    ///
    /// Always `true`: the IPC publishes no animations switch yet, and a caller that read
    /// `false` would disable its own animations for no reason. The matrix's `partial` note is
    /// where a reader learns that.
    #[zbus(property)]
    fn animations_enabled(&self) -> fdo::Result<bool> {
        self.admit(IntrospectCall::AnimationsEnabled)?;
        Ok(true)
    }

    /// The screen's size: the union of the enabled logical monitors.
    #[zbus(property)]
    fn screen_size(&self) -> fdo::Result<(i32, i32)> {
        self.admit(IntrospectCall::ScreenSize)?;
        self.state_lock().screen_size().map_err(fdo::Error::from)
    }

    /// The interface version.
    ///
    /// The name is spelled out because the interface's property is lower case (`version`) and
    /// zbus would capitalise the Rust name into `Version`.
    #[zbus(property, name = "version")]
    fn version(&self) -> fdo::Result<u32> {
        self.admit(IntrospectCall::Version)?;
        Ok(VERSION)
    }

    /// A window opened or closed.
    #[zbus(signal)]
    async fn windows_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// The set of running applications changed.
    #[zbus(signal)]
    async fn running_applications_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// A string as the dictionary's value type.
///
/// `OwnedVariant` has no `From<String>`, because a string has to become a `Value` before it can
/// own its bytes; this is that one conversion, so the dictionary reads as what it is.
fn owned(text: String) -> OwnedVariant {
    OwnedVariant::try_from(zbus::Variant::from(text)).expect("a string converts to an owned value")
}

/// One window, as the interface's dictionary describes it.
///
/// `wm-class` is the app id because that is what Mutter reports for a Wayland client, and
/// `client-type` is 0 for the same reason: the schema publishes no client kind yet, and the
/// matrix's note on `GetWindows` says so.
fn window_entry(window: &WindowInfo) -> (u64, WindowProperties) {
    let mut properties = WindowProperties::new();
    properties.insert("title".to_owned(), owned(window.title.clone()));
    properties.insert("app-id".to_owned(), owned(window.app_id.clone()));
    properties.insert("wm-class".to_owned(), owned(window.app_id.clone()));
    properties.insert("client-type".to_owned(), OwnedVariant::from(0u32));
    properties.insert("is-hidden".to_owned(), OwnedVariant::from(false));
    properties.insert("has-focus".to_owned(), OwnedVariant::from(window.focused));
    properties.insert("width".to_owned(), OwnedVariant::from(window.width));
    properties.insert("height".to_owned(), OwnedVariant::from(window.height));
    (window.id, properties)
}

/// Export the object and own the name.
///
/// The object is exported before the name is requested, because a caller that resolves the
/// name must find an object behind it: a name without an object is a method call that never
/// gets a reply.
///
/// # Errors
///
/// [`IntrospectError::NameTaken`] when another process owns the name, which is the ordinary
/// case while GNOME Shell is running beside this adapter.
pub async fn export(connection: &Connection, service: &IntrospectService) -> Result<(), IntrospectError> {
    service.take_export()?;
    connection
        .object_server()
        .at(PATH, service.clone())
        .await
        .map_err(|error| IntrospectError::Bus {
            reason: error.to_string(),
        })?;
    service.attach(connection);
    connection
        .request_name_with_flags(BUS_NAME, fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
        .map_err(|_| IntrospectError::NameTaken {
            name: BUS_NAME.to_owned(),
        })?;
    log::info!(name = BUS_NAME, path = PATH, "serving shell introspection");
    Ok(())
}

/// The lock state the service last published, for a host's log line.
pub fn lock_state(service: &IntrospectService) -> LockState {
    service.state_lock().lock
}
