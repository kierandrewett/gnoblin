//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! Every member of this adapter is a [`OperationClass::Read`]: an idle watch observes the
//! session, it does not change it. That is why the redirect map files this adapter as "on"
//! rather than under the capture setting — gsd-power and every screensaver must be able to
//! ask how long the user has been away while the session is in D13's default `portal`
//! mode. `ResetIdletime` is the one member that would change the session's state, and it is
//! declared `unsupported` rather than gated, because no caller may hold a session awake by
//! resetting its counter.

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::service::{BUS_NAME, INTERFACE, VERSION};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-idlemonitor";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member the matrix declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdleMonitorCall {
    /// How long the session has been idle.
    GetIdletime,
    /// Watch for the idle time reaching an interval.
    AddIdleWatch,
    /// Watch for the user becoming active again.
    AddUserActiveWatch,
    /// Remove a watch.
    RemoveWatch,
    /// Reset the idle counter, which this adapter does not serve.
    ResetIdletime,
}

impl gnoblin_compat_core::Call for IdleMonitorCall {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            IdleMonitorCall::GetIdletime => "GetIdletime",
            IdleMonitorCall::AddIdleWatch => "AddIdleWatch",
            IdleMonitorCall::AddUserActiveWatch => "AddUserActiveWatch",
            IdleMonitorCall::RemoveWatch => "RemoveWatch",
            IdleMonitorCall::ResetIdletime => "ResetIdletime",
        }
    }

    fn operation(&self) -> OperationClass {
        // A watch is the caller's own bookkeeping and the counter is a read: nothing here
        // changes the session, so the mode gate never has a write to refuse.
        OperationClass::Read
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct IdleMonitorAdmission {
    matrix: SupportMatrix,
}

impl IdleMonitorAdmission {
    /// Load the checked-in declaration.
    ///
    /// # Errors
    ///
    /// [`gnoblin_compat_core::MatrixError`] when the checked-in manifest is not one the
    /// frame can read.
    pub fn load() -> Result<IdleMonitorAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| IdleMonitorAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal or the matrix's.
    pub fn admit(&self, call: IdleMonitorCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }
}

impl Adapter for IdleMonitorAdmission {
    type Call = IdleMonitorCall;
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

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: what a call *does* needs the registry and the bus, which no `handle` can
    /// express, and returning `()` keeps the fuse honest.
    fn handle(&self, _call: IdleMonitorCall) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The gate the adapter uses when the composition root did not choose one.
///
/// `Mode::Portal`, an unsandboxed same-user caller and an unlocked session: the defaults
/// of D13 and of a session that is not locked.
pub fn default_gate() -> Gate {
    Gate::new(Mode::default(), Default::default(), Default::default())
}

/// The bus name this adapter serves, which is also the name it steals from Mutter.
pub const NAMES: [&str; 1] = [BUS_NAME];
