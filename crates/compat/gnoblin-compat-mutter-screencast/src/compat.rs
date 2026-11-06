//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! This is the whole of what the frame asks of an adapter: declare fidelity per
//! member, and send every call through the gate before it reaches the mapping. The
//! declaration is [`MATRIX`], the checked-in `support-matrix.json` that
//! `cargo xtask compat-report` prints, compiled in so the file a reviewer approved is
//! the file this build honours.
//!
//! The class split is D13: `Version` is a read, and everything that opens, records or
//! starts a capture is [`OperationClass::Capture`]. In `portal` mode the read answers
//! and every capture member is refused with `capture-portal-only`, before the matrix
//! or the adapter sees it.

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::service::{INTERFACE, SESSION_INTERFACE, STREAM_INTERFACE, VERSION};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "mutter-screencast";

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member a caller can invoke.
///
/// Signals have no variant: an adapter emits them, and a caller cannot call one, so
/// their declaration lives in the matrix alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenCastCall {
    /// `CreateSession`.
    CreateSession,
    /// The `Version` property.
    Version,
    /// `Session.Start`.
    Start,
    /// `Session.Stop`.
    Stop,
    /// `Session.RecordMonitor`.
    RecordMonitor,
    /// `Session.RecordWindow`.
    RecordWindow,
    /// `Session.RecordArea`.
    RecordArea,
    /// `Session.RecordVirtual`.
    RecordVirtual,
    /// `Stream.Start`.
    StreamStart,
    /// `Stream.Stop`.
    StreamStop,
    /// The `Stream.Parameters` property.
    StreamParameters,
}

impl gnoblin_compat_core::Call for ScreenCastCall {
    fn interface(&self) -> &'static str {
        match self {
            ScreenCastCall::CreateSession | ScreenCastCall::Version => INTERFACE,
            ScreenCastCall::Start
            | ScreenCastCall::Stop
            | ScreenCastCall::RecordMonitor
            | ScreenCastCall::RecordWindow
            | ScreenCastCall::RecordArea
            | ScreenCastCall::RecordVirtual => SESSION_INTERFACE,
            ScreenCastCall::StreamStart | ScreenCastCall::StreamStop | ScreenCastCall::StreamParameters => {
                STREAM_INTERFACE
            }
        }
    }

    fn member(&self) -> &'static str {
        match self {
            ScreenCastCall::CreateSession => "CreateSession",
            ScreenCastCall::Version => "Version",
            ScreenCastCall::Start => "Start",
            ScreenCastCall::Stop => "Stop",
            ScreenCastCall::RecordMonitor => "RecordMonitor",
            ScreenCastCall::RecordWindow => "RecordWindow",
            ScreenCastCall::RecordArea => "RecordArea",
            ScreenCastCall::RecordVirtual => "RecordVirtual",
            ScreenCastCall::StreamStart => "Start",
            ScreenCastCall::StreamStop => "Stop",
            ScreenCastCall::StreamParameters => "Parameters",
        }
    }

    fn operation(&self) -> OperationClass {
        match self {
            // The version names no pixels, so it is a read and answers in `portal`.
            ScreenCastCall::Version => OperationClass::Read,
            // Everything else asks the compositor to capture, or asks what it captured.
            ScreenCastCall::CreateSession
            | ScreenCastCall::Start
            | ScreenCastCall::Stop
            | ScreenCastCall::RecordMonitor
            | ScreenCastCall::RecordWindow
            | ScreenCastCall::RecordArea
            | ScreenCastCall::RecordVirtual
            | ScreenCastCall::StreamStart
            | ScreenCastCall::StreamStop
            | ScreenCastCall::StreamParameters => OperationClass::Capture,
        }
    }
}

/// The declaration and the gate, as one admission check.
#[derive(Debug)]
pub struct ScreenCastAdmission {
    matrix: SupportMatrix,
}

impl ScreenCastAdmission {
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
    pub fn load() -> Result<ScreenCastAdmission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| ScreenCastAdmission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    ///
    /// # Errors
    ///
    /// The gate's refusal (mode, sandbox, lock) or the matrix's (`unsupported`,
    /// `undeclared`).
    pub fn admit(&self, call: ScreenCastCall, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }

    /// The declaration itself, so a refusal can quote the matrix verbatim.
    pub fn matrix(&self) -> &SupportMatrix {
        &self.matrix
    }
}

impl Adapter for ScreenCastAdmission {
    type Call = ScreenCastCall;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        // Mutter 51.0's ScreenCast interface is version 4: `is-recording` arrived in
        // version 4 and `RecordVirtual` in version 3, which is what the declaration
        // names.
        Upstream::new(INTERFACE, VERSION)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: what a call *does* needs the bus and the compositor, which no `handle`
    /// can express, and returning `()` keeps the fuse honest — there is no second place
    /// where a refusal could be skipped.
    fn handle(&self, _call: ScreenCastCall) -> Result<(), Refusal> {
        Ok(())
    }
}

/// The gate the adapter uses when the composition root did not choose one.
///
/// `Mode::Portal`, an unsandboxed same-user caller and an unlocked session: the
/// defaults of D13 and of a session that is not locked.
pub fn default_gate() -> Gate {
    Gate::new(Mode::default(), Default::default(), Default::default())
}
