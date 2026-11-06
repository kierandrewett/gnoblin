//! The served interface: `org.gnome.Mutter.IdleMonitor`.
//!
//! This is the name Mutter owns at `/org/gnome/Mutter/IdleMonitor/Core`, and the one
//! `gsd-power`, `gnome-screensaver` and Gnoblin's own `src/idle.c` talk to. Every member
//! admits its call through [`crate::compat::IdleMonitorAdmission`] first, so a disabled
//! adapter answers nothing and an unsupported member answers with the matrix's reason.
//!
//! Three things are worth stating rather than implying:
//!
//! - **The counter is the compositor's.** `GetIdletime` returns what the `idle` object last
//!   published; the adapter holds no clock and does not measure anything, so the value a
//!   caller reads and the value a watch fires on are the same number.
//! - **A watch belongs to its caller.** It is recorded against the caller's bus identity,
//!   another caller cannot remove it, and it ends when that identity leaves the bus
//!   ([`CallerWatcher`]). That last part is the difference between an idle policy and a
//!   leak: a watch that outlived its caller would fire at a bus name nobody holds.
//! - **A fired watch is a signal on the interface.** `WatchFired` carries the id, so a
//!   caller that holds several watches tells them apart by the id it was given, exactly as
//!   upstream does.
//!
//! # Failure modes first
//!
//! 1. **A watch outlives its caller.** It is removed when the caller's bus identity
//!    vanishes, and a test proves the registry is empty afterwards:
//!    `a_watch_ends_with_its_callers_connection`.
//! 2. **One caller removes another's watch.** Refused with the id named:
//!    `a_watch_belongs_to_its_caller`.
//! 3. **An unsupported member fails silently.** `ResetIdletime` answers with a D-Bus error
//!    that quotes the matrix: `an_unsupported_member_is_refused_by_name`.
//! 4. **A graph with no counter reports zero.** Refused at connect instead:
//!    `a_graph_without_an_idle_counter_is_refused`.
//! 5. **A fired user-active watch fires again.** It is removed when it fires, and a second
//!    activity does not produce a second signal: `an_active_watch_fires_once`.

use std::sync::{Arc, Mutex, MutexGuard};

use gnoblin_compat_core::{CallerWatcher, Gate, LockState, Mode, Refusal};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

use zbus::{Connection, fdo};

use crate::compat::{IdleMonitorAdmission, IdleMonitorCall, default_gate};
use crate::error::IdleError;
use crate::link::IdleState;
use crate::watches::Watches;

/// The bus name Mutter owns, which this adapter takes when it serves.
pub const BUS_NAME: &str = "org.gnome.Mutter.IdleMonitor";

/// The object path Mutter exports it on.
pub const PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";

/// The interface name.
pub const INTERFACE: &str = "org.gnome.Mutter.IdleMonitor";

/// The interface's version. Upstream carries no version member, so this is the one the name
/// has always meant.
pub const VERSION: u32 = 1;

/// The state the object server and the host share.
struct IdleShared {
    admission: IdleMonitorAdmission,
    gate: Mutex<Gate>,
    state: Mutex<IdleState>,
    watches: Mutex<Watches>,
    emitter: Mutex<Option<Connection>>,
    exported: Mutex<bool>,
}

impl std::fmt::Debug for IdleShared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdleShared")
            .field("admission", &self.admission)
            .field("gate", &self.gate)
            .field("state", &self.state)
            .field("watches", &self.watches)
            .field("emitter", &self.emitter)
            .field("exported", &self.exported)
            .finish()
    }
}

/// The `IdleMonitor` object: exportable, clonable and cheap.
#[derive(Clone, Debug)]
pub struct IdleMonitorService {
    shared: Arc<IdleShared>,
}

impl IdleMonitorService {
    /// An adapter with the default gate.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn new() -> Result<IdleMonitorService, gnoblin_compat_core::MatrixError> {
        IdleMonitorService::with_gate(default_gate())
    }

    /// An adapter with a gate the composition root chose.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn with_gate(gate: Gate) -> Result<IdleMonitorService, gnoblin_compat_core::MatrixError> {
        Ok(IdleMonitorService {
            shared: Arc::new(IdleShared {
                admission: IdleMonitorAdmission::load()?,
                gate: Mutex::new(gate),
                state: Mutex::new(IdleState::default()),
                watches: Mutex::new(Watches::new()),
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

    /// How many watches are live, for a host's log line or a test.
    pub fn watch_count(&self) -> usize {
        self.watches_lock().len()
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// # Errors
    ///
    /// [`IdleError::Exported`] when this service was already exported.
    pub fn take_export(&self) -> Result<(), IdleError> {
        let mut exported = self.exported_lock();
        if *exported {
            return Err(IdleError::Exported);
        }
        *exported = true;
        Ok(())
    }

    /// Remember the connection, so a fired watch can raise `WatchFired`.
    pub fn attach(&self, connection: &Connection) {
        *self
            .shared
            .emitter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection.clone());
    }

    /// Publish the counter the link read, firing every watch the new idle time reaches.
    ///
    /// # Errors
    ///
    /// The bus error from emitting a signal. The counter is stored either way: the
    /// compositor's idle time is not a bus transaction.
    pub async fn publish(&self, state: IdleState) -> Result<(), zbus::Error> {
        let fired = {
            let mut stored = self.state_lock();
            *stored = state;
            self.watches_lock().advance(state.idle_ms)
        };
        if fired.is_empty() {
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
            for id in fired {
                IdleMonitorService::watch_fired(&emitter, id).await?;
            }
        }
        Ok(())
    }

    /// Drop every watch a caller owned, answering how many went.
    pub fn forget_owner(&self, owner: &str) -> usize {
        self.watches_lock().forget_owner(owner)
    }

    fn gate_lock(&self) -> MutexGuard<'_, Gate> {
        self.shared.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn state_lock(&self) -> MutexGuard<'_, IdleState> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn watches_lock(&self) -> MutexGuard<'_, Watches> {
        self.shared
            .watches
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

    fn admit(&self, call: IdleMonitorCall) -> Result<(), fdo::Error> {
        self.shared
            .admission
            .admit(call, self.gate())
            .map_err(|refusal: Refusal| fdo::Error::Failed(refusal.to_string()))
    }

    /// The caller's bus identity, which is what a watch is recorded against.
    fn owner(header: &Header<'_>) -> fdo::Result<String> {
        header
            .sender()
            .map(ToString::to_string)
            // The bus always fills the sender in; a message without one is not a caller this
            // adapter can answer.
            .ok_or_else(|| fdo::Error::Failed("the call carried no sender".to_owned()))
    }
}

/// The interface itself.
///
/// The methods are private on purpose: the object server calls them, and the registry and
/// the published counter are the whole of what this adapter does with a call.
#[zbus::interface(name = "org.gnome.Mutter.IdleMonitor")]
impl IdleMonitorService {
    /// How long the session has been idle, in milliseconds.
    fn get_idletime(&self) -> fdo::Result<u64> {
        self.admit(IdleMonitorCall::GetIdletime)?;
        Ok(self.state_lock().idle_ms)
    }

    /// Watch for the idle time reaching `interval` milliseconds.
    ///
    /// The watch fires every time the idle time crosses the interval, which is what a
    /// screensaver with two thresholds and a "dim now" button all need from one registry.
    fn add_idle_watch(&self, #[zbus(header)] header: Header<'_>, interval: u64) -> fdo::Result<u32> {
        self.admit(IdleMonitorCall::AddIdleWatch)?;
        let owner = Self::owner(&header)?;
        let id = self.watches_lock().add_idle(&owner, interval);
        gnoblin_log::debug_in!("compat-idle", id, interval, "an idle watch was added");
        Ok(id)
    }

    /// Watch for the user becoming active again. The watch fires once and is gone.
    fn add_user_active_watch(&self, #[zbus(header)] header: Header<'_>) -> fdo::Result<u32> {
        self.admit(IdleMonitorCall::AddUserActiveWatch)?;
        let owner = Self::owner(&header)?;
        let id = self.watches_lock().add_active(&owner);
        gnoblin_log::debug_in!("compat-idle", id, "a user-active watch was added");
        Ok(id)
    }

    /// Remove a watch this caller added.
    fn remove_watch(&self, #[zbus(header)] header: Header<'_>, id: u32) -> fdo::Result<()> {
        self.admit(IdleMonitorCall::RemoveWatch)?;
        let owner = Self::owner(&header)?;
        self.watches_lock().remove(&owner, id).map_err(fdo::Error::from)
    }

    /// Reset the idle counter, which this adapter does not serve.
    fn reset_idletime(&self) -> fdo::Result<()> {
        self.admit(IdleMonitorCall::ResetIdletime)?;
        Err(fdo::Error::Failed(
            "ResetIdletime is unsupported: the idle counter is the compositor's".to_owned(),
        ))
    }

    /// A watch reached its condition.
    #[zbus(signal)]
    async fn watch_fired(emitter: &SignalEmitter<'_>, id: u32) -> zbus::Result<()>;
}

/// Remove the watches of every caller that leaves the bus, until the stream ends.
///
/// This is the loop a host runs beside its connection's service loop; it ends when the
/// connection does.
pub async fn watch_callers(connection: Connection, service: IdleMonitorService) {
    let Ok(mut watcher) = CallerWatcher::new(&connection).await else {
        gnoblin_log::error_in!(
            "compat-idle",
            "[compat] the idle adapter could not watch caller lifetimes"
        );
        return;
    };
    while let Some(owner) = watcher.next().await {
        let removed = service.forget_owner(&owner);
        if removed > 0 {
            gnoblin_log::debug_in!("compat-idle", caller = %owner, removed, "a caller left the bus and its watches ended");
        }
    }
}

/// Export the object and own the name.
///
/// The object is exported before the name is requested, because a caller that resolves the
/// name must find an object behind it: a name without an object is a method call that never
/// gets a reply.
///
/// # Errors
///
/// [`IdleError::NameTaken`] when another process owns the name, which is the ordinary case
/// while Mutter is running beside this adapter.
pub async fn export(connection: &Connection, service: &IdleMonitorService) -> Result<(), IdleError> {
    service.take_export()?;
    connection
        .object_server()
        .at(PATH, service.clone())
        .await
        .map_err(|error| IdleError::Bus {
            reason: error.to_string(),
        })?;
    service.attach(connection);
    connection
        .request_name_with_flags(BUS_NAME, fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
        .map_err(|_| IdleError::NameTaken {
            name: BUS_NAME.to_owned(),
        })?;
    gnoblin_log::info_in!("compat-idle", name = BUS_NAME, path = PATH, "serving the idle monitor");
    Ok(())
}

/// The lock state the service last published, for a host's log line.
pub fn lock_state(service: &IdleMonitorService) -> LockState {
    service.state_lock().lock
}
