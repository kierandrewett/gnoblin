//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! The class split follows D13. Every call names its operation class in
//! [`Call::operation`], so the gate cannot be bypassed by a weaker class.

use gnoblin_compat_core::{Adapter, Call, Gate, OperationClass, Refusal, Rule, SupportMatrix, Upstream};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-inputcapture";

/// The root upstream interface this adapter adapts.
pub const INTERFACE: &str = "org.gnome.Mutter.InputCapture";

/// The session interface this adapter adapts.
pub const SESSION_INTERFACE: &str = "org.gnome.Mutter.InputCapture.Session";

/// The upstream interface version the vendored XML describes.
pub const VERSION: u32 = 1;

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per method or property a caller can invoke. Signals are not
/// calls and are declared in the matrix only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputCaptureCall {
    /// `CreateSession`.
    CreateSession,
    /// `SupportedCapabilities`.
    SupportedCapabilities,
    /// `GetZones`.
    GetZones,
    /// `AddBarrier`.
    AddBarrier,
    /// `ClearBarriers`.
    ClearBarriers,
    /// `Enable`.
    Enable,
    /// `Disable`.
    Disable,
    /// `ConnectToEIS`.
    ConnectToEIS,
    /// `Release`.
    Release,
    /// `Close`.
    Close,
}

const ROOT: &str = INTERFACE;
const SESSION: &str = SESSION_INTERFACE;

impl Call for InputCaptureCall {
    fn interface(&self) -> &'static str {
        match self {
            Self::CreateSession => ROOT,
            Self::SupportedCapabilities => ROOT,
            Self::GetZones => SESSION,
            Self::AddBarrier => SESSION,
            Self::ClearBarriers => SESSION,
            Self::Enable => SESSION,
            Self::Disable => SESSION,
            Self::ConnectToEIS => SESSION,
            Self::Release => SESSION,
            Self::Close => SESSION,
        }
    }

    fn member(&self) -> &'static str {
        match self {
            Self::CreateSession => "CreateSession",
            Self::SupportedCapabilities => "SupportedCapabilities",
            Self::GetZones => "GetZones",
            Self::AddBarrier => "AddBarrier",
            Self::ClearBarriers => "ClearBarriers",
            Self::Enable => "Enable",
            Self::Disable => "Disable",
            Self::ConnectToEIS => "ConnectToEIS",
            Self::Release => "Release",
            Self::Close => "Close",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            Self::CreateSession => OperationClass::Write,
            Self::SupportedCapabilities => OperationClass::Read,
            Self::GetZones => OperationClass::Read,
            Self::AddBarrier => OperationClass::Write,
            Self::ClearBarriers => OperationClass::Write,
            Self::Enable => OperationClass::Write,
            Self::Disable => OperationClass::Write,
            Self::ConnectToEIS => OperationClass::InputInjection,
            Self::Release => OperationClass::Write,
            Self::Close => OperationClass::Write,
        }
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct InputCaptureAdmission {
    matrix: SupportMatrix,
}

impl InputCaptureAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the compiled-in manifest is unreadable.
    pub fn load() -> Result<InputCaptureAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| InputCaptureAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's (`unsupported`).
    pub fn admit(&self, call: InputCaptureCall, gate: Gate) -> Result<(), Refusal> {
        // The lock embargo covers input as well as capture (`design/rust-compositor.md`,
        // Lock: "input, capture, clipboard and drag embargo"). `gnoblin-compat-core` embargoes
        // only capture, so the input half lives here until core learns it.
        if call.operation() == OperationClass::InputInjection && gate.lock().is_engaged() {
            return Err(Refusal::new(
                NAME,
                call.operation(),
                Rule::LockedCapture,
                "input injection is refused while the session is locking or locked",
            )
            .at(call.interface(), call.member()));
        }
        self.serve(call, gate)
    }

    /// The declaration itself.
    pub fn matrix(&self) -> &SupportMatrix {
        &self.matrix
    }
}

impl Adapter for InputCaptureAdmission {
    type Call = InputCaptureCall;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        Upstream::new(INTERFACE, VERSION)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// Nothing is mapped yet: every member is declared `unsupported`, so `serve` never
    /// reaches this for a call the matrix allows. It refuses rather than answering, so a
    /// future edit to the matrix alone cannot produce a fake success.
    fn handle(&self, call: InputCaptureCall) -> Result<(), Refusal> {
        Err(Refusal::new(
            NAME,
            call.operation(),
            Rule::Adapter,
            format!("`{}` is declared served, but nothing maps it yet", call.member()),
        ))
    }
}
