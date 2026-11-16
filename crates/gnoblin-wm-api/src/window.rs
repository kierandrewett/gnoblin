//! The records a window manager keeps and the vocabulary of its decisions: what a window and an output are, what a
//! decision asks the caller to do, and what a decision starts from.
//!
//! The records are plain data. A decision never touches a Smithay type: it returns [`WmEffect`]s the caller applies, and
//! reads the paint order and the focus through [`Standing`], so the one record of both stays the scene's and the
//! keyboard's.

use gnoblin_types::{Insets, OutputId, Rect, Size, WindowId};

/// The size of a window whose client asked for none.
pub const DEFAULT_SIZE: Size = Size::new(800, 600);

/// The most windows a provider holds. A window past it is left unmanaged, so a client that opens toplevels in a
/// loop cannot grow the record without bound.
pub const MAX_WINDOWS: usize = 256;

/// What kind of toplevel a window is. A transient is centred over its parent and focus goes to a modal child first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowKind {
    /// An application toplevel.
    #[default]
    Normal,
    /// A dialog with a parent.
    Dialog,
    /// A dialog that blocks its parent.
    Modal,
}

/// The three state flags of a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    /// The window fills its output's work area.
    pub maximised: bool,
    /// The window covers its whole output, over panels.
    pub fullscreen: bool,
    /// The window is hidden and out of the focus rotation.
    pub minimised: bool,
}

/// One window a provider holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// The compositor's identity for the window.
    pub id: WindowId,
    /// What kind of toplevel it is.
    pub kind: WindowKind,
    /// The window it is a transient of, while that window is open.
    pub transient_for: Option<WindowId>,
    /// The output it is on; `None` only while no output is connected.
    pub output: Option<OutputId>,
    /// Its frame rectangle in global logical coordinates.
    pub geometry: Rect,
    /// The floating rectangle it returns to when it leaves maximise or fullscreen. Equal to `geometry` while floating.
    pub restore: Rect,
    /// The state flags.
    pub state: WindowState,
}

/// One connected output: its rectangle and what layer surfaces reserved along its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputRecord {
    /// The output's identity, as the backend numbers it.
    pub id: OutputId,
    /// The whole monitor rectangle, which a fullscreen window covers.
    pub rect: Rect,
    /// The insets layer surfaces reserved.
    pub reserved: Insets,
}

impl OutputRecord {
    /// The monitor rectangle less the reserved insets, never smaller than a pixel.
    #[must_use]
    pub fn work_area(&self) -> Rect {
        self.reserved.applied_to(self.rect).at_least_one()
    }
}

/// What a decision asks the caller to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WmEffect {
    /// Place the window at `geometry` and tell its client the state.
    Configure {
        window: WindowId,
        geometry: Rect,
        state: WindowState,
    },
    /// Show or hide the window.
    Visible { window: WindowId, visible: bool },
    /// The paint order, topmost first.
    Stack(Vec<WindowId>),
    /// Move keyboard focus to the window, or to none.
    Focus(Option<WindowId>),
}

/// The paint order and the focus a decision starts from, which the scene and the keyboard own.
#[derive(Clone, Copy, Debug)]
pub struct Standing<'a> {
    /// Every window drawn, topmost first.
    pub stack: &'a [WindowId],
    /// The focused window.
    pub focus: Option<WindowId>,
}

/// A window the compositor opens.
#[derive(Clone, Copy, Debug)]
pub struct Open {
    pub id: WindowId,
    pub kind: WindowKind,
    pub transient_for: Option<WindowId>,
    /// The rectangle the client asked for; only its size is used.
    pub requested: Option<Rect>,
}

/// Why a provider refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmRefusal {
    /// The window is already open.
    Duplicate,
    /// The record is full.
    TooMany,
    /// No output is connected to place the window on.
    NoOutput,
    /// The window is not open.
    Gone,
    /// The window is maximised or fullscreen, so it has no floating rectangle to change.
    NotFloating,
    /// The output is not connected.
    UnknownOutput,
    /// A transient moves with its parent, not on its own.
    FollowsParent,
}
