//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! The same frame `gnoblin-compat-gtk-shell1` uses, with one difference worth stating:
//! both members here are [`OperationClass::Write`], because a caller that is not the
//! window's own client is changing how that window relates to another one. The portal is
//! the caller D15 names ("a portal-built dialog"), so a session in `portal` mode refuses
//! both members until writes are enabled — which is D13's rule applied honestly rather
//! than a redirect that quietly does what it likes.
//!
//! That refusal is not an error the client can see: a Wayland request has no reply, so a
//! refused `set_x11_parent` records nothing, and the dialog appears without a parent —
//! exactly what it did before this adapter existed.

use gnoblin_compat_core::{Adapter, Gate, MatrixError, OperationClass, Refusal, SupportMatrix, Upstream};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-x11-interop";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// The interface the protocol declares.
pub const INTERFACE: &str = "mutter_x11_interop";

/// One request, one variant per member the matrix declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// `mutter_x11_interop.destroy`.
    Destroy,
    /// `mutter_x11_interop.set_x11_parent`.
    SetX11Parent,
}

impl gnoblin_compat_core::Call for Call {
    fn interface(&self) -> &'static str {
        INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            Self::Destroy => "destroy",
            Self::SetX11Parent => "set_x11_parent",
        }
    }

    fn operation(&self) -> OperationClass {
        // `destroy` changes nothing this adapter owns once the object is gone, but it is
        // the same adapter's surface and the same class keeps the gate's answer one
        // answer per adapter.
        OperationClass::Write
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct Admission {
    matrix: SupportMatrix,
    gate: Gate,
}

impl Admission {
    /// Load the checked-in declaration for one gate.
    ///
    /// # Errors
    ///
    /// [`MatrixError`] when the manifest is not one the frame can honour, which is a
    /// build-time bug: the bytes are compiled in and the frame's tests pin the format.
    pub fn load(gate: Gate) -> Result<Self, MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| Self { matrix, gate })
    }

    /// The gate every request is screened with.
    #[must_use]
    pub const fn gate(&self) -> Gate {
        self.gate
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The frame's [`Refusal`] when the gate refuses the call or the matrix does not
    /// declare it.
    pub fn admit(&self, call: Call) -> Result<(), Refusal> {
        Adapter::serve(&Screening { matrix: &self.matrix }, call, self.gate)
    }
}

/// The frame's own trait: the gate and the declaration, and no mapping.
struct Screening<'a> {
    matrix: &'a SupportMatrix,
}

impl Adapter for Screening<'_> {
    type Call = Call;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        Upstream::new(INTERFACE, 1)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        self.matrix
    }

    fn handle(&self, _call: Self::Call) -> Result<(), Refusal> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_compat_core::{CallerContext, LockState, Mode};

    fn gate(mode: Mode) -> Gate {
        Gate::new(mode, CallerContext::UNSANDBOXED, LockState::Unlocked)
    }

    /// Both members are declared, so the only refusals are the gate's.
    #[test]
    fn both_members_are_declared() {
        let admission = Admission::load(gate(Mode::Unmediated)).expect("the checked-in manifest");
        assert!(admission.admit(Call::Destroy).is_ok());
        assert!(admission.admit(Call::SetX11Parent).is_ok());
    }

    /// In the default mode a write is refused, which is D13's rule: the portal's request
    /// changes how two windows relate, and a session that has not enabled writes answers
    /// nothing.
    #[test]
    fn the_default_mode_refuses_the_write() {
        let admission = Admission::load(gate(Mode::Portal)).expect("the checked-in manifest");
        let refusal = admission
            .admit(Call::SetX11Parent)
            .expect_err("a write is off by default");
        assert_eq!(refusal.rule(), gnoblin_compat_core::Rule::WriteNotEnabled);
    }
}
