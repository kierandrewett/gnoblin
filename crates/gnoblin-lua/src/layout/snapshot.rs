//! What the host hands a layout: the output, the work area, the tiled windows in
//! stacking order, the focus, the gaps, and the messages the window manager sends.
//! There is no clock field: a layout's output depends only on this snapshot and its
//! own state.

use gnoblin_types::{OutputId, RectF, Scale, WindowId, WorkspaceId};

use super::data::Data;

/// The output a workspace is on.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputInfo {
    /// The output's id.
    pub id: OutputId,
    /// The output's connector name.
    pub name: String,
    /// The output's scale.
    pub scale: Scale,
}

/// A window's size limits. A maximum of zero means no limit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeHints {
    /// Smallest width the window accepts.
    pub min_width: f64,
    /// Smallest height the window accepts.
    pub min_height: f64,
    /// Largest width the window accepts, or zero for no limit.
    pub max_width: f64,
    /// Largest height the window accepts, or zero for no limit.
    pub max_height: f64,
}

/// One tiled window as a layout sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotWindow {
    /// The window's id.
    pub id: WindowId,
    /// The application id.
    pub app_id: String,
    /// The window title.
    pub title: String,
    /// Current width.
    pub width: f64,
    /// Current height.
    pub height: f64,
    /// The window's size limits.
    pub size_hints: SizeHints,
}

/// The gaps around and between tiles, and the border width.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gaps {
    /// Gap between neighbouring tiles.
    pub inner: f64,
    /// Gap between the tiles and the area's edge.
    pub outer: f64,
    /// Border width. Not clamped.
    pub border: f64,
}

impl Gaps {
    /// The gaps a layout sees for `area`. A gap wider than the area's smaller side
    /// is zero, because the area is too small to hold it.
    pub fn clamped(self, area: RectF) -> Self {
        let limit = area.width.min(area.height);
        let clamp = |gap: f64| if gap > limit { 0.0 } else { gap };
        Self {
            inner: clamp(self.inner),
            outer: clamp(self.outer),
            border: self.border,
        }
    }
}

/// Everything a layout is told for one arrange or message.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutSnapshot {
    /// The workspace. Its layout state is kept per workspace.
    pub workspace: WorkspaceId,
    /// The output the workspace is on.
    pub output: OutputInfo,
    /// The work area the layout places windows in.
    pub area: RectF,
    /// The tiled windows, in stacking order.
    pub windows: Vec<SnapshotWindow>,
    /// The focused window, if it is tiled.
    pub focus: Option<WindowId>,
    /// The gaps. [`Gaps::clamped`] is applied by the host before the call.
    pub gaps: Gaps,
}

/// One message to a layout, other than the batch arrange pass.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutRequest {
    /// A window joins the tiled set. Delivered to `on_request` as `kind = "place"`.
    Place {
        /// The window that joins.
        window: WindowId,
    },
    /// A window leaves the tiled set. Delivered to `on_request` as `kind = "remove"`.
    Remove {
        /// The window that leaves.
        window: WindowId,
    },
    /// A named command, delivered to `command[name]` with its parameters.
    Command {
        /// The command name.
        name: String,
        /// The caller's parameters, as plain data.
        params: Data,
    },
}
