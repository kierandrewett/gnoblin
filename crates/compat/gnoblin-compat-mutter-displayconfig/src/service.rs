//! The served interface: `org.gnome.Mutter.DisplayConfig`.
//!
//! This is the name Mutter owns and the interface GNOME Settings, the stock GNOME
//! portal and `gnome-remote-desktop` read to learn the desktop's layout. The members
//! this module implements are the ones [`crate::compat::MATRIX`] declares, and every
//! one of them admits the call first, so a refused member never reaches the mapping.
//!
//! Three things are worth stating rather than implying:
//!
//! - **The serial is this adapter's.** Mutter's serial is its own configuration
//!   counter; here it is the count of states the link published, which makes
//!   `ApplyMonitorsConfig` refuse a configuration read before the last hotplug. A
//!   caller that sends a serial it invented is refused, which is the behaviour
//!   `StaleSerial` exists for.
//! - **One output is one logical monitor.** The schema's `output` object carries the
//!   position, scale and transform, so a logical monitor is a property of an output
//!   rather than an object of its own. A configuration that names several connectors
//!   in one logical monitor gives each of them the same position and scale.
//! - **A write goes through the object's own method.** `ApplyMonitorsConfig` calls
//!   `apply` on each `output` object; it does not keep a second way to change the
//!   layout. [`OutputApplier`] is the seam that carries those calls to the IPC
//!   connection, which is what lets this object stay `Send + Sync` while the client is
//!   not.
//!
//! # Failure modes first
//!
//! 1. **A member the matrix calls unsupported is called.** Refused with the matrix's
//!    reason, before any mapping runs: `an_unsupported_member_is_refused_by_name`.
//! 2. **A write is attempted in the default mode.** Refused with `write-not-enabled`,
//!    and the property that announces writability says so:
//!    `an_apply_is_refused_in_the_default_mode`.
//! 3. **An apply races a hotplug.** Refused with the current serial named:
//!    `an_apply_with_a_stale_serial_is_refused`.
//! 4. **Two exports in one process.** The second is refused rather than replacing the
//!    object behind a live name: `an_adapter_registers_once`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use gnoblin_compat_core::{Gate, LockState, Mode, Refusal};
use gnoblin_model::Fields;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedVariant, fdo};

use crate::compat::{DisplayConfigAdmission, DisplayConfigCall, default_gate};
use crate::error::DisplayError;
use crate::state::{ApplyMethod, ApplyPlan, DisplayOutput, DisplayState, LogicalMonitor, MonitorAssignment};

/// The bus name Mutter owns, which this adapter takes when it serves.
pub const BUS_NAME: &str = "org.gnome.Mutter.DisplayConfig";

/// The object path Mutter exports it on.
pub const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

/// The interface name.
pub const INTERFACE: &str = "org.gnome.Mutter.DisplayConfig";

/// The interface's version, as Mutter 51.0 reports it.
pub const VERSION: u32 = 3;

/// A monitor's property dict, which is empty in this slice.
type MonitorProperties = HashMap<String, OwnedVariant>;

/// One available mode: id, width, height, refresh, preferred scale, supported scales,
/// properties.
type ModeEntry = (String, i32, i32, f64, f64, Vec<f64>, MonitorProperties);

/// One monitor: connector, vendor, product, serial; modes; properties.
type MonitorEntry = ((String, String, String, String), Vec<ModeEntry>, MonitorProperties);

/// One logical monitor: x, y, scale, transform, primary, its monitors, properties.
type LogicalEntry = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<(String, String, String, String)>,
    MonitorProperties,
);

/// What `GetCurrentState` answers: serial, monitors, logical monitors, properties.
type CurrentState = (u32, Vec<MonitorEntry>, Vec<LogicalEntry>, MonitorProperties);

/// One logical monitor as `ApplyMonitorsConfig` takes it: position, scale, transform,
/// primary, then each monitor as a connector, a mode id and a property dict.
///
/// The shape is the interface's, so it is spelled the way the vendored XML spells it
/// rather than being flattened into a struct this crate would then have to convert.
#[allow(clippy::type_complexity)]
type LogicalArg = (i32, i32, f64, u32, bool, Vec<(String, String, MonitorProperties)>);

/// One CRTC as the legacy `ApplyConfiguration` takes it, which this adapter only refuses.
#[allow(clippy::type_complexity)]
type CrtcEntry = (u32, i32, i32, i32, u32, Vec<u32>, MonitorProperties);

/// The write half's seam: one output's new configuration, carried to the IPC.
///
/// The service cannot hold the client itself. A zbus interface object must be `Send +
/// Sync`, and the client a host threads onto the IPC is a blocking `Client<C>` whose
/// transport may not be — the in-process link the tests use certainly is not. This
/// trait is the one place the two halves meet, so the D-Bus object stays a plain
/// `Arc<Mutex<…>>` and the client stays where it belongs.
///
/// A host implements it over the connection it already owns; the tests implement it by
/// recording, because what the mapping has to prove is which calls it makes, and the
/// client half is proved by the read tests over the loopback.
pub trait OutputApplier: Send + Sync {
    /// Apply `fields` to the output called `output`.
    ///
    /// # Errors
    ///
    /// The compositor's refusal, or why the call never completed, as a sentence a
    /// D-Bus caller reads.
    fn apply(&self, output: &str, fields: &Fields) -> Result<(), String>;
}

/// What the adapter last published, and what it is serving with.
#[derive(Debug)]
struct Reported {
    /// The serial a caller sends back, one more than the previous state's.
    serial: u32,
    /// The outputs.
    state: DisplayState,
    /// The lock state the gate reads.
    lock: LockState,
}

/// The state the object server and the host share.
///
/// `Debug` is written out rather than derived because the write seam is a trait object,
/// and a host's applier is not something this crate can print.
struct DisplayShared {
    admission: DisplayConfigAdmission,
    gate: Mutex<Gate>,
    published: Mutex<Reported>,
    applier: Box<dyn OutputApplier>,
    emitter: Mutex<Option<Connection>>,
    exported: Mutex<bool>,
}

/// The `DisplayConfig` object: exportable, clonable and cheap.
#[derive(Clone)]
pub struct DisplayConfigService {
    shared: Arc<DisplayShared>,
}

impl std::fmt::Debug for DisplayShared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DisplayShared")
            .field("admission", &self.admission)
            .field("gate", &self.gate)
            .field("published", &self.published)
            .field("applier", &"<the host's write seam>")
            .field("emitter", &self.emitter)
            .field("exported", &self.exported)
            .finish()
    }
}

impl std::fmt::Debug for DisplayConfigService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("DisplayConfigService")
            .field(&self.shared)
            .finish()
    }
}

impl DisplayConfigService {
    /// An adapter with the default gate and the supplied write seam.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn new(applier: Box<dyn OutputApplier>) -> Result<DisplayConfigService, gnoblin_compat_core::MatrixError> {
        DisplayConfigService::with_gate(default_gate(), applier)
    }

    /// An adapter with a gate the composition root chose.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is unreadable.
    pub fn with_gate(
        gate: Gate,
        applier: Box<dyn OutputApplier>,
    ) -> Result<DisplayConfigService, gnoblin_compat_core::MatrixError> {
        Ok(DisplayConfigService {
            shared: Arc::new(DisplayShared {
                admission: DisplayConfigAdmission::load()?,
                gate: Mutex::new(gate),
                published: Mutex::new(Reported {
                    serial: 1,
                    state: DisplayState::default(),
                    lock: LockState::Unlocked,
                }),
                applier,
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

    /// The mode the gate screens with.
    pub fn mode(&self) -> Mode {
        self.gate_lock().mode()
    }

    /// The serial the adapter last published.
    pub fn serial(&self) -> u32 {
        self.published_lock().serial
    }

    /// The state the adapter last published.
    pub fn state(&self) -> DisplayState {
        self.published_lock().state.clone()
    }

    /// Record that this service was exported, or refuse a second export.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Exported`] when this service was already exported: a second
    /// export would replace the object behind a live name while the first export's
    /// signal emitter is still pointing at the same path.
    pub fn take_export(&self) -> Result<(), DisplayError> {
        let mut exported = self.exported_lock();
        if *exported {
            return Err(DisplayError::Exported);
        }
        *exported = true;
        Ok(())
    }

    /// Remember the connection, so a change can raise `MonitorsChanged`.
    pub fn attach(&self, connection: &Connection) {
        *self
            .shared
            .emitter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection.clone());
    }

    /// Publish the state the link read, raising `MonitorsChanged` when it changed.
    ///
    /// The serial advances exactly when the outputs changed, so a caller that read the
    /// state and then applies a configuration is refused only if something really moved
    /// underneath it.
    ///
    /// # Errors
    ///
    /// The bus error from emitting the signal. The state is stored either way: the
    /// compositor's layout is not a bus transaction.
    pub async fn publish(&self, state: DisplayState, lock: LockState) -> Result<bool, zbus::Error> {
        let changed = {
            let mut published = self.published_lock();
            let changed = published.state != state;
            if changed {
                published.serial = published.serial.wrapping_add(1).max(1);
            }
            published.state = state;
            published.lock = lock;
            changed
        };
        if changed {
            let emitter = self
                .shared
                .emitter
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(connection) = emitter {
                let emitter = SignalEmitter::new(&connection, PATH)?;
                DisplayConfigService::monitors_changed(&emitter).await?;
            }
        }
        Ok(changed)
    }

    fn gate_lock(&self) -> MutexGuard<'_, Gate> {
        self.shared.gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn published_lock(&self) -> MutexGuard<'_, Reported> {
        self.shared
            .published
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn exported_lock(&self) -> MutexGuard<'_, bool> {
        self.shared
            .exported
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The gate for one call: the configured mode, an unsandboxed caller and the lock
    /// state the graph last published.
    fn gate(&self) -> Gate {
        let mode = self.gate_lock().mode();
        let lock = self.published_lock().lock;
        Gate::new(mode, Default::default(), lock)
    }

    fn admit(&self, call: DisplayConfigCall) -> Result<(), fdo::Error> {
        self.shared
            .admission
            .admit(call, self.gate())
            .map_err(|refusal: Refusal| fdo::Error::Failed(refusal.to_string()))
    }
}

/// The interface itself.
///
/// The methods are private on purpose: the object server calls them, and the mapping
/// they perform is the whole of what this adapter does with a call.
#[zbus::interface(name = "org.gnome.Mutter.DisplayConfig")]
impl DisplayConfigService {
    /// The whole display state: serial, monitors, logical monitors, properties.
    async fn get_current_state(&self) -> fdo::Result<CurrentState> {
        self.admit(DisplayConfigCall::GetCurrentState)?;
        let published = self.published_lock();
        let mut monitors = Vec::with_capacity(published.state.outputs.len());
        let mut logical = Vec::new();
        let primary = published
            .state
            .outputs
            .iter()
            .find(|output| output.enabled && output.primary)
            .or_else(|| published.state.outputs.iter().find(|output| output.enabled))
            .map(|output| output.name.clone());
        for output in &published.state.outputs {
            monitors.push(monitor_entry(output));
            if output.enabled {
                logical.push(logical_entry(output, primary.as_deref() == Some(output.name.as_str())));
            }
        }
        Ok((published.serial, monitors, logical, MonitorProperties::new()))
    }

    /// Apply a configuration.
    ///
    /// `properties` is accepted and ignored: the only global property upstream defines
    /// is the layout mode, which this slice has no member for, and a caller that sends
    /// one gets a desktop laid out logically rather than a silent difference between
    /// what it asked for and what it got.
    fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<LogicalArg>,
        _properties: MonitorProperties,
    ) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::ApplyMonitorsConfig)?;
        let method =
            ApplyMethod::parse(method).ok_or_else(|| fdo::Error::from(DisplayError::UnknownMethod { method }))?;
        let plan = ApplyPlan {
            serial,
            method,
            logical: logical_monitors
                .into_iter()
                .map(|(x, y, scale, transform, primary, monitors)| LogicalMonitor {
                    x,
                    y,
                    scale,
                    transform,
                    primary,
                    monitors: monitors
                        .into_iter()
                        .map(|(connector, mode, _properties)| MonitorAssignment { connector, mode })
                        .collect(),
                })
                .collect(),
        };
        let (state, current) = {
            let published = self.published_lock();
            (published.state.clone(), published.serial)
        };
        plan.validate(&state, current).map_err(fdo::Error::from)?;
        if method == ApplyMethod::Verify {
            // Verification is a real answer: the plan was checked and nothing was
            // written, which is exactly what the method asks for.
            gnoblin_log::debug!("a configuration verified without being applied");
            return Ok(());
        }
        for (connector, fields) in plan.calls(&state) {
            self.shared
                .applier
                .apply(&connector, &fields)
                .map_err(|reason| fdo::Error::Failed(format!("`{connector}` refused the configuration: {reason}")))?;
        }
        gnoblin_log::info!(serial, outputs = state.outputs.len(), "applied a monitor configuration");
        Ok(())
    }

    /// Whether this adapter would accept an `ApplyMonitorsConfig` right now.
    ///
    /// It is the mode, not a stored flag: a caller reads it to decide whether to offer
    /// a configuration at all, so it must answer the gate's own question.
    #[zbus(property)]
    fn apply_monitors_config_allowed(&self) -> fdo::Result<bool> {
        self.admit(DisplayConfigCall::ApplyMonitorsConfigAllowed)?;
        Ok(self.gate().mode() == Mode::Unmediated)
    }

    /// Whether this compositor rotates the panel by itself.
    ///
    /// `false`: orientation is a property of the output a caller sets, and the
    /// compositor does not second-guess it.
    #[zbus(property)]
    fn panel_orientation_managed(&self) -> fdo::Result<bool> {
        self.admit(DisplayConfigCall::PanelOrientationManaged)?;
        Ok(false)
    }

    /// Whether Night Light is available through this interface.
    ///
    /// `false`: Night Light is the services role's (D15) and the IPC publishes no
    /// colour temperature for this adapter to report.
    #[zbus(property)]
    fn night_light_supported(&self) -> fdo::Result<bool> {
        self.admit(DisplayConfigCall::NightLightSupported)?;
        Ok(false)
    }

    /// The legacy resource list, which this adapter does not serve.
    fn get_resources(&self) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::GetResources)?;
        Err(fdo::Error::Failed(
            "GetResources is unsupported: the `output` objects and GetCurrentState replace it".to_owned(),
        ))
    }

    /// The legacy configuration call, which this adapter does not serve.
    fn apply_configuration(
        &self,
        _serial: u32,
        _persistent: bool,
        _crtcs: Vec<CrtcEntry>,
        _outputs: Vec<(u32, MonitorProperties)>,
    ) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::ApplyConfiguration)?;
        Err(fdo::Error::Failed(
            "ApplyConfiguration is unsupported: a caller uses ApplyMonitorsConfig".to_owned(),
        ))
    }

    /// The legacy backlight property, which the schema does not publish.
    #[zbus(property)]
    fn backlight(&self) -> fdo::Result<(u32, Vec<MonitorProperties>)> {
        self.admit(DisplayConfigCall::Backlight)?;
        Err(fdo::Error::Failed(
            "Backlight is unsupported: the schema publishes no backlight".to_owned(),
        ))
    }

    /// The legacy backlight call.
    fn change_backlight(&self, _serial: u32, _output: u32, _value: i32) -> fdo::Result<i32> {
        self.admit(DisplayConfigCall::ChangeBacklight)?;
        Err(fdo::Error::Failed(
            "ChangeBacklight is unsupported: the schema publishes no backlight".to_owned(),
        ))
    }

    /// The legacy backlight call.
    fn set_backlight(&self, _serial: u32, _connector: String, _value: i32) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::SetBacklight)?;
        Err(fdo::Error::Failed(
            "SetBacklight is unsupported: the schema publishes no backlight".to_owned(),
        ))
    }

    /// Display power saving, which has no IPC member.
    #[zbus(property)]
    fn power_save_mode(&self) -> fdo::Result<i32> {
        self.admit(DisplayConfigCall::PowerSaveMode)?;
        Err(fdo::Error::Failed(
            "PowerSaveMode is unsupported: display power saving has no IPC member".to_owned(),
        ))
    }

    /// Display power saving, which has no IPC member.
    #[zbus(property)]
    fn set_power_save_mode(&self, _mode: i32) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::PowerSaveMode)?;
        Err(fdo::Error::Failed(
            "PowerSaveMode is unsupported: display power saving has no IPC member".to_owned(),
        ))
    }

    /// Whether an external monitor is active, which the schema cannot answer.
    #[zbus(property)]
    fn has_external_monitor(&self) -> fdo::Result<bool> {
        self.admit(DisplayConfigCall::HasExternalMonitor)?;
        Err(fdo::Error::Failed(
            "HasExternalMonitor is unsupported: the schema does not publish whether an output is built in".to_owned(),
        ))
    }

    /// CRTC gamma, which the legacy path owns.
    fn get_crtc_gamma(&self, _serial: u32, _crtc: u32) -> fdo::Result<(Vec<u16>, Vec<u16>, Vec<u16>)> {
        self.admit(DisplayConfigCall::GetCrtcGamma)?;
        Err(fdo::Error::Failed(
            "GetCrtcGamma is unsupported: CRTC gamma is a hardware detail of the legacy path".to_owned(),
        ))
    }

    /// CRTC gamma, which the legacy path owns.
    fn set_crtc_gamma(
        &self,
        _serial: u32,
        _crtc: u32,
        _red: Vec<u16>,
        _green: Vec<u16>,
        _blue: Vec<u16>,
    ) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::SetCrtcGamma)?;
        Err(fdo::Error::Failed(
            "SetCrtcGamma is unsupported: CRTC gamma is a hardware detail of the legacy path".to_owned(),
        ))
    }

    /// The colour transform matrix, which the schema does not publish.
    ///
    /// The name is spelled out because zbus derives a member's D-Bus name from the Rust
    /// one by capitalising each word, which turns `ctm` into `Ctm`; the interface's own
    /// spelling is `CTM` and a caller resolves the exact name.
    #[zbus(name = "SetOutputCTM")]
    fn set_output_ctm(
        &self,
        _serial: u32,
        _output: u32,
        _ctm: (u64, u64, u64, u64, u64, u64, u64, u64, u64),
    ) -> fdo::Result<()> {
        self.admit(DisplayConfigCall::SetOutputCTM)?;
        Err(fdo::Error::Failed(
            "SetOutputCTM is unsupported: the schema publishes no colour transform matrix".to_owned(),
        ))
    }

    /// The screen configuration changed. A client answers by reading the state again.
    #[zbus(signal)]
    async fn monitors_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// One monitor, as `GetCurrentState` reports it.
fn monitor_entry(output: &DisplayOutput) -> MonitorEntry {
    let modes = output
        .modes
        .iter()
        .map(|mode| {
            let mut properties = MonitorProperties::new();
            properties.insert("is-current".to_owned(), OwnedVariant::from(mode.id == output.mode.id));
            (
                mode.id.clone(),
                mode.width,
                mode.height,
                mode.refresh,
                output.scale,
                vec![output.scale],
                properties,
            )
        })
        .collect();
    (
        (
            output.name.clone(),
            output.make.clone(),
            output.model.clone(),
            String::new(),
        ),
        modes,
        MonitorProperties::new(),
    )
}

/// One logical monitor, as `GetCurrentState` reports it.
fn logical_entry(output: &DisplayOutput, primary: bool) -> LogicalEntry {
    (
        output.x,
        output.y,
        output.scale,
        output.transform,
        primary,
        vec![(
            output.name.clone(),
            output.make.clone(),
            output.model.clone(),
            String::new(),
        )],
        MonitorProperties::new(),
    )
}

/// Export the object and own the name.
///
/// The object is exported before the name is requested, because a caller that resolves
/// the name must find an object behind it: a name without an object is a method call
/// that never gets a reply, which is worse than a name that was never taken.
///
/// # Errors
///
/// [`DisplayError::NameTaken`] when another process owns the name, which is the
/// ordinary case while Mutter is running beside this adapter.
pub async fn export(connection: &Connection, service: &DisplayConfigService) -> Result<(), DisplayError> {
    service.take_export()?;
    connection
        .object_server()
        .at(PATH, service.clone())
        .await
        .map_err(|error| DisplayError::Bus {
            reason: error.to_string(),
        })?;
    service.attach(connection);
    connection
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await
        .map_err(|_| DisplayError::NameTaken {
            name: BUS_NAME.to_owned(),
        })?;
    gnoblin_log::info!(name = BUS_NAME, path = PATH, "serving the display configuration");
    Ok(())
}
