//! The mode gate: how much compat surface answers, to whom, at which moment.
//!
//! Three inputs decide one call, and [`allow`] is a pure function of them, so a
//! refusal is reproducible from the values in its log line:
//!
//! - [`Mode`] is `gnoblin.configure.compat.capture`: the default set from D13
//!   is "reads on; writes off; capture via portal only";
//! - [`CallerContext`] says whether the caller is sandboxed, which no mode
//!   relaxes;
//! - [`LockState`] is the lock embargo, which refuses capture whenever the
//!   session is not fully unlocked.
//!
//! Precedence is fixed and documented because a refusal quotes exactly one rule:
//! the mode first (an adapter that is off answers nothing at all), then the
//! sandbox rule (which holds in every mode), then the lock embargo (which
//! protects the modes that would otherwise permit capture).
//!
//! Input injection is deliberately not refused here by the lock: injected input
//! travels the normal object-protocol path, and `gnoblin-lock`'s input gate is
//! what keeps it away from a covered client. Refusing it a second time here
//! would be a second policy that could drift.

use std::fmt;

use crate::refusal::{Refusal, Rule};

/// How much of the compatibility surface answers at all.
///
/// The values are the strings of `gnoblin.configure.compat.capture`. A user
/// chooses them; nothing here reads configuration.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// `off`: no adapter owns its upstream name, so every call is refused.
    Off,
    /// `portal`, the default from D13: reads answer, state changes and capture
    /// do not, and capture and input happen through the Gnoblin portal only.
    #[default]
    Portal,
    /// `unmediated`: the upstream names answer as Mutter answers, for
    /// unsandboxed same-user callers, as far as the support matrix allows.
    Unmediated,
}

impl Mode {
    /// The configuration spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Portal => "portal",
            Self::Unmediated => "unmediated",
        }
    }

    /// Parse the configured value.
    ///
    /// `None` means the value is not one of the three, which the configuration
    /// layer reports as a problem naming the key rather than guessing a mode.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "portal" => Some(Self::Portal),
            "unmediated" => Some(Self::Unmediated),
            _ => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What one call does, which is what the gate screens.
///
/// The class comes from the adapter's own [`crate::Call`] implementation, so a
/// transport cannot choose a weaker class for a call that changes state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum OperationClass {
    /// Reads state: a property read, a getter, an introspection call.
    Read,
    /// Changes state: a setter, an apply, a logout, a session start.
    Write,
    /// Produces frames or pixels: ScreenCast, Shell Screenshot, Clipboard read.
    Capture,
    /// Injects input: RemoteDesktop, InputCapture.
    InputInjection,
}

impl OperationClass {
    /// The spelling used in refusals and reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Capture => "capture",
            Self::InputInjection => "input injection",
        }
    }

    /// Whether a sandboxed caller is refused this class in every mode.
    pub const fn is_privileged(self) -> bool {
        matches!(self, Self::Capture | Self::InputInjection)
    }
}

impl fmt::Display for OperationClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What isolates the caller from the rest of the session.
///
/// Any same-user process can own a bus name or run unsandboxed, so a bus name
/// proves nothing about a caller; the sandbox it runs in is what the frame can
/// see. Both variants the plan names are here so a refusal can say which one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sandbox {
    /// An ordinary same-user process outside any sandbox.
    #[default]
    None,
    /// A Flatpak application.
    Flatpak,
    /// A client behind a `wp_security_context_v1` security context.
    SecurityContext,
}

impl Sandbox {
    /// The spelling used in refusals.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "unsandboxed",
            Self::Flatpak => "Flatpak",
            Self::SecurityContext => "wp_security_context_v1",
        }
    }
}

impl fmt::Display for CallerContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.sandbox.as_str())
    }
}

impl fmt::Display for Sandbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Who is calling, as far as the frame can tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CallerContext {
    sandbox: Sandbox,
}

impl CallerContext {
    /// An ordinary same-user process, which is what Mutter's capture names
    /// accept today.
    pub const UNSANDBOXED: Self = Self { sandbox: Sandbox::None };

    /// A Flatpak caller.
    pub const fn flatpak() -> Self {
        Self {
            sandbox: Sandbox::Flatpak,
        }
    }

    /// A caller behind a `wp_security_context_v1` security context.
    pub const fn security_context() -> Self {
        Self {
            sandbox: Sandbox::SecurityContext,
        }
    }

    /// The sandbox the caller runs in.
    pub const fn sandbox(&self) -> Sandbox {
        self.sandbox
    }

    /// Whether capture and input injection must be refused outright.
    pub const fn is_sandboxed(&self) -> bool {
        !matches!(self.sandbox, Sandbox::None)
    }
}

/// Whether the session lock is holding the desktop.
///
/// `Locking` is the transition before a cover has been presented. Both engaged
/// states refuse capture, matching `gnoblin-lock`'s `Lock::capture`, which
/// answers [`crate::Decision`]-style refusals until `State::Unlocked`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LockState {
    /// No lock request is pending or active.
    #[default]
    Unlocked,
    /// A lock was requested and its cover is not presented yet.
    Locking,
    /// The cover is presented and the session is locked.
    Locked,
}

impl LockState {
    /// Whether the lock embargo is in force.
    pub const fn is_engaged(self) -> bool {
        !matches!(self, Self::Unlocked)
    }

    /// The spelling used in refusals.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unlocked => "unlocked",
            Self::Locking => "locking",
            Self::Locked => "locked",
        }
    }
}

impl fmt::Display for LockState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The answer for one call: allowed, or refused with the reason.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decision {
    /// The call may proceed to the adapter's mapping.
    Allow,
    /// The call is refused; the refusal names the adapter, operation and rule.
    Refuse(Refusal),
}

impl Decision {
    /// Whether the call may proceed.
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// The refusal, when there is one.
    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Self::Allow => None,
            Self::Refuse(refusal) => Some(refusal),
        }
    }
}

/// Decide one call from the mode, the caller and the lock alone.
///
/// Pure: the same five inputs always answer the same way, which is what lets a
/// refusal be replayed from the values in a log line.
///
/// Only the adapter name, the operation and the rule are known here, so the
/// [`Refusal`] this returns has no interface or member yet;
/// [`crate::Adapter::serve`] adds them from the call before returning it.
pub fn allow(
    adapter: &'static str,
    operation: OperationClass,
    caller: CallerContext,
    mode: Mode,
    lock: LockState,
) -> Decision {
    // The mode first: an adapter that is off or read-only never sees the call,
    // so its refusal is the mode's, whatever the caller or the lock would add.
    match (mode, operation) {
        (Mode::Off, _) => {
            return refuse(
                adapter,
                operation,
                Rule::ModeOff,
                "the compat adapters are off (`gnoblin.configure.compat.capture = off`)",
            );
        }
        (Mode::Portal, OperationClass::Read) => {}
        (Mode::Portal, OperationClass::Write) => {
            return refuse(
                adapter,
                operation,
                Rule::WriteNotEnabled,
                "state changes are off by default (D13); enable them in `gnoblin.configure.compat`",
            );
        }
        (Mode::Portal, OperationClass::Capture | OperationClass::InputInjection) => {
            return refuse(
                adapter,
                operation,
                Rule::PortalOnly,
                "capture and input happen only through the Gnoblin portal in `portal` mode, after consent",
            );
        }
        (Mode::Unmediated, _) => {}
    }

    // A sandboxed caller never captures and never injects, in every mode: a
    // sandbox is the one thing about a caller this frame can rely on.
    if caller.is_sandboxed() {
        let sandbox = caller.sandbox();
        match operation {
            OperationClass::Capture => {
                return refuse(
                    adapter,
                    operation,
                    Rule::SandboxedCapture,
                    format!("capture is refused for a {sandbox} caller in every mode"),
                );
            }
            OperationClass::InputInjection => {
                return refuse(
                    adapter,
                    operation,
                    Rule::SandboxedInput,
                    format!("input injection is refused for a {sandbox} caller in every mode"),
                );
            }
            OperationClass::Read | OperationClass::Write => {}
        }
    }

    // The lock embargo, which is what keeps `unmediated` from capturing the
    // lock screen.
    if operation == OperationClass::Capture && lock.is_engaged() {
        let when = if lock == LockState::Locked {
            "while the session is locked"
        } else {
            "during the lock transition"
        };
        return refuse(
            adapter,
            operation,
            Rule::LockedCapture,
            format!("capture is refused {when}"),
        );
    }

    Decision::Allow
}

fn refuse(adapter: &'static str, operation: OperationClass, rule: Rule, detail: impl Into<String>) -> Decision {
    Decision::Refuse(Refusal::new(adapter, operation, rule, detail))
}
