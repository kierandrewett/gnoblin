//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! This is the whole of what the frame asks of an adapter: declare fidelity per
//! member, and send every call through the gate before it reaches the mapping. The
//! declaration is [`MATRIX`], the checked-in `support-matrix.json` that
//! `cargo xtask compat-report` prints, compiled in so the file a reviewer approved is
//! the file this build honours.
//!
//! One member is a write and the rest are reads, and that split is the whole of D13
//! for this adapter: `GetCurrentState` and the properties answer in `portal` mode, and
//! `ApplyMonitorsConfig` is refused with [`gnoblin_compat_core::Rule::WriteNotEnabled`]
//! until a user enables state changes. The plan's redirect map states it as "reads on;
//! apply off".

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::service::{BUS_NAME, INTERFACE, VERSION};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-displayconfig";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member the matrix declares.
///
/// The class comes from here and not from the transport, so a caller cannot ask for a
/// read where it is making a write.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplayConfigCall {
    /// The whole display state.
    GetCurrentState,
    /// A new configuration.
    ApplyMonitorsConfig,
    /// Whether a caller may apply a configuration.
    ApplyMonitorsConfigAllowed,
    /// Whether this compositor rotates the panel on its own.
    PanelOrientationManaged,
    /// Whether Night Light is available.
    NightLightSupported,
    /// The legacy resource list.
    GetResources,
    /// The legacy configuration call.
    ApplyConfiguration,
    /// The legacy backlight property.
    Backlight,
    /// The legacy backlight call.
    ChangeBacklight,
    /// The legacy backlight call.
    SetBacklight,
    /// Display power saving.
    PowerSaveMode,
    /// Whether an external monitor is active.
    HasExternalMonitor,
    /// CRTC gamma.
    GetCrtcGamma,
    /// CRTC gamma.
    SetCrtcGamma,
    /// The colour transform matrix.
    SetOutputCTM,
}

impl gnoblin_compat_core::Call for DisplayConfigCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            DisplayConfigCall::GetCurrentState => "GetCurrentState",
            DisplayConfigCall::ApplyMonitorsConfig => "ApplyMonitorsConfig",
            DisplayConfigCall::ApplyMonitorsConfigAllowed => "ApplyMonitorsConfigAllowed",
            DisplayConfigCall::PanelOrientationManaged => "PanelOrientationManaged",
            DisplayConfigCall::NightLightSupported => "NightLightSupported",
            DisplayConfigCall::GetResources => "GetResources",
            DisplayConfigCall::ApplyConfiguration => "ApplyConfiguration",
            DisplayConfigCall::Backlight => "Backlight",
            DisplayConfigCall::ChangeBacklight => "ChangeBacklight",
            DisplayConfigCall::SetBacklight => "SetBacklight",
            DisplayConfigCall::PowerSaveMode => "PowerSaveMode",
            DisplayConfigCall::HasExternalMonitor => "HasExternalMonitor",
            DisplayConfigCall::GetCrtcGamma => "GetCrtcGamma",
            DisplayConfigCall::SetCrtcGamma => "SetCrtcGamma",
            DisplayConfigCall::SetOutputCTM => "SetOutputCTM",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            // The one write. Everything else reads state, and the legacy members read
            // or write hardware this adapter does not serve: they are refused by the
            // matrix, so their class only has to be the honest one.
            DisplayConfigCall::ApplyMonitorsConfig
            | DisplayConfigCall::ApplyConfiguration
            | DisplayConfigCall::ChangeBacklight
            | DisplayConfigCall::SetBacklight
            | DisplayConfigCall::PowerSaveMode
            | DisplayConfigCall::SetCrtcGamma
            | DisplayConfigCall::SetOutputCTM => OperationClass::Write,
            DisplayConfigCall::GetCurrentState
            | DisplayConfigCall::ApplyMonitorsConfigAllowed
            | DisplayConfigCall::PanelOrientationManaged
            | DisplayConfigCall::NightLightSupported
            | DisplayConfigCall::GetResources
            | DisplayConfigCall::Backlight
            | DisplayConfigCall::HasExternalMonitor
            | DisplayConfigCall::GetCrtcGamma => OperationClass::Read,
        }
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct DisplayConfigAdmission {
    matrix: SupportMatrix,
}

impl DisplayConfigAdmission {
    /// Load the checked-in declaration.
    ///
    /// A manifest the frame cannot read is a build-time bug, not a run-time condition:
    /// the bytes are compiled in and the frame's own tests pin the format, so the
    /// caller of this function is a `main` that reports it and exits.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is not one the
    /// frame can read.
    pub fn load() -> Result<DisplayConfigAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| DisplayConfigAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's (`unsupported`,
    /// `undeclared`).
    pub fn admit(&self, call: DisplayConfigCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }
}

impl Adapter for DisplayConfigAdmission {
    type Call = DisplayConfigCall;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        // Mutter 51.0's `DisplayConfig` is version 3, which is the version this
        // declaration was written against and the one the workspace's own frame
        // fixture names.
        Upstream::new(INTERFACE, VERSION)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: what a call *does* needs the graph and the bus, which no `handle` can
    /// express, and returning `()` keeps the fuse honest — there is no second place
    /// where a refusal could be skipped.
    fn handle(&self, _call: DisplayConfigCall) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The gate the adapter uses when the composition root did not choose one.
///
/// `Mode::Portal`, an unsandboxed same-user caller and an unlocked session: the
/// defaults of D13 and of a session that is not locked. A caller context is
/// `unsandboxed` because a D-Bus caller on the session bus is a same-user process and
/// the frame's sandbox rule is about Flatpak and security contexts, which a bus name
/// cannot prove either way.
pub fn default_gate() -> Gate {
    Gate::new(Mode::default(), Default::default(), Default::default())
}

/// The bus name this adapter serves, which is also the name it steals from Mutter.
pub const NAMES: [&str; 1] = [BUS_NAME];
