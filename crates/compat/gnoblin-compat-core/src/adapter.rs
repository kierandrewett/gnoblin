//! The trait every compat crate implements, and the gate its calls must pass.
//!
//! An adapter is one upstream name translated into the Gnoblin object protocol.
//! It implements three declarations and one mapping:
//!
//! - [`Adapter::name`] and [`Adapter::upstream`] say which name and which
//!   upstream version this adapter speaks;
//! - [`Adapter::support_matrix`] says how faithfully;
//! - [`Call`] types the calls, one variant per member, with the operation class
//!   the gate screens;
//! - [`Adapter::handle`] is the whole mapping, and it is the only code an
//!   adapter crate writes itself.
//!
//! [`Adapter::serve`] is the entry point a transport uses. It applies the mode
//! gate, the sandbox rule and the lock embargo before `handle` can run, then
//! checks the support matrix, so an adapter cannot skip a refusal even by
//! accident: there is no other public path into `handle` from this crate.

use std::fmt;

use crate::matrix::{Support, SupportMatrix};
use crate::mode::{CallerContext, Decision, LockState, Mode, OperationClass, allow};
use crate::refusal::{Refusal, Rule};

/// What one call asks an adapter for, typed by the adapter.
///
/// The interface and member are the names the support matrix declares, and the
/// operation class is what the gate screens. A call that changes state and a
/// call that only reads it must not share a class, or the mode gate would let
/// one through under the other's name.
pub trait Call {
    /// The upstream interface this call targets.
    fn interface(&self) -> &'static str;

    /// The method, property or signal name this call targets.
    fn member(&self) -> &'static str;

    /// What this call does.
    fn operation(&self) -> OperationClass;
}

/// The upstream name and version an adapter was tested against.
///
/// The version is upstream's own number, not a Cargo version: for
/// `DisplayConfig` it is the version its state call returns, and for an
/// unversioned interface it is `1`. [`Adapter::support_matrix`] is what says how
/// much of that version is served.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Upstream {
    interface: &'static str,
    version: u32,
}

impl Upstream {
    /// The name and version an adapter was tested against.
    pub const fn new(interface: &'static str, version: u32) -> Self {
        Self { interface, version }
    }

    /// The upstream interface name.
    pub const fn interface(&self) -> &'static str {
        self.interface
    }

    /// The upstream version.
    pub const fn version(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for Upstream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} version {}", self.interface, self.version)
    }
}

/// Who is calling, how much surface is enabled and whether the session is
/// locked: the three inputs [`allow`] needs, carried together.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Gate {
    mode: Mode,
    caller: CallerContext,
    lock: LockState,
}

impl Gate {
    /// A gate for one call.
    pub const fn new(mode: Mode, caller: CallerContext, lock: LockState) -> Self {
        Self { mode, caller, lock }
    }

    /// The configured mode, `portal` when nobody said otherwise.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Who is calling.
    pub const fn caller(&self) -> CallerContext {
        self.caller
    }

    /// Whether the lock is engaged.
    pub const fn lock(&self) -> LockState {
        self.lock
    }
}

/// One compatibility adapter.
pub trait Adapter {
    /// The adapter's calls, one variant per member it maps.
    type Call: Call;

    /// What a served call answers, in the adapter's own typed values.
    type Reply;

    /// Short name for logs and refusals, such as `screencast`. It is the
    /// adapter, not the bus name: one adapter can serve two names.
    fn name(&self) -> &'static str;

    /// The upstream interface and version this adapter was tested against.
    fn upstream(&self) -> Upstream;

    /// The declared fidelity, usually parsed from the crate's
    /// `support-matrix.json`.
    fn support_matrix(&self) -> &SupportMatrix;

    /// Translate one admitted call. This is the whole mapping.
    fn handle(&self, call: Self::Call) -> Result<Self::Reply, Refusal>;

    /// Serve one call: refuse it, or hand it to [`Adapter::handle`].
    ///
    /// This is the only entry point a transport may use, because it is the only
    /// one that cannot skip the lock embargo, the sandbox rule or the support
    /// matrix. Refusals returned here name the interface and member the call
    /// asked for, so a caller's error and a report agree.
    fn serve(&self, call: Self::Call, gate: Gate) -> Result<Self::Reply, Refusal>
    where
        Self: Sized,
    {
        let interface = call.interface();
        let member = call.member();
        let operation = call.operation();

        if let Decision::Refuse(refusal) = allow(self.name(), operation, gate.caller(), gate.mode(), gate.lock()) {
            return Err(refusal.at(interface, member));
        }

        match self.support_matrix().support(interface, member) {
            Some(Support::Supported | Support::Partial { .. }) => self.handle(call),
            Some(Support::Unsupported { reason }) => {
                Err(Refusal::new(self.name(), operation, Rule::Unsupported, reason.clone()).at(interface, member))
            }
            None => Err(Refusal::new(
                self.name(),
                operation,
                Rule::Undeclared,
                format!("`{interface}.{member}` is not declared, so nothing promises what it does"),
            )
            .at(interface, member)),
        }
    }
}
