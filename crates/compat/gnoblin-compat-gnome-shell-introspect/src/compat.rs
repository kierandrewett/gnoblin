//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! Every member of this adapter is a [`OperationClass::Read`]: it reports what the desktop
//! holds and changes nothing. That is why the redirect map files it as "adapter, on" rather
//! than under the capture setting — the portal's window picker must be able to ask which
//! windows exist while the session is in D13's default `portal` mode, and the only member
//! that would not be a read, `GetRunningApplications`, is declared `unsupported` because the
//! schema publishes no application object.

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::service::{BUS_NAME, INTERFACE, VERSION};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "gnome-shell-introspect";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member the matrix declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IntrospectCall {
    /// The window list.
    GetWindows,
    /// The application list, which this adapter does not serve.
    GetRunningApplications,
    /// Whether the shell's animations are enabled.
    AnimationsEnabled,
    /// The screen's size.
    ScreenSize,
    /// The interface version.
    Version,
}

impl gnoblin_compat_core::Call for IntrospectCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            IntrospectCall::GetWindows => "GetWindows",
            IntrospectCall::GetRunningApplications => "GetRunningApplications",
            IntrospectCall::AnimationsEnabled => "AnimationsEnabled",
            IntrospectCall::ScreenSize => "ScreenSize",
            IntrospectCall::Version => "version",
        }
    }

    fn operation(&self) -> OperationClass {
        // Nothing here changes the session, so the mode gate never has a write to refuse and
        // the lock embargo never has a capture to refuse: introspection answers a locked
        // session's own lock screen, which is what a picker behind it needs.
        OperationClass::Read
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct IntrospectAdmission {
    matrix: SupportMatrix,
}

impl IntrospectAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is not one the frame
    /// can read.
    pub fn load() -> Result<IntrospectAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| IntrospectAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal or the matrix's.
    pub fn admit(&self, call: IntrospectCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }
}

impl Adapter for IntrospectAdmission {
    type Call = IntrospectCall;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        // `Introspect`'s own `version` property is 3 in gnome-shell 51.0, which is the
        // release the workspace's version pin names.
        Upstream::new(INTERFACE, VERSION)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: what a call *does* needs the graph and the bus, which no `handle` can
    /// express, and returning `()` keeps the fuse honest.
    fn handle(&self, _call: IntrospectCall) -> Result<(), Refusal> {
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
