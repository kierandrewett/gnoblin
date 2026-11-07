//! Admission: the support matrix and the mode gate, from `gnoblin-compat-core`.
//!
//! This is the whole of what the frame asks of an adapter: declare fidelity per
//! member, and send every call through the gate before it reaches the mapping. The
//! declaration is [`MATRIX`], the checked-in `support-matrix.json` that
//! `cargo xtask compat-report` prints, compiled in so the file a reviewer approved is
//! the file this build honours.
//!
//! One thing is worth stating rather than implying: every member of this adapter is
//! an [`OperationClass::Read`]. A prompter captures nothing and injects nothing, and
//! the state it changes is the caller's — the passphrase leaves this process
//! encrypted and only to the program that asked. Under D13's `portal` mode that
//! means prompts answer while capture does not, which is the intended split: the
//! switch for this adapter is `gnoblin.configure.prompts.enabled`, and the mode gate
//! is what turns it off entirely.

use gnoblin_compat_core::{Adapter, Gate, Mode, OperationClass, Refusal, SupportMatrix, Upstream};

use crate::prompter::{PRIVATE_NAME, PROMPTER_INTERFACE, SYSTEM_NAME};

/// The adapter's short name, used in refusals and in the report.
pub const NAME: &str = "gnome-keyring-prompter";

/// The two bus names this adapter serves, which is also what it steals from GNOME's
/// `gcr-prompter`.
pub const NAMES: [&str; 2] = [SYSTEM_NAME, PRIVATE_NAME];

/// The checked-in declaration, compiled into the binary.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// One call, one variant per member the matrix declares.
///
/// The plan's rule is that the class comes from the adapter and not from the
/// transport, so a caller cannot ask for a weaker class than the call it is making.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    /// A caller registered its callback object.
    BeginPrompting,
    /// A caller asked for a prompt.
    PerformPrompt,
    /// A caller closed its callback object.
    StopPrompting,
}

impl gnoblin_compat_core::Call for Call {
    fn interface(&self) -> &'static str {
        PROMPTER_INTERFACE
    }

    fn member(&self) -> &'static str {
        match self {
            Call::BeginPrompting => "BeginPrompting",
            Call::PerformPrompt => "PerformPrompt",
            Call::StopPrompting => "StopPrompting",
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
}

impl Admission {
    /// Load the checked-in declaration.
    ///
    /// A manifest the frame cannot read is a build-time bug, not a run-time
    /// condition: the bytes are compiled in and the frame's own tests pin the format,
    /// so the caller of this function is a `main` that reports it and exits.
    pub fn load() -> Result<Admission, gnoblin_compat_core::MatrixError> {
        SupportMatrix::from_json(MATRIX).map(|matrix| Admission { matrix })
    }

    /// Admit one call, or return the refusal that names the rule.
    pub fn admit(&self, call: Call, gate: Gate) -> Result<(), Refusal> {
        self.serve(call, gate)
    }
}

impl Adapter for Admission {
    type Call = Call;
    type Reply = ();

    fn name(&self) -> &'static str {
        NAME
    }

    fn upstream(&self) -> Upstream {
        // gcr's prompter interface carries no version of its own, so the version is
        // the one the interface name has always meant.
        Upstream::new(PROMPTER_INTERFACE, 1)
    }

    fn support_matrix(&self) -> &SupportMatrix {
        &self.matrix
    }

    /// The mapping this adapter has to do after the gate.
    ///
    /// Nothing: the gate, the sandbox rule and the matrix are the whole admission
    /// decision, and what a call *does* is asynchronous D-Bus work that no
    /// `handle` can express. Returning `()` here is what keeps the fuse honest —
    /// there is no second place where a refusal could be skipped.
    fn handle(&self, _call: Call) -> Result<(), Refusal> {
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
