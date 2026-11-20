//! What a refused activation looks like to a reader of the session.

use gnoblin_types::WindowId;

/// An `xdg_activation_v1` request the compositor refused, with the authority's reason.
///
/// The reason is the authority's own word for it (`unknown-token`, `reused-token`, `expired-token`,
/// `session-locked`, ...) or `not-a-window` when the token named a surface that is not a managed window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActivationDenial {
    /// The window that asked, when the surface was a managed one.
    pub window: Option<WindowId>,
    /// Why it was refused.
    pub reason: String,
}
