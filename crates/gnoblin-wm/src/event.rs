//! Everything that can happen to the window manager.

use crate::activation::ActivationDecision;
use crate::geometry::{Insets, Rect};
use crate::ids::{OutputId, WindowId, WorkspaceId};
use crate::rules::Rule;
use crate::snapping::SnapZone;
use crate::window::WindowKind;

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
    /// A client mapped a new window.
    WindowOpened(WindowSpec),
    /// A client unmapped or destroyed a window.
    WindowClosed { window: WindowId },
    /// A window's title changed.
    WindowTitleChanged { window: WindowId, title: String },
    /// A window's application id changed, as it can for a late-starting client.
    WindowAppIdChanged { window: WindowId, app_id: String },
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
    /// A window should move to the top of its layer.
    Raise { window: WindowId },
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
    /// A drag or resize produced this candidate rectangle.
    MoveResize { window: WindowId, geometry: Rect },
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
    /// `window_management.edge_tiling` is off, so edge snapping is unavailable. The
    /// rejection is not about one window, because the setting applies to the session.
    EdgeTilingDisabled,
}
