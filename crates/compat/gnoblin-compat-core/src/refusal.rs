//! Why one call was refused, and the stable rule ids a log or a report can be
//! grepped by.
//!
//! A compat adapter may not fail quietly: the plan's rule is that "a rejected
//! call returns a D-Bus error; it never fails silently" ("Compatibility
//! adapters", rule 2). [`Refusal`] is that error. It always names the adapter,
//! the operation class and the [`Rule`] that refused, and names the interface
//! and member too whenever the call got that far, so a report of two adapters
//! that both serve `Start` cannot be misread as one.

use std::fmt;

use crate::mode::OperationClass;

/// The stable reason one call was refused.
///
/// Every variant has a fixed spelling in [`Rule::as_str`], because a log line,
/// a test and a support report all quote it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Rule {
    /// The adapters are off, so nothing owns its upstream name.
    ModeOff,
    /// A state change in `portal` mode, which is off until a user enables it
    /// (D13).
    WriteNotEnabled,
    /// Capture or input injection in `portal` mode, where the Gnoblin portal is
    /// the only path after consent.
    PortalOnly,
    /// Capture asked for by a sandboxed caller, which no mode allows.
    SandboxedCapture,
    /// Input injection asked for by a sandboxed caller, which no mode allows.
    SandboxedInput,
    /// Capture while the session is locking or locked: the lock embargo.
    LockedCapture,
    /// The member is declared `unsupported` in the adapter's matrix.
    Unsupported,
    /// The member is not declared in the matrix at all.
    Undeclared,
    /// The adapter's own mapping refused the call after the gate admitted it.
    Adapter,
}

impl Rule {
    /// The id quoted by logs, reports and tests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModeOff => "mode-off",
            Self::WriteNotEnabled => "write-not-enabled",
            Self::PortalOnly => "capture-portal-only",
            Self::SandboxedCapture => "sandboxed-capture",
            Self::SandboxedInput => "sandboxed-input",
            Self::LockedCapture => "locked-capture",
            Self::Unsupported => "unsupported",
            Self::Undeclared => "undeclared",
            Self::Adapter => "adapter",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One refused call.
///
/// The fields are exactly what a caller, a log and a support report need: which
/// adapter, which operation, which rule, and what the caller asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Refusal {
    adapter: &'static str,
    interface: Option<&'static str>,
    member: Option<&'static str>,
    operation: OperationClass,
    rule: Rule,
    detail: String,
}

impl Refusal {
    /// A refusal that names the adapter, the operation and the rule.
    ///
    /// [`crate::Adapter::serve`] calls this itself; an adapter uses it when its
    /// own mapping refuses a call that the gate admitted, with
    /// [`Rule::Adapter`].
    pub fn new(adapter: &'static str, operation: OperationClass, rule: Rule, detail: impl Into<String>) -> Self {
        Self {
            adapter,
            interface: None,
            member: None,
            operation,
            rule,
            detail: detail.into(),
        }
    }

    /// Name the interface and member the caller asked for.
    pub fn at(mut self, interface: &'static str, member: &'static str) -> Self {
        self.interface = Some(interface);
        self.member = Some(member);
        self
    }

    /// The adapter that refused, by its short name such as `screencast`.
    pub const fn adapter(&self) -> &'static str {
        self.adapter
    }

    /// The interface the call targeted, when the call reached that far.
    pub const fn interface(&self) -> Option<&'static str> {
        self.interface
    }

    /// The member the call targeted, when the call reached that far.
    pub const fn member(&self) -> Option<&'static str> {
        self.member
    }

    /// What the call was screened as.
    pub const fn operation(&self) -> OperationClass {
        self.operation
    }

    /// The rule that refused.
    pub const fn rule(&self) -> Rule {
        self.rule
    }

    /// The sentence a person reads: why this call is refused.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "compat {}: ", self.adapter)?;
        if let (Some(interface), Some(member)) = (self.interface, self.member) {
            write!(formatter, "{interface}.{member} ")?;
        }
        write!(
            formatter,
            "({}) refused [{}]: {}",
            self.operation,
            self.rule.as_str(),
            self.detail
        )
    }
}

impl std::error::Error for Refusal {}
