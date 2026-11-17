//! The library's tiling layouts: [`MasterStackLayout`], [`GridLayout`], [`MonocleLayout`], [`DwindleLayout`], [`StripLayout`],
//! [`ArcLayout`] and [`Carousel3dLayout`].
//!
//! None of them has an algorithm here. Each is a typed value that names a built-in kind of `libgnoblin-std` and the options to give it
//! (`gnoblin_std_abi::layouts::LAYOUT_KINDS` declares both), so a workspace lays windows out the same way whichever language the shell
//! is written in: the library does it. A layout is given to a workspace like any other, and the actions of a kind are methods that
//! name the workspace they run on:
//!
//! ```ignore
//! workspace.set_layout(DwindleLayout::with_options(DwindleOptions { preserve_split: true, ..DwindleOptions::default() })?)?;
//! g.bind("Super+T", { let w = workspace.clone(); move || { DwindleLayout::new().toggle_split(&w).ok(); } })?;
//! ```
//!
//! A layout of your own is a [`Layout`](crate::Layout); it runs through the same table as the library's kinds.

use gnoblin_std_abi as abi;

use crate::bindings::Point;
use crate::builtin::{BuiltinLayout, LayoutValue, answer_flag};
use crate::error::Result;
use crate::layout::{IntoLayout, LayoutSpec};
use crate::workspace::Workspace;

pub use crate::builtin::{RingOptions, RingSlot};

fn builtin(kind: u32, options: Vec<(&'static str, LayoutValue)>) -> LayoutSpec {
    LayoutSpec::Builtin(BuiltinLayout { kind, options })
}

fn some(options: &mut Vec<(&'static str, LayoutValue)>, name: &'static str, value: Option<f64>) {
    if let Some(value) = value {
        options.push((name, LayoutValue::Number(value)));
    }
}

/// Where the master area sits in a master-stack layout.
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

/// One master area and a stack: the first `master_count` windows share the master area, the rest share the stack. The divider between
/// them can be dragged ([`SplitResizeBinding`](crate::SplitResizeBinding)).
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
}

impl Default for MasterStackLayout {
    fn default() -> MasterStackLayout {
        MasterStackLayout {
            ratio: 0.5,
            master_count: 1,
            gap: None,
            orientation: Orientation::Left,
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

impl IntoLayout for MasterStackLayout {
    fn into_layout(self) -> LayoutSpec {
        let mut options = vec![
            ("ratio", LayoutValue::Number(self.ratio)),
            ("master_count", LayoutValue::Number(self.master_count as f64)),
            ("orientation", LayoutValue::Enum(self.orientation as u32)),
        ];
        some(&mut options, "gap", self.gap);
        builtin(abi::STD_BUILTIN_MASTER_STACK, options)
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
}

impl IntoLayout for GridLayout {
    fn into_layout(self) -> LayoutSpec {
        let columns = match self.columns {
            Columns::Auto => 0.0,
            Columns::Fixed(count) => count.max(1) as f64,
        };
        let mut options = vec![
            ("columns", LayoutValue::Number(columns)),
            ("fill", LayoutValue::Enum(self.fill as u32)),
        ];
        some(&mut options, "gap", self.gap);
        builtin(abi::STD_BUILTIN_GRID, options)
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

impl IntoLayout for MonocleLayout {
    fn into_layout(self) -> LayoutSpec {
        builtin(abi::STD_BUILTIN_MONOCLE, Vec::new())
    }
}

/// The kiosk layout of the TypeScript library is a monocle by another name: one window fills the area and the rest are hidden.
pub type KioskLayout = MonocleLayout;

/// How a split picks its direction. `Aspect` follows the shape of the cell, as Hyprland's dwindle does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitMode {
    /// The shape of the cell, every compute.
    #[default]
    Aspect,
    /// Side by side, for the splits this layout makes.
    Horizontal,
    /// One above the other, for the splits this layout makes.
    Vertical,
}

/// Where [`DwindleLayout::preselect`] puts the next window, relative to the window it splits. `None` clears the choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preselect {
    /// To the left of it.
    Left,
    /// To the right of it.
    Right,
    /// Above it.
    Up,
    /// Below it.
    Down,
    /// No choice.
    #[default]
    None,
}

impl Preselect {
    fn from_member(member: u32) -> Preselect {
        match member {
            0 => Preselect::Left,
            1 => Preselect::Right,
            2 => Preselect::Up,
            3 => Preselect::Down,
            _ => Preselect::None,
        }
    }
}

/// Where a new window goes in the cell it splits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForceSplit {
    /// On the side of the cell the pointer is on, and second (right or below) when there is no pointer.
    #[default]
    ByPointer,
    /// First (left or above).
    First,
    /// Second (right or below).
    Second,
}

/// Which window of a new split gets the layout's ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitBias {
    /// The first one (top or left).
    #[default]
    First,
    /// The window the new one was opened on, wherever it sits.
    Opened,
}

/// Where a dwindle reads the pointer from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PointerSource {
    /// There is no pointer: the options that read it do nothing (the default).
    #[default]
    None,
    /// The seat's pointer.
    Seat,
    /// This point, which [`DwindleLayout::set_pointer`] may move.
    Fixed(Point),
}

/// The options of a [`DwindleLayout`]. The names are Hyprland's `dwindle` options, with their defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DwindleOptions {
    /// Split direction for each new window. Default [`SplitMode::Aspect`].
    pub split: SplitMode,
    /// The first child's share of every split that was not dragged or set, held to 0.1 to 0.9. Default 0.5.
    pub ratio: f64,
    /// Overrides the workspace's inner gap.
    pub gap: Option<f64>,
    /// Where a new window goes in the cell it splits. Default [`ForceSplit::ByPointer`].
    pub force_split: ForceSplit,
    /// Keeps each split's direction as it was made instead of following the shape of its cell. Default false.
    pub preserve_split: bool,
    /// Chooses the direction and side of a new window by the pointer's place in the cell it splits (the cell's diagonals make four
    /// triangles). Implies `preserve_split`. With no pointer it does nothing. Default false.
    pub smart_split: bool,
    /// Which divider a split-resize drag moves. True: the divider nearest the press. False: both dividers that bound the window by
    /// its place in the tree. Default true.
    pub smart_resizing: bool,
    /// Keeps a [`preselect`](DwindleLayout::preselect) choice for every new window until it is cleared, instead of once. Default false.
    pub permanent_direction_override: bool,
    /// A split goes side by side when the cell's width is at least its height times this, held to 0.1 to 3.0. Default 1.
    pub split_width_multiplier: f64,
    /// True: a new window splits the focused window. False: it splits the window under the pointer, or the first window when there
    /// is no pointer. Default true.
    pub use_active_for_splits: bool,
    /// Which window of a new split gets `ratio`. Default [`SplitBias::First`].
    pub split_bias: SplitBias,
    /// Where the pointer is read from. `force_split`, `smart_split` and `use_active_for_splits` read it when a window arrives.
    /// Default none.
    pub pointer: PointerSource,
}

impl Default for DwindleOptions {
    fn default() -> DwindleOptions {
        DwindleOptions {
            split: SplitMode::Aspect,
            ratio: 0.5,
            gap: None,
            force_split: ForceSplit::ByPointer,
            preserve_split: false,
            smart_split: false,
            smart_resizing: true,
            permanent_direction_override: false,
            split_width_multiplier: 1.0,
            use_active_for_splits: true,
            split_bias: SplitBias::First,
            pointer: PointerSource::None,
        }
    }
}

impl DwindleOptions {
    /// The options as the library names them.
    fn values(&self) -> Vec<(&'static str, LayoutValue)> {
        let mut options = vec![
            ("split", LayoutValue::Enum(self.split as u32)),
            ("ratio", LayoutValue::Number(self.ratio)),
            ("force_split", LayoutValue::Enum(self.force_split as u32)),
            ("preserve_split", LayoutValue::Bool(self.preserve_split)),
            ("smart_split", LayoutValue::Bool(self.smart_split)),
            ("smart_resizing", LayoutValue::Bool(self.smart_resizing)),
            (
                "permanent_direction_override",
                LayoutValue::Bool(self.permanent_direction_override),
            ),
            (
                "split_width_multiplier",
                LayoutValue::Number(self.split_width_multiplier),
            ),
            ("use_active_for_splits", LayoutValue::Bool(self.use_active_for_splits)),
            ("split_bias", LayoutValue::Enum(self.split_bias as u32)),
        ];
        some(&mut options, "gap", self.gap);
        let (source, point) = match self.pointer {
            PointerSource::None => (0, None),
            PointerSource::Seat => (1, None),
            PointerSource::Fixed(point) => (2, Some(point)),
        };
        options.push(("pointer", LayoutValue::Enum(source)));
        some(&mut options, "pointer_x", point.map(|point| point.x));
        some(&mut options, "pointer_y", point.map(|point| point.y));
        options
    }
}

/// A dwindle tiling layout: each new window splits the cell it arrives in, as Hyprland's does. The library keeps the split tree in the
/// workspace; this value names the options, and its methods are the actions on the workspace they are given. See
/// `gnoblin_std_abi::layouts` for what each option and action does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DwindleLayout {
    options: DwindleOptions,
}

impl DwindleLayout {
    /// A dwindle layout with Hyprland's defaults.
    pub fn new() -> DwindleLayout {
        DwindleLayout::default()
    }

    /// A dwindle layout with `options`.
    ///
    /// # Errors
    ///
    /// Never: the library checks the options when the layout is given to a workspace.
    pub fn with_options(options: DwindleOptions) -> Result<DwindleLayout> {
        Ok(DwindleLayout { options })
    }

    /// The layout's options.
    pub fn options(&self) -> DwindleOptions {
        self.options
    }

    /// Changes the options of the layout `workspace` has now. The tree is kept, so splits already made keep their direction and ratio.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, an option is out of range, or the generation retired.
    pub fn set_options(workspace: &Workspace, options: DwindleOptions) -> Result<()> {
        for (name, value) in options.values() {
            workspace.set_layout_option(name, value)?;
        }
        Ok(())
    }

    /// Moves the fixed pointer of the workspace's dwindle (see [`PointerSource::Fixed`]).
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, or the generation retired.
    pub fn set_pointer(workspace: &Workspace, point: Point) -> Result<()> {
        workspace.set_layout_option("pointer_x", LayoutValue::Number(point.x))?;
        workspace.set_layout_option("pointer_y", LayoutValue::Number(point.y))
    }

    /// Whether the layout `workspace` has now is a dwindle.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn is_active(workspace: &Workspace) -> Result<bool> {
        Ok(workspace.layout_kind()? == Some(abi::STD_BUILTIN_DWINDLE))
    }

    fn act(&self, workspace: &Workspace, action: &str, args: &[(&str, LayoutValue)]) -> Result<bool> {
        answer_flag(action, workspace.layout_act(action, args)?)
    }

    /// The direction [`preselect`](DwindleLayout::preselect) set for the next window on `workspace`, or [`Preselect::None`].
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, or the generation retired.
    pub fn preselected(&self, workspace: &Workspace) -> Result<Preselect> {
        match workspace.layout_act("preselected", &[])? {
            LayoutValue::Enum(member) => Ok(Preselect::from_member(member)),
            _ => Ok(Preselect::None),
        }
    }

    /// Sets where the next window opens on `workspace`: `Left` or `Up` puts it first (left or above) with a split that goes that way,
    /// `Right` or `Down` second. The choice is used once, unless `permanent_direction_override` is set; `None` clears it.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, or the generation retired.
    pub fn preselect(&self, workspace: &Workspace, direction: Preselect) -> Result<()> {
        workspace
            .layout_act("preselect", &[("direction", LayoutValue::Enum(direction as u32))])
            .map(drop)
    }

    /// Turns the split above the focused window of `workspace` from side by side to top and bottom or back. `false` when no window
    /// with a split above it has focus.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, or the generation retired.
    pub fn toggle_split(&self, workspace: &Workspace) -> Result<bool> {
        self.act(workspace, "toggle_split", &[])
    }

    /// Swaps the two halves of the split above the focused window. `false` when there is none.
    ///
    /// # Errors
    ///
    /// As for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn swap_split(&self, workspace: &Workspace) -> Result<bool> {
        self.act(workspace, "swap_split", &[])
    }

    /// Turns the split above the focused window by `angle` degrees clockwise, a multiple of 90 (negative turns the other way).
    ///
    /// # Errors
    ///
    /// `angle` is not a multiple of 90, or as for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn rotate_split(&self, workspace: &Workspace, angle: f64) -> Result<bool> {
        self.act(workspace, "rotate_split", &[("angle", LayoutValue::Number(angle))])
    }

    /// Makes the focused window a direct child of the root split. `stable` keeps the window on the same side of the screen.
    ///
    /// # Errors
    ///
    /// As for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn move_to_root(&self, workspace: &Workspace, stable: bool) -> Result<bool> {
        self.act(workspace, "move_to_root", &[("stable", LayoutValue::Bool(stable))])
    }

    /// Changes the share of the split above the focused window by `amount` (a fraction), held to 0.1 to 0.9.
    ///
    /// # Errors
    ///
    /// `amount` is not finite, or as for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn split_ratio(&self, workspace: &Workspace, amount: f64) -> Result<bool> {
        self.act(workspace, "split_ratio", &[("amount", LayoutValue::Number(amount))])
    }

    /// Sets the share of the split above the focused window to `ratio`, held to 0.1 to 0.9.
    ///
    /// # Errors
    ///
    /// As for [`split_ratio`](DwindleLayout::split_ratio).
    pub fn set_split_ratio(&self, workspace: &Workspace, ratio: f64) -> Result<bool> {
        self.act(workspace, "set_split_ratio", &[("ratio", LayoutValue::Number(ratio))])
    }

    /// Toggles pseudotiling of the focused window: it keeps its own size (`size`, else the window's own with its frame, else its
    /// cell's), centred in its cell. Whether it is pseudotiled after the call.
    ///
    /// # Errors
    ///
    /// `size` is not two numbers above 0, or as for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn pseudotile(&self, workspace: &Workspace, size: Option<(f64, f64)>) -> Result<bool> {
        match size {
            Some((width, height)) => self.act(
                workspace,
                "pseudotile",
                &[
                    ("width", LayoutValue::Number(width)),
                    ("height", LayoutValue::Number(height)),
                ],
            ),
            None => self.act(workspace, "pseudotile", &[]),
        }
    }

    /// Whether `client` is pseudotiled on `workspace`.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a dwindle, or the generation retired.
    pub fn is_pseudotiled(&self, workspace: &Workspace, client: impl crate::client::AsClient) -> Result<bool> {
        self.act(
            workspace,
            "is_pseudotiled",
            &[("client", LayoutValue::Node(client.client_id().0))],
        )
    }
}

impl IntoLayout for DwindleLayout {
    fn into_layout(self) -> LayoutSpec {
        builtin(abi::STD_BUILTIN_DWINDLE, self.options.values())
    }
}

/// A strip of columns: one per window, with the focus kept in view. A column's width is a fraction of the strip's, held to 0.1 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripLayout {
    column: f64,
}

impl Default for StripLayout {
    fn default() -> StripLayout {
        StripLayout { column: 0.5 }
    }
}

impl StripLayout {
    /// A strip whose columns are half the strip's width.
    pub fn new() -> StripLayout {
        StripLayout::default()
    }

    /// A strip whose columns are `column` of the strip's width, held to 0.1 to 1.
    ///
    /// # Errors
    ///
    /// Never: the library checks the width when the layout is given to a workspace.
    pub fn with_column(column: f64) -> Result<StripLayout> {
        Ok(StripLayout { column })
    }

    /// Whether the layout `workspace` has now is a strip.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn is_active(workspace: &Workspace) -> Result<bool> {
        Ok(workspace.layout_kind()? == Some(abi::STD_BUILTIN_STRIP))
    }

    /// A column's width as a fraction of the strip's.
    pub fn column(&self) -> f64 {
        self.column
    }

    /// Sets the column width of the strip `workspace` has now. The workspace computes again.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a strip, `column` is not finite, or the generation retired.
    pub fn set_column(workspace: &Workspace, column: f64) -> Result<()> {
        workspace.set_layout_option("column", LayoutValue::Number(column))
    }

    /// The view offset in pixels that [`scroll_to`](StripLayout::scroll_to) pinned on `workspace`, or `None` while the view follows
    /// the focus.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a strip, or the generation retired.
    pub fn pin(&self, workspace: &Workspace) -> Result<Option<f64>> {
        Ok(workspace.layout_act("pin", &[])?.number())
    }

    /// Scrolls the view to `offset` pixels and keeps it there until the focused window changes.
    ///
    /// # Errors
    ///
    /// `offset` is not a finite number, the workspace's layout is not a strip, or the generation retired.
    pub fn scroll_to(&self, workspace: &Workspace, offset: f64) -> Result<()> {
        workspace
            .layout_act("scroll_to", &[("offset", LayoutValue::Number(offset))])
            .map(drop)
    }

    /// Moves the keyboard one column left; at the first column nothing happens.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a strip, or the generation retired.
    pub fn focus_left(&self, workspace: &Workspace) -> Result<()> {
        workspace.layout_act("focus_left", &[]).map(drop)
    }

    /// Moves the keyboard one column right; at the last column nothing happens.
    ///
    /// # Errors
    ///
    /// As for [`focus_left`](StripLayout::focus_left).
    pub fn focus_right(&self, workspace: &Workspace) -> Result<()> {
        workspace.layout_act("focus_right", &[]).map(drop)
    }
}

impl IntoLayout for StripLayout {
    fn into_layout(self) -> LayoutSpec {
        builtin(
            abi::STD_BUILTIN_STRIP,
            vec![("column", LayoutValue::Number(self.column))],
        )
    }
}

/// Windows around the viewer on an arc, the focused window at the front.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcLayout {
    radius: f64,
    spacing: f64,
}

impl Default for ArcLayout {
    fn default() -> ArcLayout {
        ArcLayout {
            radius: 1200.0,
            spacing: 0.35,
        }
    }
}

impl ArcLayout {
    /// An arc of radius 1200 with 0.35 radians between neighbours.
    pub fn new() -> ArcLayout {
        ArcLayout::default()
    }

    /// An arc of `radius` scene units with `spacing` radians between neighbours.
    ///
    /// # Errors
    ///
    /// Never: the library checks the numbers when the layout is given to a workspace.
    pub fn with(radius: f64, spacing: f64) -> Result<ArcLayout> {
        Ok(ArcLayout { radius, spacing })
    }

    /// The distance from the centre of the arc to each window, in scene units.
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// The angle between neighbouring windows, in radians.
    pub fn spacing(&self) -> f64 {
        self.spacing
    }
}

impl IntoLayout for ArcLayout {
    fn into_layout(self) -> LayoutSpec {
        builtin(
            abi::STD_BUILTIN_ARC,
            vec![
                ("radius", LayoutValue::Number(self.radius)),
                ("spacing", LayoutValue::Number(self.spacing)),
            ],
        )
    }
}

/// Cards on a ring, the focused window at the front; with no focus among the windows the first is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Carousel3dLayout {
    options: RingOptions,
}

impl Carousel3dLayout {
    /// A carousel with a 40 degree field of view and cards 0.36 of the area wide and 0.6 high.
    pub fn new() -> Carousel3dLayout {
        Carousel3dLayout::default()
    }

    /// A carousel with `options`.
    pub fn with_options(options: RingOptions) -> Carousel3dLayout {
        Carousel3dLayout { options }
    }

    /// The ring's options.
    pub fn options(&self) -> RingOptions {
        self.options
    }

    /// Moves the keyboard `steps` cards round the ring of `workspace` (its windows in their order), wrapping at both ends. Nothing
    /// happens while no window has the keyboard.
    ///
    /// # Errors
    ///
    /// The workspace's layout is not a carousel, or the generation retired.
    pub fn rotate(&self, workspace: &Workspace, steps: i64) -> Result<()> {
        workspace
            .layout_act("rotate", &[("steps", LayoutValue::Number(steps as f64))])
            .map(drop)
    }
}

impl IntoLayout for Carousel3dLayout {
    fn into_layout(self) -> LayoutSpec {
        builtin(abi::STD_BUILTIN_CAROUSEL, self.options.options())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_std_abi::layouts::{Initial, LAYOUT_KINDS};

    /// What a typed layout sends must be what the table declares: every name exists, and a default layout sends the table's defaults.
    fn check(spec: LayoutSpec) {
        let LayoutSpec::Builtin(layout) = spec else {
            panic!("a built-in layout");
        };
        let kind = LAYOUT_KINDS
            .iter()
            .find(|kind| kind.id == layout.kind())
            .expect("a declared kind");
        for (name, value) in layout.options() {
            let desc = kind
                .option(name)
                .unwrap_or_else(|| panic!("{}.{name} is not declared", kind.name));
            let declared = match desc.default {
                Initial::Bool(flag) => Some(LayoutValue::Bool(flag)),
                Initial::Number(number) => match desc.value.ty {
                    abi::layouts::ValueShape::Enum(_) => Some(LayoutValue::Enum(number as u32)),
                    _ => Some(LayoutValue::Number(number)),
                },
                Initial::None => None,
            };
            if let Some(declared) = declared {
                assert_eq!(
                    *value, declared,
                    "{}.{name} differs from the table's default",
                    kind.name
                );
            }
        }
    }

    #[test]
    fn a_default_typed_layout_sends_the_defaults_the_table_declares() {
        check(MasterStackLayout::new().into_layout());
        check(GridLayout::new().into_layout());
        check(MonocleLayout::new().into_layout());
        check(DwindleLayout::new().into_layout());
        check(StripLayout::new().into_layout());
        check(ArcLayout::new().into_layout());
        check(Carousel3dLayout::new().into_layout());
    }

    #[test]
    fn every_option_a_dwindle_sets_is_declared() {
        let kind = LAYOUT_KINDS
            .iter()
            .find(|kind| kind.id == abi::STD_BUILTIN_DWINDLE)
            .expect("the dwindle is declared");
        let options = DwindleOptions {
            gap: Some(4.0),
            pointer: PointerSource::Fixed(Point { x: 1.0, y: 2.0 }),
            ..DwindleOptions::default()
        };
        for (name, _) in options.values() {
            assert!(kind.option(name).is_some(), "{name}");
        }
        assert_eq!(options.values().len(), kind.options.len());
    }
}
