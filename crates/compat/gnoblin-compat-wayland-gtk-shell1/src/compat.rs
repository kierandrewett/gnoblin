//! Admission: the support matrix, the mode gate and the protocol's operation classes.
//!
//! This is the frame from `gnoblin-compat-core`, applied to a Wayland redirect. The
//! checked-in `support-matrix.json` is the declaration `cargo xtask compat-report`
//! prints and the one this build honours, and every request goes through
//! [`Admission::admit`] before the mapping runs, so a session that turned the adapters
//! off answers nothing at all.
//!
//! # Why every request here is a [`OperationClass::Read`]
//!
//! D13's gate exists so a *remote* caller cannot change the session: a D-Bus name
//! lets any same-user process ask the compositor to do something on its behalf, and
//! `portal` mode refuses those state changes until a user enables them. A Wayland
//! request is the opposite case. The client already owns the surface it names — the
//! compositor gave it the object — so "make my own window modal" or "maximise my own
//! window" is not authority a caller acquires, and refusing it in the default mode
//! would break GTK's titlebars on every stock install, which is the one thing the
//! plan says this redirect is for ("`gtk_shell1` matters most for daily use").
//!
//! The class is therefore `read`, and the switch that turns this adapter off is
//! `gnoblin.configure.compat` naming the target — which is [`crate::Admission::off`],
//! not a weaker class. Nothing here captures, injects input or reads another client's
//! data, so the classes the gate's other rules screen never appear.

use gnoblin_compat_core::{Adapter, Gate, MatrixError, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "gtk-shell1";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// The upstream protocol these calls belong to.
pub const SHELL: &str = "gtk_shell1";

/// The per-surface half of the protocol.
pub const SURFACE: &str = "gtk_surface1";

/// One request, one variant per member the matrix declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// `gtk_shell1.get_gtk_surface`.
    GetGtkSurface,
    /// `gtk_shell1.set_startup_id`.
    SetStartupId,
    /// `gtk_shell1.system_bell`.
    SystemBell,
    /// `gtk_shell1.notify_launch`.
    NotifyLaunch,
    /// `gtk_surface1.set_dbus_properties`.
    SetDbusProperties,
    /// `gtk_surface1.set_modal`.
    SetModal,
    /// `gtk_surface1.unset_modal`.
    UnsetModal,
    /// `gtk_surface1.present`.
    Present,
    /// `gtk_surface1.request_focus`.
    RequestFocus,
    /// `gtk_surface1.release`.
    Release,
    /// `gtk_surface1.titlebar_gesture`.
    TitlebarGesture,
    /// `gtk_surface1.set_a11y_properties`.
    SetA11yProperties,
}

impl gnoblin_compat_core::Call for Call {
    fn interface(&self) -> &'static str {
        match self {
            Self::GetGtkSurface | Self::SetStartupId | Self::SystemBell | Self::NotifyLaunch => SHELL,
            _ => SURFACE,
        }
    }

    fn member(&self) -> &'static str {
        match self {
            Self::GetGtkSurface => "get_gtk_surface",
            Self::SetStartupId => "set_startup_id",
            Self::SystemBell => "system_bell",
            Self::NotifyLaunch => "notify_launch",
            Self::SetDbusProperties => "set_dbus_properties",
            Self::SetModal => "set_modal",
            Self::UnsetModal => "unset_modal",
            Self::Present => "present",
            Self::RequestFocus => "request_focus",
            Self::Release => "release",
            Self::TitlebarGesture => "titlebar_gesture",
            Self::SetA11yProperties => "set_a11y_properties",
        }
    }

    fn operation(&self) -> OperationClass {
        OperationClass::Read
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct Admission {
    matrix: SupportMatrix,
    mode: Mode,
}

impl Admission {
    /// Load the checked-in declaration for one configured mode.
    ///
    /// A manifest the frame cannot read is a build-time bug, not a run-time
    /// condition: the bytes are compiled in and the frame's own tests pin the format,
    /// so the caller of this function is a `main` that reports it and exits.
    ///
    /// # Errors
    ///
    /// [`MatrixError`] when the manifest is not one the frame can honour.
    pub fn load(mode: Mode) -> Result<Self, MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| Self { matrix, mode })
    }

    /// An admission whose answer is "off" for every call, without loading anything.
    ///
    /// What a session uses when `gnoblin.configure.compat` turns the targets off: the
    /// declarations stay in the build, and the mode is what makes them answer nothing.
    #[must_use]
    pub fn off() -> Self {
        Self {
            matrix: SupportMatrix::new(),
            mode: Mode::Off,
        }
    }

    /// The mode this admission screens with.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The frame's [`Refusal`] when the mode refuses the call or the matrix does not
    /// declare it. A [`gnoblin_compat_core::Rule::Undeclared`] refusal for a mode that
    /// is on means the manifest is missing a member, which is a declaration bug the
    /// frame reports rather than a call that quietly succeeds.
    pub fn admit(&self, call: Call, gate: Gate) -> Result<(), Refusal> {
        // The lock is not consulted: every call here is a read, and the embargo's only
        // rule is over capture. A session that is locking still lets a client present
        // its own window — the activation authority is what refuses that while covered.
        Adapter::serve(&Screening { matrix: &self.matrix }, call, gate)
    }
}

/// The frame's own trait, implemented for the admission so `serve` can gate it.
///
/// It exists because [`gnoblin_compat_core::Adapter::serve`] is the one entry point
/// that cannot skip a refusal, and this adapter's mapping is not a D-Bus handler: the
/// answer is `()`, the transport does the work, and what the frame contributes is the
/// gate and the declaration.
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
        Upstream::new(SHELL, crate::SHELL_VERSION)
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
    use gnoblin_compat_core::{Call as _, CallerContext, Decision, LockState, allow};

    fn gate(mode: Mode) -> Gate {
        Gate::new(mode, CallerContext::UNSANDBOXED, LockState::Unlocked)
    }

    /// Every protocol member is declared, so no request is ever refused as undeclared:
    /// what the matrix says about it decides, which is the difference between "nobody
    /// promised this" and "this is not served here".
    #[test]
    fn every_protocol_member_is_declared() {
        let admission = Admission::load(Mode::Portal).expect("the checked-in manifest");
        let mut refused = Vec::new();
        for call in [
            Call::GetGtkSurface,
            Call::SetStartupId,
            Call::SystemBell,
            Call::NotifyLaunch,
            Call::SetDbusProperties,
            Call::SetModal,
            Call::UnsetModal,
            Call::Present,
            Call::RequestFocus,
            Call::Release,
            Call::TitlebarGesture,
            Call::SetA11yProperties,
        ] {
            match admission.admit(call, gate(Mode::Portal)) {
                Ok(()) => {}
                Err(refusal) => {
                    assert_ne!(
                        refusal.rule(),
                        gnoblin_compat_core::Rule::Undeclared,
                        "{}.{} must be declared",
                        gnoblin_compat_core::Call::interface(&call),
                        gnoblin_compat_core::Call::member(&call)
                    );
                    refused.push(refusal.rule());
                }
            }
        }
        assert_eq!(
            refused,
            vec![gnoblin_compat_core::Rule::Unsupported],
            "exactly the one member the matrix declares unsupported is refused, and for that reason"
        );
    }

    /// A session that turned the adapter off answers nothing, and the refusal names
    /// the mode rather than the member.
    #[test]
    fn the_mode_gate_refuses_a_disabled_adapter() {
        let admission = Admission::off();
        let refusal = admission
            .admit(Call::SystemBell, gate(Mode::Off))
            .expect_err("an adapter that is off answers nothing");
        assert_eq!(refusal.rule(), gnoblin_compat_core::Rule::ModeOff);
    }

    /// The default mode serves every declared member, which is what makes GTK's
    /// titlebars work on a stock install.
    #[test]
    fn the_default_mode_serves_the_redirect() {
        for call in [Call::Present, Call::TitlebarGesture, Call::SetModal] {
            assert_eq!(
                allow(
                    NAME,
                    call.operation(),
                    CallerContext::UNSANDBOXED,
                    Mode::Portal,
                    LockState::Locked
                ),
                Decision::Allow,
                "a read is served in `portal` mode even while locking"
            );
        }
    }
}
