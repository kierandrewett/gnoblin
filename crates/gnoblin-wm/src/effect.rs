//! What the window manager asks the rest of the compositor to do.
//!
//! Effects are the only output of [`crate::Wm::apply`]. They are declarative and
//! idempotent: applying the same effect twice is a no-op, so a caller that loses
//! track can re-send the current state. Nothing here names a protocol object; the
//! composition root maps a [`Effect::Configure`] to a toplevel configure, a
//! [`Effect::Stack`] to scene-graph reordering and a [`Effect::Focus`] to seat
//! keyboard focus.

use gnoblin_types::{Rect, WindowId, WorkspaceId};

use crate::layout::Placement;
use crate::rules::WorkspaceSelector;
use crate::tiled::TiledEdges;
use crate::window::WindowState;

/// Which way the active workspace moved in the workspace order.
///
/// The renderer slides the outgoing windows off the side the new workspace is not on, so a
/// switch from workspace 1 to 2 moves content left, and back again moves it right. A switch
/// with no order to read, such as the removal of the active workspace, carries no direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkspaceDirection {
    /// The new workspace comes after the old one in the order.
    Later,
    /// The new workspace comes before the old one in the order.
    Earlier,
}

/// Why a rule could not be applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuleRejection {
    /// The workspace the rule names is not declared. The window stays where it is,
    /// and the caller logs a warning, which is what the window-rules guide
    /// promises when a placement target does not exist.
    MissingWorkspace(WorkspaceSelector),
}

/// A request to the compositor.
#[derive(Clone, PartialEq, Debug)]
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
    /// A window's surface became secure or stopped being secure, because a rule that
    /// matched it changed. The compositor masks secure surfaces in every capture.
    SecureChanged {
        /// The window whose surface changed.
        window: WindowId,
        /// Whether its surface is secure now.
        secure: bool,
    },
    /// A window's opacity or corner radius changed because a rule that matched it
    /// changed. The compositor blends and rounds the window's surface with these values.
    Appearance {
        /// The window whose appearance changed.
        window: WindowId,
        /// Opacity from 0 (invisible) to 1 (opaque).
        opacity: f32,
        /// Corner radius in logical pixels; 0 draws square corners.
        corner_radius: f32,
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
        /// Which way the workspace moved, or `None` when the order gives no direction.
        direction: Option<WorkspaceDirection>,
    },
    /// A workspace set named an output that is not connected, so its workspace went to
    /// the primary output instead. The set is still activated; this reports the fallback.
    WorkspaceSetOutputFallback {
        /// The set that named the output.
        set: String,
        /// The connector name the set asked for.
        output: String,
    },
    /// A workspace set changed which workspace each output shows. Emitted only when the
    /// shown workspaces actually change, so activating the live set again is silent.
    WorkspaceSetActivated {
        /// The set that became live.
        name: String,
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
    /// A layout settled and places this window. Sent once per placed window of a
    /// settled run; the compositor applies the 3D placement, visibility, opacity and
    /// decoration it carries.
    PlacementTarget {
        /// The workspace whose layout produced the placement.
        workspace: WorkspaceId,
        /// Where the window goes and how it is drawn.
        placement: Placement,
        /// Which edges of the window are tiled, for the xdg `tiled_*` states. All false
        /// unless the workspace's layout tiles.
        tiled: TiledEdges,
        /// The work area of the window's output: the largest size the window should take,
        /// sent as the xdg `configure_bounds` hint. `None` while no output is connected.
        work_area: Option<Rect>,
    },
    /// A layout run failed or its input was refused. The last good placements of
    /// the workspace stay in force.
    LayoutFailed {
        /// The workspace whose layout failed.
        workspace: WorkspaceId,
        /// The failure's name, as in [`crate::layout::LayoutFailure::reason`], or
        /// `invalid-params` when the input was refused.
        reason: &'static str,
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
    pub(crate) fn workspace_shown(workspace: WorkspaceId, direction: Option<WorkspaceDirection>) -> Self {
        Self::WorkspaceShown { workspace, direction }
    }

    /// A rule-rejection effect.
    pub(crate) fn rule_rejected(window: WindowId, rule: usize, reason: RuleRejection) -> Self {
        Self::RuleRejected { window, rule, reason }
    }
}
