//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! The class split follows D13: reads answer in `portal`; selection writes and
//! enablement are `Write`; reading a selection is `Capture`.

use gnoblin_compat_core::{Adapter, Call, Gate, OperationClass, Refusal, Rule, SupportMatrix, Upstream};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-clipboard";

/// The upstream interface this adapter adapts.
pub const INTERFACE: &str = "org.gnome.Mutter.Clipboard";

/// The upstream interface version the vendored XML describes.
pub const VERSION: u32 = 1;

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member a caller can invoke.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipboardCall {
    /// `Enable`.
    Enable,
    /// `Disable`.
    Disable,
    /// `SetSelection`.
    SetSelection,
    /// `SelectionWrite`.
    SelectionWrite,
    /// `SelectionWriteDone`.
    SelectionWriteDone,
    /// `SelectionRead`.
    SelectionRead,
}

impl Call for ClipboardCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            ClipboardCall::Enable => "Enable",
            ClipboardCall::Disable => "Disable",
            ClipboardCall::SetSelection => "SetSelection",
            ClipboardCall::SelectionWrite => "SelectionWrite",
            ClipboardCall::SelectionWriteDone => "SelectionWriteDone",
            ClipboardCall::SelectionRead => "SelectionRead",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            // Reading a selection moves clipboard contents to a caller: a capture.
            ClipboardCall::SelectionRead => OperationClass::Capture,
            ClipboardCall::Enable
            | ClipboardCall::Disable
            | ClipboardCall::SetSelection
            | ClipboardCall::SelectionWrite
            | ClipboardCall::SelectionWriteDone => OperationClass::Write,
        }
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct ClipboardAdmission {
    matrix: SupportMatrix,
}

impl ClipboardAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the compiled-in manifest is unreadable.
    pub fn load() -> Result<ClipboardAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| ClipboardAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's (`unsupported`).
    pub fn admit(&self, call: ClipboardCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }

    /// The declaration itself.
    pub fn matrix(&self) -> &SupportMatrix {
        &self.matrix
    }
}

impl Adapter for ClipboardAdmission {
    type Call = ClipboardCall;
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
    fn handle(&self, call: ClipboardCall) -> Result<(), Refusal> {
        Err(Refusal::new(
            NAME,
            call.operation(),
            Rule::Adapter,
            format!("`{}` is declared served, but nothing maps it yet", call.member()),
        ))
    }
}
