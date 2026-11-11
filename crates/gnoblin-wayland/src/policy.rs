//! The window management options a configuration document carries, and the three choices in them.
//!
//! The compositor decides none of this itself. The values are published on the main seat
//! ([`State::publish_seat_policy`](crate::State)) where the stock shell reads them, and the no-shell
//! fallback ([`crate::fallback`]) reads only [`NewWindowFocus`].

/// How far a moved or resized window may leave its work area.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DragConstraint {
    /// The whole window stays inside the work area. This is the default.
    #[default]
    WorkArea,
    /// A strip of the window stays inside the work area in each direction, so a window can be pushed partly
    /// off screen but never become unreachable.
    MinimumStrip,
}

/// How keyboard focus follows the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FocusMode {
    /// A click moves focus. This is the default.
    #[default]
    Click,
    /// Entering a window focuses it; crossing a gap keeps the focus where it was.
    Hover,
    /// Entering a window focuses it; leaving every window clears the focus.
    HoverStrict,
}

/// Whether a window that has just mapped may take focus (`window_management.focus_new_windows`).
///
/// An activation request with a valid token does not depend on it: those are decided by the caller.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NewWindowFocus {
    /// A new window may take focus.
    Allow,
    /// A new window takes focus when nothing is focused, or when it is a transient child of the focused
    /// window. Otherwise it is left unfocused, so it cannot steal focus from a window that has it.
    #[default]
    Prevent,
}

/// The configuration the seat policy is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WmOptions {
    /// `window_management.edge_tiling`.
    pub edge_tiling: bool,
    /// `window_management.constrain_drag_to_work_area`.
    pub drag_constraint: DragConstraint,
    /// `window_management.focus_new_windows`.
    pub new_window_focus: NewWindowFocus,
    /// `window_management.focus_mode`.
    pub focus_mode: FocusMode,
    /// `window_management.focus_change_on_pointer_rest`.
    pub focus_change_on_pointer_rest: bool,
    /// `window_management.raise_on_click`.
    pub raise_on_click: bool,
    /// `window_management.auto_raise`.
    pub auto_raise: bool,
    /// `window_management.auto_raise_delay`, in milliseconds.
    pub auto_raise_delay: u64,
}

impl Default for WmOptions {
    /// The defaults of the configuration reference, which are the values a session gets when its
    /// configuration omits the keys.
    fn default() -> Self {
        Self {
            edge_tiling: false,
            drag_constraint: DragConstraint::WorkArea,
            new_window_focus: NewWindowFocus::Prevent,
            focus_mode: FocusMode::Click,
            focus_change_on_pointer_rest: false,
            raise_on_click: true,
            auto_raise: false,
            auto_raise_delay: 500,
        }
    }
}

impl DragConstraint {
    /// The name the seat property carries.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::WorkArea => "work_area",
            Self::MinimumStrip => "minimum_strip",
        }
    }
}

impl FocusMode {
    /// The name the seat property carries.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Hover => "hover",
            Self::HoverStrict => "hover-strict",
        }
    }
}

impl NewWindowFocus {
    /// The name the seat property carries.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Prevent => "prevent",
        }
    }
}
