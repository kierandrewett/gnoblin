//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! Every member of this adapter is a [`OperationClass::Capture`], and that one fact is the whole
//! of its policy. D13's default `portal` mode refuses capture with `capture-portal-only`,
//! because capture after consent is the Gnoblin portal's job and not a session-bus method's; the
//! lock embargo refuses it in every mode, so an `unmediated` session still cannot screenshot the
//! lock screen; and the matrix declares every member `unsupported` because the IPC publishes no
//! image-capture member at all. The three refusals are therefore distinguishable and each names
//! itself, which is what a caller needs in order to know whether to ask the portal instead.
//!
//! The gate's precedence is what decides which of the three a caller sees: mode first, then the
//! sandbox rule, then the lock. A `portal`-mode call is refused for the mode even though the
//! matrix would also refuse it, and a locked `unmediated` call is refused for the lock even
//! though the matrix would also refuse it — both are tested.

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::service::{BUS_NAME, INTERFACE, VERSION};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "gnome-shell-screenshot";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member the matrix declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenshotCall {
    /// The whole screen, saved to a file.
    Screenshot,
    /// A rectangle, saved to a file.
    ScreenshotArea,
    /// The focused window, saved to a file.
    ScreenshotWindow,
    /// An interactive selection, answered with its rectangle.
    SelectArea,
    /// A flash over a rectangle.
    FlashArea,
    /// The shell's interactive screenshot dialog.
    InteractiveScreenshot,
    /// A colour under the pointer.
    PickColor,
}

impl gnoblin_compat_core::Call for ScreenshotCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            ScreenshotCall::Screenshot => "Screenshot",
            ScreenshotCall::ScreenshotArea => "ScreenshotArea",
            ScreenshotCall::ScreenshotWindow => "ScreenshotWindow",
            ScreenshotCall::SelectArea => "SelectArea",
            ScreenshotCall::FlashArea => "FlashArea",
            ScreenshotCall::InteractiveScreenshot => "InteractiveScreenshot",
            ScreenshotCall::PickColor => "PickColor",
        }
    }

    fn operation(&self) -> OperationClass {
        // Every one of these reads pixels off the desktop, so every one is capture: a caller
        // cannot ask for a flash or a colour pick and be screened as a read.
        OperationClass::Capture
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct ScreenshotAdmission {
    matrix: SupportMatrix,
}

impl ScreenshotAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is not one the frame
    /// can read.
    pub fn load() -> Result<ScreenshotAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| ScreenshotAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's.
    pub fn admit(&self, call: ScreenshotCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }
}

impl Adapter for ScreenshotAdmission {
    type Call = ScreenshotCall;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        // The interface carries no version member, so this is the one the name has always meant.
        Upstream::new(INTERFACE, VERSION)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: every member is refused by the gate or the matrix, and a `handle` that could do
    /// something would be the second capture path this adapter exists to avoid.
    fn handle(&self, _call: ScreenshotCall) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The gate the adapter uses when the composition root did not choose one.
///
/// `Mode::Portal`, an unsandboxed same-user caller and an unlocked session: the defaults of
/// D13 and of a session that is not locked.
pub fn default_gate() -> Gate {
    Gate::new(Mode::default(), Default::default(), Default::default())
}

/// The bus name this adapter serves, which is also the name it steals from GNOME Shell.
pub const NAMES: [&str; 1] = [BUS_NAME];
