//! What the window manager asks the rest of the compositor to do.
//!
//! Effects are the only output of [`crate::Wm::apply`]. They are declarative and
//! idempotent: applying the same effect twice is a no-op, so a caller that loses
//! track can re-send the current state. Nothing here names a protocol object; the
//! composition root maps a [`Effect::Configure`] to a toplevel configure, a
//! [`Effect::Stack`] to scene-graph reordering and a [`Effect::Focus`] to seat
//! keyboard focus.

use gnoblin_types::{Rect, WindowId, WorkspaceId};

use crate::rules::WorkspaceSelector;
use crate::window::WindowState;

/// Why a rule could not be applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleRejection {
    /// The workspace the rule names is not declared. The window stays where it is,
    /// and the caller logs a warning, which is what the window-rules guide
    /// promises when a placement target does not exist.
    MissingWorkspace(WorkspaceSelector),
}

/// A request to the compositor.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Send this geometry and state to the client and the frame renderer.
    Configure {
        /// The window to configure.
        window: WindowId,
        /// Its frame rectangle in global logical coordinates.
        geometry: Rect,
        /// Its maximise, fullscreen, minimise and snap flags.
        state: WindowState,
    },
    /// Show or hide the window.
    SetVisible {
        /// The window to show or hide.
        window: WindowId,
        /// Whether it should be drawn.
        visible: bool,
    },
    /// The stacking order changed. The list is topmost first and holds every
    /// managed window exactly once.
    Stack {
        /// Window ids, topmost first.
        order: Vec<WindowId>,
    },
    /// Keyboard focus moved.
    Focus {
        /// The previously focused window, if any.
        previous: Option<WindowId>,
        /// The newly focused window, if any.
        focused: Option<WindowId>,
    },
    /// A caller-supplied activation decision refused focus, so nothing changed.
    ///
    /// The caller usually turns this into an `activation-denied` signal; the window
    /// manager reports it because refusing is observable behaviour that must be
    /// distinguishable from a request that silently did nothing.
    ActivationDenied {
        /// The window that asked for activation.
        window: WindowId,
    },
    /// The active workspace changed. Windows on the previous workspace are hidden
    /// by accompanying [`Effect::SetVisible`] effects.
    WorkspaceShown {
        /// The workspace now shown on the outputs.
        workspace: WorkspaceId,
    },
    /// A rule could not be applied to a newly managed window.
    RuleRejected {
        /// The window the rule would have applied to.
        window: WindowId,
        /// The index of the rule in the configured list.
        rule: usize,
        /// Why it did not apply.
        reason: RuleRejection,
    },
    /// The event changed nothing.
    Rejected {
        /// Why.
        reason: crate::event::Rejection,
    },
}

impl Effect {
    /// A rejection effect, for the many event arms that change nothing.
    pub(crate) fn rejected(reason: crate::event::Rejection) -> Self {
        Self::Rejected { reason }
    }

    /// A configure effect for a window's current geometry and state.
    pub(crate) fn configure(window: WindowId, geometry: Rect, state: WindowState) -> Self {
        Self::Configure {
            window,
            geometry,
            state,
        }
    }

    /// A visibility effect.
    pub(crate) fn visible(window: WindowId, visible: bool) -> Self {
        Self::SetVisible { window, visible }
    }

    /// A stacking effect.
    pub(crate) fn stack(order: Vec<WindowId>) -> Self {
        Self::Stack { order }
    }

    /// A focus effect.
    pub(crate) fn focus(previous: Option<WindowId>, focused: Option<WindowId>) -> Self {
        Self::Focus { previous, focused }
    }

    /// A workspace effect.
    pub(crate) fn workspace_shown(workspace: WorkspaceId) -> Self {
        Self::WorkspaceShown { workspace }
    }

    /// A rule-rejection effect.
    pub(crate) fn rule_rejected(window: WindowId, rule: usize, reason: RuleRejection) -> Self {
        Self::RuleRejected { window, rule, reason }
    }
}
