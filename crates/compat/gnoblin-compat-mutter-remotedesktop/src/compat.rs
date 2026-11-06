//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! The class split follows D13. Every call names its operation class in
//! [`Call::operation`], so the gate cannot be bypassed by a weaker class.

use gnoblin_compat_core::{Adapter, Call, Gate, OperationClass, Refusal, Rule, SupportMatrix, Upstream};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-remotedesktop";

/// The root upstream interface this adapter adapts.
pub const INTERFACE: &str = "org.gnome.Mutter.RemoteDesktop";

/// The session interface this adapter adapts.
pub const SESSION_INTERFACE: &str = "org.gnome.Mutter.RemoteDesktop.Session";

/// The upstream interface version the vendored XML describes.
pub const VERSION: u32 = 1;

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per method or property a caller can invoke. Signals are not
/// calls and are declared in the matrix only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RemoteDesktopCall {
    /// `CreateSession`.
    CreateSession,
    /// `SupportedDeviceTypes`.
    SupportedDeviceTypes,
    /// `Version`.
    Version,
    /// `SessionId`.
    SessionId,
    /// `Start`.
    Start,
    /// `Stop`.
    Stop,
    /// `NotifyKeyboardKeycode`.
    NotifyKeyboardKeycode,
    /// `NotifyKeyboardKeysym`.
    NotifyKeyboardKeysym,
    /// `NotifyPointerButton`.
    NotifyPointerButton,
    /// `NotifyPointerAxis`.
    NotifyPointerAxis,
    /// `NotifyPointerAxisDiscrete`.
    NotifyPointerAxisDiscrete,
    /// `NotifyPointerMotionRelative`.
    NotifyPointerMotionRelative,
    /// `NotifyPointerMotionAbsolute`.
    NotifyPointerMotionAbsolute,
    /// `NotifyTouchDown`.
    NotifyTouchDown,
    /// `NotifyTouchMotion`.
    NotifyTouchMotion,
    /// `NotifyTouchUp`.
    NotifyTouchUp,
    /// `EnableClipboard`.
    EnableClipboard,
    /// `DisableClipboard`.
    DisableClipboard,
    /// `SetSelection`.
    SetSelection,
    /// `SelectionWrite`.
    SelectionWrite,
    /// `SelectionWriteDone`.
    SelectionWriteDone,
    /// `SelectionRead`.
    SelectionRead,
    /// `CapsLockState`.
    CapsLockState,
    /// `NumLockState`.
    NumLockState,
    /// `ConnectToEIS`.
    ConnectToEIS,
    /// `KeymapCapabilities`.
    KeymapCapabilities,
    /// `SetKeymap`.
    SetKeymap,
    /// `SetKeymapLayoutIndex`.
    SetKeymapLayoutIndex,
    /// `CurrentKeymap`.
    CurrentKeymap,
}

const ROOT: &str = INTERFACE;
const SESSION: &str = SESSION_INTERFACE;

impl Call for RemoteDesktopCall {
    fn interface(&self) -> &'static str {
        match self {
            Self::CreateSession => ROOT,
            Self::SupportedDeviceTypes => ROOT,
            Self::Version => ROOT,
            Self::SessionId => SESSION,
            Self::Start => SESSION,
            Self::Stop => SESSION,
            Self::NotifyKeyboardKeycode => SESSION,
            Self::NotifyKeyboardKeysym => SESSION,
            Self::NotifyPointerButton => SESSION,
            Self::NotifyPointerAxis => SESSION,
            Self::NotifyPointerAxisDiscrete => SESSION,
            Self::NotifyPointerMotionRelative => SESSION,
            Self::NotifyPointerMotionAbsolute => SESSION,
            Self::NotifyTouchDown => SESSION,
            Self::NotifyTouchMotion => SESSION,
            Self::NotifyTouchUp => SESSION,
            Self::EnableClipboard => SESSION,
            Self::DisableClipboard => SESSION,
            Self::SetSelection => SESSION,
            Self::SelectionWrite => SESSION,
            Self::SelectionWriteDone => SESSION,
            Self::SelectionRead => SESSION,
            Self::CapsLockState => SESSION,
            Self::NumLockState => SESSION,
            Self::ConnectToEIS => SESSION,
            Self::KeymapCapabilities => SESSION,
            Self::SetKeymap => SESSION,
            Self::SetKeymapLayoutIndex => SESSION,
            Self::CurrentKeymap => SESSION,
        }
    }

    fn member(&self) -> &'static str {
        match self {
            Self::CreateSession => "CreateSession",
            Self::SupportedDeviceTypes => "SupportedDeviceTypes",
            Self::Version => "Version",
            Self::SessionId => "SessionId",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::NotifyKeyboardKeycode => "NotifyKeyboardKeycode",
            Self::NotifyKeyboardKeysym => "NotifyKeyboardKeysym",
            Self::NotifyPointerButton => "NotifyPointerButton",
            Self::NotifyPointerAxis => "NotifyPointerAxis",
            Self::NotifyPointerAxisDiscrete => "NotifyPointerAxisDiscrete",
            Self::NotifyPointerMotionRelative => "NotifyPointerMotionRelative",
            Self::NotifyPointerMotionAbsolute => "NotifyPointerMotionAbsolute",
            Self::NotifyTouchDown => "NotifyTouchDown",
            Self::NotifyTouchMotion => "NotifyTouchMotion",
            Self::NotifyTouchUp => "NotifyTouchUp",
            Self::EnableClipboard => "EnableClipboard",
            Self::DisableClipboard => "DisableClipboard",
            Self::SetSelection => "SetSelection",
            Self::SelectionWrite => "SelectionWrite",
            Self::SelectionWriteDone => "SelectionWriteDone",
            Self::SelectionRead => "SelectionRead",
            Self::CapsLockState => "CapsLockState",
            Self::NumLockState => "NumLockState",
            Self::ConnectToEIS => "ConnectToEIS",
            Self::KeymapCapabilities => "KeymapCapabilities",
            Self::SetKeymap => "SetKeymap",
            Self::SetKeymapLayoutIndex => "SetKeymapLayoutIndex",
            Self::CurrentKeymap => "CurrentKeymap",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            Self::CreateSession => OperationClass::Write,
            Self::SupportedDeviceTypes => OperationClass::Read,
            Self::Version => OperationClass::Read,
            Self::SessionId => OperationClass::Read,
            Self::Start => OperationClass::Write,
            Self::Stop => OperationClass::Write,
            Self::NotifyKeyboardKeycode => OperationClass::InputInjection,
            Self::NotifyKeyboardKeysym => OperationClass::InputInjection,
            Self::NotifyPointerButton => OperationClass::InputInjection,
            Self::NotifyPointerAxis => OperationClass::InputInjection,
            Self::NotifyPointerAxisDiscrete => OperationClass::InputInjection,
            Self::NotifyPointerMotionRelative => OperationClass::InputInjection,
            Self::NotifyPointerMotionAbsolute => OperationClass::InputInjection,
            Self::NotifyTouchDown => OperationClass::InputInjection,
            Self::NotifyTouchMotion => OperationClass::InputInjection,
            Self::NotifyTouchUp => OperationClass::InputInjection,
            Self::EnableClipboard => OperationClass::Write,
            Self::DisableClipboard => OperationClass::Write,
            Self::SetSelection => OperationClass::Write,
            Self::SelectionWrite => OperationClass::Write,
            Self::SelectionWriteDone => OperationClass::Write,
            Self::SelectionRead => OperationClass::Capture,
            Self::CapsLockState => OperationClass::Read,
            Self::NumLockState => OperationClass::Read,
            Self::ConnectToEIS => OperationClass::InputInjection,
            Self::KeymapCapabilities => OperationClass::Read,
            Self::SetKeymap => OperationClass::Write,
            Self::SetKeymapLayoutIndex => OperationClass::Write,
            Self::CurrentKeymap => OperationClass::Read,
        }
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct RemoteDesktopAdmission {
    matrix: SupportMatrix,
}

impl RemoteDesktopAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the compiled-in manifest is unreadable.
    pub fn load() -> Result<RemoteDesktopAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| RemoteDesktopAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's (`unsupported`).
    pub fn admit(&self, call: RemoteDesktopCall, gate: Gate) -> Result<(), Refusal> {
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

impl Adapter for RemoteDesktopAdmission {
    type Call = RemoteDesktopCall;
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
    fn handle(&self, call: RemoteDesktopCall) -> Result<(), Refusal> {
        Err(Refusal::new(
            NAME,
            call.operation(),
            Rule::Adapter,
            format!("`{}` is declared served, but nothing maps it yet", call.member()),
        ))
    }
}
