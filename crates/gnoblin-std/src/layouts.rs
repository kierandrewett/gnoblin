//! The library's tiling layouts: [`MasterStackLayout`], [`GridLayout`], [`MonocleLayout`], [`DwindleLayout`], [`StripLayout`],
//! [`ArcLayout`] and [`Carousel3dLayout`].
//!
//! Each is a [`Layout`] like any a shell writes: it runs through the same C table, so the library's pass cannot tell it from a shell's
//! own, and a shell may read this file as the model for one. They are ports of `data/js/gnoblin-std/layouts/*.ts`, which are the
//! reference: the same arithmetic, the same clamps, the same flooring of every tile edge, so a workspace lays the same windows out the
//! same way whichever language the shell is written in (the parity tests of `gnoblin-std-impl` hold them to it).
//!
//! The dwindle and the strip keep a record of who sits where; they are handles, so a shell keeps a clone for the actions that change
//! the record (`toggle_split`, `focus_left`, ...) and gives the other to the workspace. The dwindle also answers
//! [`Layout::find_split`](crate::Layout::find_split) for the split-resize drag. The arc and the carousel answer with the 3D fields of
//! a [`Placement`] (depth, rotation, offset).

use gnoblin_core::Rect;

use crate::bindings::Point;
use crate::client::Client;
use crate::layout::{Layout, LayoutContext, Placement, inset_rect};
use crate::split::{Divider, Split, SplitAxis};

mod arc;
mod carousel;
mod dwindle;
mod strip;

pub use arc::ArcLayout;
pub use carousel::{Carousel3dLayout, RingOptions, RingSlot, ring};
pub use dwindle::{DwindleLayout, DwindleOptions, ForceSplit, PointerSource, Preselect, SplitBias, SplitMode};
pub use strip::StripLayout;

/// The smallest and largest share of the span the master column may take.
const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

/// `value` limited to `low..=high`, as the TypeScript library's `clamp`: `low` must not be above `high`.
pub(crate) fn clamp(value: f64, low: f64, high: f64) -> f64 {
    low.max(high.min(value))
}

/// The rectangle a tiling layout fills and the gap between its tiles: the area inset by the workspace's outer gap, and the layout's own
/// gap when it has one, else the workspace's inner gap.
fn geometry(own: Option<f64>, cx: &LayoutContext) -> (Rect, f64) {
    (inset_rect(&cx.area, cx.gap_outer), own.unwrap_or(cx.gap_inner))
}

/// Where the master area sits in the workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// Master on the left, the stack to its right (the default).
    #[default]
    Left,
    /// Master on the right.
    Right,
    /// Master on top, the stack below.
    Top,
    /// Master at the bottom.
    Bottom,
}

/// One master area and a stack: the first `master_count` windows share the master area, the rest share the stack.
///
/// ```
/// # use gnoblin_std::MasterStackLayout;
/// let layout = MasterStackLayout::new().ratio(0.6).master_count(2).gap(4.0);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct MasterStackLayout {
    ratio: f64,
    master_count: usize,
    gap: Option<f64>,
    orientation: Orientation,
    split: SplitState,
}

impl Default for MasterStackLayout {
    fn default() -> MasterStackLayout {
        MasterStackLayout {
            ratio: 0.5,
            master_count: 1,
            gap: None,
            orientation: Orientation::Left,
            split: SplitState::default(),
        }
    }
}

impl MasterStackLayout {
    /// Half the span for the master, one master window, the workspace's gaps.
    pub fn new() -> MasterStackLayout {
        MasterStackLayout::default()
    }

    /// The share of the span the master area takes, limited to 0.1 to 0.9 when it is used.
    #[must_use]
    pub fn ratio(mut self, ratio: f64) -> MasterStackLayout {
        self.ratio = ratio;
        self
    }

    /// How many windows share the master area; 0 makes everything stack.
    #[must_use]
    pub fn master_count(mut self, count: usize) -> MasterStackLayout {
        self.master_count = count;
        self
    }

    /// The gap between tiles, replacing the workspace's inner gap.
    #[must_use]
    pub fn gap(mut self, gap: f64) -> MasterStackLayout {
        self.gap = Some(gap);
        self
    }

    /// Which side the master area is on.
    #[must_use]
    pub fn orientation(mut self, orientation: Orientation) -> MasterStackLayout {
        self.orientation = orientation;
        self
    }
}

/// What the master-stack divider needs between a compute and a drag: the windows of the last compute, how many are master, the span
/// the master and the stack divide, and the drag in flight. Layouts compare equal whatever it holds.
#[derive(Clone, Debug, Default)]
struct SplitState {
    last: Option<(Vec<u64>, usize, f64)>,
    drag: Option<(u64, f64)>,
    token: u64,
    revision: u64,
}

impl PartialEq for SplitState {
    fn eq(&self, _other: &SplitState) -> bool {
        true
    }
}

/// A tile as `[u, v, p, q]`: `u` and `p` along the span the master and the stack divide, `v` and `q` across it.
type Frame = [f64; 4];

fn master_stack_frames(span: f64, across: f64, n: usize, master: usize, ratio: f64, gap: f64) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(n);
    let m = master.min(n);
    let s = n - m;
    let mut master_width = 0.0;
    if m > 0 {
        master_width = if s == 0 { span } else { ((span - gap) * ratio).max(0.0) };
        let h = ((across - (m - 1) as f64 * gap) / m as f64).max(0.0);
        for i in 0..m {
            frames.push([0.0, i as f64 * (h + gap), master_width, h]);
        }
    }
    if s > 0 {
        let start = if m > 0 { master_width + gap } else { 0.0 };
        let w = (span - start).max(0.0);
        let h = ((across - (s - 1) as f64 * gap) / s as f64).max(0.0);
        for i in 0..s {
            frames.push([start, i as f64 * (h + gap), w, h]);
        }
    }
    frames
}

impl Layout for MasterStackLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        if cx.children.is_empty() {
            return Vec::new();
        }
        let ratio = if self.ratio.is_finite() {
            self.ratio.clamp(MIN_RATIO, MAX_RATIO)
        } else {
            0.5
        };
        let (area, gap) = geometry(self.gap, cx);
        let sideways = matches!(self.orientation, Orientation::Top | Orientation::Bottom);
        let (span, across) = if sideways {
            (area.height, area.width)
        } else {
            (area.width, area.height)
        };
        self.split.last = Some((
            cx.children.iter().map(|client| client.window().id().0).collect(),
            self.master_count,
            span - gap,
        ));
        let frames = master_stack_frames(span, across, cx.children.len(), self.master_count, ratio, gap);
        cx.children
            .iter()
            .zip(frames)
            .map(|(client, [u, v, p, q])| {
                let (rx, ry, rw, rh) = match self.orientation {
                    Orientation::Left => (u, v, p, q),
                    Orientation::Right => (area.width - u - p, v, p, q),
                    Orientation::Top => (v, u, q, p),
                    Orientation::Bottom => (v, area.height - u - p, q, p),
                };
                Placement::tile(client, area.x + rx, area.y + ry, rw, rh)
            })
            .collect()
    }

    /// The divider between the master and the stack, for any tile of the last compute: dragging the pointer along the master's axis
    /// moves it by as far, whichever side the master is on. With no master or no stack there is none.
    fn find_split(&mut self, client: &Client, _point: Point) -> Option<Split> {
        let (nodes, master, span) = self.split.last.as_ref()?;
        if !nodes.contains(&client.window().id().0) || *span <= 0.0 || *master == 0 || *master >= nodes.len() {
            return None;
        }
        let sideways = matches!(self.orientation, Orientation::Top | Orientation::Bottom);
        // The master grows as the pointer moves away from the edge it sits on.
        let direction = if matches!(self.orientation, Orientation::Left | Orientation::Top) {
            1.0
        } else {
            -1.0
        };
        let divider = Divider {
            axis: if sideways { SplitAxis::Y } else { SplitAxis::X },
            span: *span,
            direction,
            min: MIN_RATIO,
            max: MAX_RATIO,
            ratio: clamp(self.ratio, MIN_RATIO, MAX_RATIO),
        };
        self.split.token += 1;
        self.split.drag = Some((self.split.token, self.ratio));
        Some(Split {
            token: self.split.token,
            divider,
            cross: None,
        })
    }

    fn set_split(&mut self, token: u64, ratio: f64, _cross_ratio: Option<f64>) {
        if self.split.drag.is_some_and(|(held, _)| held == token) {
            self.ratio = ratio;
            self.split.revision += 1;
        }
    }

    fn reset_split(&mut self, token: u64) {
        if let Some((held, original)) = self.split.drag
            && held == token
        {
            self.ratio = original;
            self.split.revision += 1;
        }
    }

    fn revision(&self) -> u64 {
        self.split.revision
    }
}

/// How a grid fills its cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridFill {
    /// Left to right, then the next row (the default).
    #[default]
    Row,
    /// Top to bottom, then the next column.
    Column,
}

/// The number of columns of a grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Columns {
    /// As square as the number of windows allows (the default).
    #[default]
    Auto,
    /// This many, at least 1.
    Fixed(usize),
}

/// Every window the same size, in rows and columns.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GridLayout {
    columns: Columns,
    fill: GridFill,
    gap: Option<f64>,
}

impl GridLayout {
    /// An automatic grid filled by rows, with the workspace's gaps.
    pub fn new() -> GridLayout {
        GridLayout::default()
    }

    /// The number of columns.
    #[must_use]
    pub fn columns(mut self, columns: Columns) -> GridLayout {
        self.columns = columns;
        self
    }

    /// How the cells fill.
    #[must_use]
    pub fn fill(mut self, fill: GridFill) -> GridLayout {
        self.fill = fill;
        self
    }

    /// The gap between tiles, replacing the workspace's inner gap.
    #[must_use]
    pub fn gap(mut self, gap: f64) -> GridLayout {
        self.gap = Some(gap);
        self
    }

    fn shape(&self, n: usize) -> (usize, usize) {
        match self.columns {
            Columns::Fixed(columns) => {
                let columns = columns.max(1);
                (columns, n.div_ceil(columns))
            }
            Columns::Auto => {
                let side = (n as f64).sqrt().ceil() as usize;
                match self.fill {
                    GridFill::Column => (n.div_ceil(side), side),
                    GridFill::Row => (side, n.div_ceil(side)),
                }
            }
        }
    }
}

impl Layout for GridLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let n = cx.children.len();
        if n == 0 {
            return Vec::new();
        }
        let (columns, rows) = self.shape(n);
        let (area, gap) = geometry(self.gap, cx);
        let cell_width = ((area.width - (columns - 1) as f64 * gap) / columns as f64).max(0.0);
        let cell_height = ((area.height - (rows - 1) as f64 * gap) / rows as f64).max(0.0);
        cx.children
            .iter()
            .enumerate()
            .map(|(i, client)| {
                let (column, row) = match self.fill {
                    GridFill::Column => (i / rows, i % rows),
                    GridFill::Row => (i % columns, i / columns),
                };
                Placement::tile(
                    client,
                    area.x + column as f64 * (cell_width + gap),
                    area.y + row as f64 * (cell_height + gap),
                    cell_width,
                    cell_height,
                )
            })
            .collect()
    }
}

/// One window fills the area and the rest are hidden: the one that has the keyboard, else the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonocleLayout;

impl MonocleLayout {
    /// A monocle.
    pub fn new() -> MonocleLayout {
        MonocleLayout
    }
}

impl Layout for MonocleLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let Some(first) = cx.children.first() else {
            return Vec::new();
        };
        let (area, _) = geometry(None, cx);
        let shown = cx
            .focused
            .as_ref()
            .filter(|focused| {
                cx.children
                    .iter()
                    .any(|child| child.window().id() == focused.window().id())
            })
            .unwrap_or(first)
            .window()
            .id();
        cx.children
            .iter()
            .map(|client| {
                let placement = Placement::tile(client, area.x, area.y, area.width, area.height);
                if client.window().id() == shown {
                    placement
                } else {
                    placement.hidden()
                }
            })
            .collect()
    }
}

/// The kiosk layout of the TypeScript library is a monocle by another name: one window fills the area and the rest are hidden.
pub type KioskLayout = MonocleLayout;
