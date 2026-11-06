//! The activation decision the caller supplies with a focus request.
//!
//! Focus stealing is decided before the window manager runs. Attribution (was this
//! request made by the process that owns the window, with a valid activation token
//! or a trusted input context) lives in `gnoblin-authority`, and the
//! `window_management.focus_new_windows` policy is configuration. Together they
//! produce one of the three answers below, and the window manager only has to carry
//! out the one it is given. Keeping the decision out of this crate is why policy
//! tests can drive focus without a session or a token.

/// Whether a request to activate a window may take focus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivationDecision {
    /// Focus the window and raise it, which is what a click or a trusted activation
    /// request does.
    Allow,
    /// Focus the window without raising it, for focus-follows-pointer and for a
    /// window whose activation was allowed but whose stacking the caller wants to
    /// leave alone.
    AllowWithoutRaise,
    /// Refuse the request. The window keeps its place and the current focus stays.
    Deny,
}

/// The new-window focus policy, `window_management.focus_new_windows`.
///
/// It decides whether a window that has just been mapped may take focus. Activation
/// requests with a valid token do not depend on it: those are decided by the caller.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NewWindowFocus {
    /// A new window may take focus.
    Allow,
    /// A new window takes focus when nothing is focused, or when it is a transient
    /// child of the focused window. Otherwise it is left unfocused, so it cannot
    /// steal focus from a window that has it.
    #[default]
    Prevent,
}

impl ActivationDecision {
    /// Whether acting on this decision raises the window.
    #[must_use]
    pub const fn raises(self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Whether acting on this decision changes focus at all.
    #[must_use]
    pub const fn permits_focus(self) -> bool {
        !matches!(self, Self::Deny)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allow_raises() {
        assert!(ActivationDecision::Allow.raises());
        assert!(!ActivationDecision::AllowWithoutRaise.raises());
        assert!(!ActivationDecision::Deny.raises());
    }

    #[test]
    fn only_deny_refuses_focus() {
        assert!(ActivationDecision::Allow.permits_focus());
        assert!(ActivationDecision::AllowWithoutRaise.permits_focus());
        assert!(!ActivationDecision::Deny.permits_focus());
    }
}
