//! Everything that can happen to the window manager.
//!
//! The composition root translates protocol events into these values and feeds
//! them to [`crate::Wm::apply`] one at a time, in the order they happened. An event
//! is a fact about the world (a window opened, an output moved) or a request the
//! caller wants honoured (raise, minimise, move). The window manager answers each
//! event with [`crate::Effect`]s and with rejections for events that name unknown
//! objects, so a stale event from a closed window is harmless.
//!
//! Coordinates in events are logical pixels. Time-bearing events carry a
//! [`gnoblin_types::Tick`] in milliseconds from the caller's clock.

use gnoblin_types::{Insets, OutputId, Rect, Scale, WindowId, WorkspaceId};

use crate::activation::ActivationDecision;
use crate::rules::Rule;
use crate::snapping::SnapZone;
use crate::window::WindowKind;

/// The protocol a window arrived on. It decides which surface record the window has.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WindowProtocol {
    /// An xdg_toplevel. The default, because most windows are Wayland windows.
    #[default]
    Wayland,
    /// A managed Xwayland window.
    X11,
}

/// A new window the caller wants managed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowSpec {
    /// The caller's identity for the window.
    pub id: WindowId,
    /// The application id rules match against.
    pub app_id: String,
    /// The title rules match against.
    pub title: String,
    /// What kind of toplevel it is.
    pub kind: WindowKind,
    /// The window this transient follows.
    pub transient_for: Option<WindowId>,
    /// The geometry the client asked for, if it asked for one.
    pub requested: Option<Rect>,
    /// The protocol the window arrived on. A Wayland window gets a `surface_toplevel`
    /// record here; an X11 window's record is `surface_x11`, registered by Xwayland,
    /// so no window has two.
    pub protocol: WindowProtocol,
}

impl WindowSpec {
    /// A normal toplevel with no requested geometry: the common case.
    #[must_use]
    pub fn normal(id: WindowId, app_id: &str) -> Self {
        Self {
            id,
            app_id: app_id.to_owned(),
            title: String::new(),
            kind: WindowKind::Normal,
            transient_for: None,
            requested: None,
            protocol: WindowProtocol::Wayland,
        }
    }

    /// A dialog that follows `parent`.
    #[must_use]
    pub fn transient(id: WindowId, app_id: &str, parent: WindowId) -> Self {
        Self {
            kind: WindowKind::Dialog,
            transient_for: Some(parent),
            ..Self::normal(id, app_id)
        }
    }

    /// Sets the initial title.
    #[must_use]
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = title.to_owned();
        self
    }

    /// Sets the requested geometry.
    #[must_use]
    pub fn with_requested(mut self, requested: Rect) -> Self {
        self.requested = Some(requested);
        self
    }
}

/// An event the caller feeds to [`crate::Wm::apply`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// An output connected with the given monitor rectangle.
    OutputConnected { output: OutputId, rect: Rect },
    /// An output went away.
    OutputDisconnected { output: OutputId },
    /// An output's monitor rectangle changed, for example after a mode set.
    OutputMoved { output: OutputId, rect: Rect },
    /// The space panels reserve on an output changed.
    OutputReserved { output: OutputId, reserved: Insets },
    /// The connector name and scale of an output, as layouts are told about it.
    OutputIdentified {
        output: OutputId,
        name: String,
        scale: Scale,
    },
    /// A client mapped a new window.
    WindowOpened(WindowSpec),
    /// A client unmapped or destroyed a window.
    WindowClosed { window: WindowId },
    /// A window's title changed.
    WindowTitleChanged { window: WindowId, title: String },
    /// A window's application id changed, as it can for a late-starting client.
    WindowAppIdChanged { window: WindowId, app_id: String },
    /// A window's role changed: it became a dialog or a modal dialog, or its parent
    /// changed. `transient_for` of `None` leaves the window without a parent.
    WindowRoleChanged {
        window: WindowId,
        kind: WindowKind,
        transient_for: Option<WindowId>,
    },
    /// The configured window rules were replaced.
    RulesChanged { rules: Vec<Rule> },
    /// A workspace was declared.
    WorkspaceAdded { workspace: WorkspaceId },
    /// A workspace was removed.
    WorkspaceRemoved { workspace: WorkspaceId },
    /// The user switched to a workspace.
    WorkspaceActivated { workspace: WorkspaceId },
    /// A client or the shell asked to focus a window.
    ActivationRequested {
        window: WindowId,
        decision: ActivationDecision,
    },
    /// The pointer is over `window`, or over no managed window when `None`, at `at`.
    PointerMoved {
        window: Option<WindowId>,
        at: gnoblin_types::Tick,
    },
    /// The clock reached `at`; a pending pointer-rest focus may be due.
    PointerTick { at: gnoblin_types::Tick },
    /// A button went down over `window`. The window is focused, and raised when
    /// `raise_on_click` is set.
    PointerPressed { window: WindowId },
    /// A button went down (`active`) or the last one came up. Focus does not follow the
    /// pointer while a button is held.
    PointerGrab { active: bool },
    /// A window should move to the top of its layer.
    Raise { window: WindowId },
    /// A compositor frame. Collects every layout run that finished before
    /// `deadline`; a run still in flight keeps the last good placements.
    Frame { deadline: std::time::Instant },
    /// A window should move to the bottom of its layer.
    Lower { window: WindowId },
    /// A window should be hidden.
    Minimise { window: WindowId },
    /// A window should be shown again.
    Unminimise { window: WindowId },
    /// Maximise or restore a window.
    ToggleMaximise { window: WindowId },
    /// Enter or leave fullscreen.
    ToggleFullscreen { window: WindowId },
    /// Snap a window to a zone, or move it to another zone.
    SnapRequested { window: WindowId, zone: SnapZone },
    /// Return a snapped window to its floating rectangle.
    Unsnap { window: WindowId },
    /// Moves a window between managed and free under a tiling layout. Refused under
    /// a layout whose kind does not allow it.
    ToggleFloat { window: WindowId },
    /// A drag or resize produced this candidate rectangle.
    MoveResize { window: WindowId, geometry: Rect },
    /// Move focus to the nearest visible window in a direction from the focused one.
    FocusDirection { direction: gnoblin_types::Direction },
}

/// Why an event changed nothing.
///
/// A rejected event is not an error in the caller: windows close between the
/// protocol message and the policy call, outputs disappear, and a configuration can
/// name a workspace that no longer exists. Reporting the reason keeps those races
/// out of the compositor's logs as panics or silent misbehaviour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejection {
    /// The window id is already managed.
    DuplicateWindow(WindowId),
    /// No such window is managed.
    UnknownWindow(WindowId),
    /// The output id is already connected.
    DuplicateOutput(OutputId),
    /// No such output is connected.
    UnknownOutput(OutputId),
    /// The workspace id is already declared.
    DuplicateWorkspace(WorkspaceId),
    /// No such workspace is declared.
    UnknownWorkspace(WorkspaceId),
    /// The last remaining workspace cannot be removed.
    LastWorkspace(WorkspaceId),
    /// No workspace is declared, so a window has nowhere to go.
    NoWorkspace,
    /// No output is connected, so a window has nowhere to be shown.
    NoOutput,
    /// The window is minimised, on another workspace or on no output at all.
    NotVisible(WindowId),
    /// The window is fullscreen, so a drag, snap or maximise does not apply.
    NotFloating(WindowId),
    /// The workspace's layout does not allow toggling a window's placement.
    NoFloatToggle(WindowId),
    /// The window is a transient, which always floats and cannot be tiled.
    TransientAlwaysFloating(WindowId),
    /// The parent of a transient is on another workspace, so the transient cannot
    /// follow it. The link is refused and the window keeps its role.
    ParentOnOtherWorkspace { window: WindowId, parent: WindowId },
    /// `window_management.edge_tiling` is off, so edge snapping is unavailable. The
    /// rejection is not about one window, because the setting applies to the session.
    EdgeTilingDisabled,
}
