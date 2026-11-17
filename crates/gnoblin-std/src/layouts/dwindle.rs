//! [`DwindleLayout`]: a binary split tree that places windows as Hyprland's dwindle layout does.
//!
//! A port of `data/js/gnoblin-std/layouts/dwindle.ts`, which is the reference: a new window splits the cell of the focused window (the
//! last window's cell when none has focus) in two. A split's direction is not fixed: every compute reads the cell the split divides,
//! wider than tall puts the halves side by side, taller than wide one above the other. `preserve_split` (or `smart_split`) keeps the
//! direction a split had when it was made, as does `split` when it is not `Aspect` and a split made by [`preselect`]. The tree persists
//! between computes: removing a window collapses its split, adding one inserts a leaf.
//!
//! Ratios are fractions of the first child, 0.1 to 0.9. Hyprland's `splitRatio` and `default_split_ratio` are twice that: its 1.0 is
//! 0.5 here.
//!
//! The layout is a handle (cheap to clone, the clones share one tree), because the actions that change the tree
//! ([`toggle_split`], [`swap_split`], [`rotate_split`], [`preselect`], [`move_to_root`], [`split_ratio`], [`pseudotile`]) are called
//! by a shell's key bindings after the layout has been given to a workspace:
//!
//! ```ignore
//! let dwindle = DwindleLayout::new();
//! workspace.set_layout(dwindle.clone())?;
//! g.bind("Super+T", { let (d, w) = (dwindle.clone(), workspace.clone()); move || { d.toggle_split(&w).ok(); } })?;
//! ```
//!
//! [`preselect`]: DwindleLayout::preselect
//! [`toggle_split`]: DwindleLayout::toggle_split
//! [`swap_split`]: DwindleLayout::swap_split
//! [`rotate_split`]: DwindleLayout::rotate_split
//! [`move_to_root`]: DwindleLayout::move_to_root
//! [`split_ratio`]: DwindleLayout::split_ratio
//! [`pseudotile`]: DwindleLayout::pseudotile

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::Rect;

use crate::bindings::Point;
use crate::client::{AsClient, Client};
use crate::error::{Error, ErrorKind, Result};
use crate::layout::{Layout, LayoutContext, Placement};
use crate::layouts::{clamp, geometry};
use crate::split::{Divider, Split, SplitAxis};
use crate::workspace::Workspace;

const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;
const MIN_MULTIPLIER: f64 = 0.1;
const MAX_MULTIPLIER: f64 = 3.0;

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

/// Where the pointer is, or `None`: the layout has no pointer of its own, so a shell gives it one.
pub type PointerSource = Rc<dyn Fn() -> Option<Point>>;

/// The options of a [`DwindleLayout`]. The names are Hyprland's `dwindle` options, with their defaults.
#[derive(Clone)]
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
    /// Where the pointer is, in the layout's coordinates. `force_split`, `smart_split` and `use_active_for_splits` read it when a
    /// window arrives. Default none.
    pub pointer: Option<PointerSource>,
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
            pointer: None,
        }
    }
}

impl std::fmt::Debug for DwindleOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DwindleOptions")
            .field("split", &self.split)
            .field("ratio", &self.ratio)
            .field("gap", &self.gap)
            .field("force_split", &self.force_split)
            .field("preserve_split", &self.preserve_split)
            .field("smart_split", &self.smart_split)
            .field("smart_resizing", &self.smart_resizing)
            .field("permanent_direction_override", &self.permanent_direction_override)
            .field("split_width_multiplier", &self.split_width_multiplier)
            .field("use_active_for_splits", &self.use_active_for_splits)
            .field("split_bias", &self.split_bias)
            .field("pointer", &self.pointer.as_ref().map(|_| "fn"))
            .finish()
    }
}

impl DwindleOptions {
    fn check(&self) -> Result<()> {
        let bad = |message: &str| Err(Error::new(ErrorKind::Type, message));
        if !self.ratio.is_finite() {
            return bad("DwindleLayout ratio must be a finite number");
        }
        if !self.split_width_multiplier.is_finite() {
            return bad("DwindleLayout splitWidthMultiplier must be a finite number");
        }
        if let Some(gap) = self.gap
            && !(gap.is_finite() && gap >= 0.0)
        {
            return Err(Error::new(
                ErrorKind::Type,
                format!("DwindleLayout gap must be a finite number of at least 0, got {gap}"),
            ));
        }
        Ok(())
    }
}

// ---- the tree --------------------------------------------------------------------------------------------------------------

/// A place in the tree's arena.
type Ix = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    /// `a` is left of `b`.
    H,
    /// `a` is above `b`.
    V,
}

#[derive(Clone, Debug)]
enum TreeKind {
    Leaf(u64),
    Split(SplitNode),
}

#[derive(Clone, Debug)]
struct SplitNode {
    /// The direction. A split that follows its cell has it set again by every compute.
    dir: Dir,
    /// The direction does not follow the shape of the cell: a `split` option, a preselection.
    fixed: bool,
    /// The layout's ratio is turned over for this split: `split_bias` gave it to the window the new one was opened on.
    biased: bool,
    /// The first child's share a drag or `split_ratio` set for this split alone.
    ratio: Option<f64>,
    a: Ix,
    b: Ix,
}

#[derive(Clone, Debug)]
struct ArenaNode {
    kind: TreeKind,
    parent: Option<Ix>,
}

/// What the cells of a compute depend on besides the tree.
#[derive(Clone, Copy, Debug)]
struct Shape {
    gap: f64,
    /// The layout's ratio, held to its range.
    ratio: f64,
    /// No split follows its cell: `preserve_split` or `smart_split`.
    preserve: bool,
    /// `split_width_multiplier`, held to its range.
    multiplier: f64,
}

/// The share of the first child of `split` when it has no ratio of its own.
fn inherited_share(split: &SplitNode, ratio: f64) -> f64 {
    if split.biased { 1.0 - ratio } else { ratio }
}

/// The squared distance from `point` to `rect`; 0 inside it.
fn distance(rect: &Rect, point: Point) -> f64 {
    let dx = (rect.x - point.x).max(0.0).max(point.x - (rect.x + rect.width));
    let dy = (rect.y - point.y).max(0.0).max(point.y - (rect.y + rect.height));
    dx * dx + dy * dy
}

/// The window whose cell holds `point`, else the one whose cell is nearest it; the first of equals.
fn nearest(rects: &[(u64, Rect)], point: Point) -> Option<u64> {
    let mut best: Option<(u64, f64)> = None;
    for (client, rect) in rects {
        let d = distance(rect, point);
        if best.is_none_or(|(_, least)| d < least) {
            best = Some((*client, d));
        }
    }
    best.map(|(client, _)| client)
}

/// The drag that `find_split` last named: the splits it moves and the ratios they had.
#[derive(Clone, Copy, Debug)]
struct DividerDrag {
    token: u64,
    main: (Ix, Option<f64>),
    cross: Option<(Ix, Option<f64>)>,
}

/// The tree and everything the actions need, over window ids.
struct DwindleCore {
    options: DwindleOptions,
    nodes: Vec<Option<ArenaNode>>,
    free: Vec<Ix>,
    root: Option<Ix>,
    /// The leaves in the order they were added (the order a first-of-equals tie is broken in).
    leaves: Vec<(u64, Ix)>,
    /// Pseudotiled windows and the size each keeps; `None` until the next compute gives it its cell's size.
    pseudo: HashMap<u64, Option<(f64, f64)>>,
    preselect: Preselect,
    /// The focus the previous compute saw: the window a new one splits, because the new one takes focus on arrival.
    focus: Option<u64>,
    geometry: Option<(Rect, Shape)>,
    drag: Option<DividerDrag>,
    next_token: u64,
    /// The workspace the layout computed for last: the one an action asks to compute again.
    workspace: Option<Workspace>,
    /// A change to it is what makes the workspace compute again.
    revision: u64,
}

impl DwindleCore {
    fn new(options: DwindleOptions) -> DwindleCore {
        DwindleCore {
            options,
            nodes: Vec::new(),
            free: Vec::new(),
            root: None,
            leaves: Vec::new(),
            pseudo: HashMap::new(),
            preselect: Preselect::None,
            focus: None,
            geometry: None,
            drag: None,
            next_token: 0,
            workspace: None,
            revision: 0,
        }
    }

    fn node(&self, at: Ix) -> &ArenaNode {
        self.nodes[at].as_ref().expect("a live node of the tree")
    }

    fn node_mut(&mut self, at: Ix) -> &mut ArenaNode {
        self.nodes[at].as_mut().expect("a live node of the tree")
    }

    fn split(&self, at: Ix) -> &SplitNode {
        match &self.node(at).kind {
            TreeKind::Split(split) => split,
            TreeKind::Leaf(_) => unreachable!("a split was expected at this place of the tree"),
        }
    }

    fn split_mut(&mut self, at: Ix) -> &mut SplitNode {
        match &mut self.node_mut(at).kind {
            TreeKind::Split(split) => split,
            TreeKind::Leaf(_) => unreachable!("a split was expected at this place of the tree"),
        }
    }

    fn alloc(&mut self, node: ArenaNode) -> Ix {
        if let Some(at) = self.free.pop() {
            self.nodes[at] = Some(node);
            at
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    fn release(&mut self, at: Ix) {
        self.nodes[at] = None;
        self.free.push(at);
        // A drag that names a place that is gone has nothing to move.
        if self
            .drag
            .is_some_and(|drag| drag.main.0 == at || drag.cross.is_some_and(|cross| cross.0 == at))
        {
            self.drag = None;
        }
    }

    fn leaf_of(&self, client: u64) -> Option<Ix> {
        self.leaves.iter().find(|(id, _)| *id == client).map(|(_, at)| *at)
    }

    fn set_leaf(&mut self, client: u64, at: Ix) {
        if let Some(entry) = self.leaves.iter_mut().find(|(id, _)| *id == client) {
            entry.1 = at;
        }
    }

    fn parent_of(&self, at: Ix) -> Option<Ix> {
        self.node(at).parent
    }

    fn bump(&mut self) {
        self.revision += 1;
    }

    fn shape(&self, gap: f64) -> Shape {
        Shape {
            gap,
            ratio: clamp(self.options.ratio, MIN_RATIO, MAX_RATIO),
            preserve: self.options.preserve_split || self.options.smart_split,
            multiplier: clamp(self.options.split_width_multiplier, MIN_MULTIPLIER, MAX_MULTIPLIER),
        }
    }

    /// The rectangle of every leaf under `root` inside `area`, in the order a depth-first walk reaches them (the first child first);
    /// with `regions`, the rectangle each split divides as well. A split that follows its cell takes its direction from the cell here:
    /// top and bottom when the height times the multiplier is above the width, side by side otherwise.
    fn cells(
        &mut self,
        root: Ix,
        area: Rect,
        shape: &Shape,
        mut regions: Option<&mut HashMap<Ix, Rect>>,
    ) -> Vec<(u64, Rect)> {
        let gap = shape.gap;
        let mut out = Vec::new();
        let mut pending = vec![(root, area)];
        while let Some((at, rect)) = pending.pop() {
            let mut split = match &self.node(at).kind {
                TreeKind::Leaf(client) => {
                    out.push((*client, rect));
                    continue;
                }
                TreeKind::Split(split) => split.clone(),
            };
            if !shape.preserve && !split.fixed {
                split.dir = if rect.height * shape.multiplier > rect.width {
                    Dir::V
                } else {
                    Dir::H
                };
                self.split_mut(at).dir = split.dir;
            }
            if let Some(regions) = regions.as_deref_mut() {
                regions.insert(at, rect);
            }
            let share = clamp(
                split.ratio.unwrap_or_else(|| inherited_share(&split, shape.ratio)),
                MIN_RATIO,
                MAX_RATIO,
            );
            if split.dir == Dir::H {
                let first = ((rect.width - gap) * share).max(0.0);
                pending.push((
                    split.b,
                    Rect::new(
                        rect.x + first + gap,
                        rect.y,
                        (rect.width - gap - first).max(0.0),
                        rect.height,
                    ),
                ));
                pending.push((split.a, Rect::new(rect.x, rect.y, first, rect.height)));
            } else {
                let first = ((rect.height - gap) * share).max(0.0);
                pending.push((
                    split.b,
                    Rect::new(
                        rect.x,
                        rect.y + first + gap,
                        rect.width,
                        (rect.height - gap - first).max(0.0),
                    ),
                ));
                pending.push((split.a, Rect::new(rect.x, rect.y, rect.width, first)));
            }
        }
        out
    }

    /// The pointer, or `None` when there is no source for it or it gave something that is not a point.
    fn pointer(&self) -> Option<Point> {
        let at = self.options.pointer.as_ref().and_then(|source| source())?;
        (at.x.is_finite() && at.y.is_finite()).then_some(at)
    }

    /// Computes the cells of `children` and updates the tree: windows that left are removed, windows that arrived are inserted.
    fn compute(&mut self, children: &[u64], focused: Option<u64>, area: Rect, gap: f64) -> Vec<(u64, Rect)> {
        let shape = self.shape(gap);
        self.geometry = Some((area, shape));
        let stale: Vec<u64> = self
            .leaves
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !children.contains(id))
            .collect();
        for client in stale {
            self.remove(client);
        }
        for client in children {
            if self.leaf_of(*client).is_none() {
                self.open(*client, area, &shape);
            }
        }
        match focused {
            Some(client) if children.contains(&client) => self.focus = Some(client),
            _ => {
                if self.focus.is_some_and(|client| !children.contains(&client)) {
                    self.focus = None;
                }
            }
        }
        let Some(root) = self.root else {
            return Vec::new();
        };
        self.cells(root, area, &shape, None)
    }

    /// Puts the new window in the tree, splitting the cell of the window the options choose.
    fn open(&mut self, client: u64, area: Rect, shape: &Shape) {
        let Some(root) = self.root else {
            let at = self.alloc(ArenaNode {
                kind: TreeKind::Leaf(client),
                parent: None,
            });
            self.leaves.push((client, at));
            self.root = Some(at);
            return;
        };
        let rects = self.cells(root, area, shape, None);
        let at = self.pointer();
        let options = self.options.clone();
        let target = if !options.use_active_for_splits {
            match at {
                None => self.leaves.first().map(|(id, _)| *id),
                Some(at) => nearest(&rects, at),
            }
        } else if let Some(focus) = self.focus.filter(|focus| self.leaf_of(*focus).is_some()) {
            Some(focus)
        } else {
            match at {
                None => self.last_leaf(),
                Some(at) => nearest(&rects, at),
            }
        };
        let Some(target) = target else {
            return;
        };
        let Some(cell) = rects.iter().find(|(id, _)| *id == target).map(|(_, rect)| *rect) else {
            return;
        };
        let mult = shape.multiplier;
        let mut dir;
        let mut first = false;
        let mut fixed = false;
        if self.preselect != Preselect::None {
            let choice = self.preselect;
            dir = if matches!(choice, Preselect::Left | Preselect::Right) {
                Dir::H
            } else {
                Dir::V
            };
            first = matches!(choice, Preselect::Left | Preselect::Up);
            fixed = true;
            if !options.permanent_direction_override {
                self.preselect = Preselect::None;
            }
        } else {
            if options.split != SplitMode::Aspect {
                dir = if options.split == SplitMode::Horizontal {
                    Dir::H
                } else {
                    Dir::V
                };
                fixed = true;
            } else if shape.preserve {
                dir = if cell.width > cell.height * mult {
                    Dir::H
                } else {
                    Dir::V
                };
            } else {
                dir = if cell.height * mult > cell.width {
                    Dir::V
                } else {
                    Dir::H
                };
            }
            if options.smart_split
                && let Some(at) = at
            {
                // The cell's diagonals make four triangles; the pointer's triangle gives the direction and the side.
                let dx = at.x - (cell.x + cell.width / 2.0);
                let dy = at.y - (cell.y + cell.height / 2.0);
                if (dy / dx).abs() < cell.height / cell.width {
                    dir = Dir::H;
                    first = dx < 0.0;
                } else {
                    dir = Dir::V;
                    first = dy <= 0.0 || dy.is_nan();
                }
            } else if options.force_split == ForceSplit::First {
                first = true;
            } else if options.force_split == ForceSplit::ByPointer
                && let Some(at) = at
            {
                first = if dir == Dir::H {
                    at.x < cell.x + cell.width / 2.0
                } else {
                    at.y < cell.y + cell.height / 2.0
                };
            }
        }
        self.insert(
            target,
            client,
            dir,
            first,
            fixed,
            options.split_bias == SplitBias::Opened && first,
        );
    }

    /// The last leaf in reading order: the one a new window splits when none had focus.
    fn last_leaf(&self) -> Option<u64> {
        let mut at = self.root?;
        loop {
            match &self.node(at).kind {
                TreeKind::Leaf(client) => return Some(*client),
                TreeKind::Split(split) => at = split.b,
            }
        }
    }

    /// Replaces the leaf of `target` with a split of it and the new leaf of `client`, which is first when `first`.
    fn insert(&mut self, target: u64, client: u64, dir: Dir, first: bool, fixed: bool, biased: bool) {
        let Some(old) = self.leaf_of(target) else {
            return;
        };
        let above = self.parent_of(old);
        let split = self.alloc(ArenaNode {
            kind: TreeKind::Split(SplitNode {
                dir,
                fixed,
                biased,
                ratio: None,
                a: old,
                b: old,
            }),
            parent: above,
        });
        let fresh = self.alloc(ArenaNode {
            kind: TreeKind::Leaf(client),
            parent: Some(split),
        });
        self.leaves.push((client, fresh));
        {
            let node = self.split_mut(split);
            node.a = if first { fresh } else { old };
            node.b = if first { old } else { fresh };
        }
        self.replace(old, split);
        self.node_mut(old).parent = Some(split);
    }

    /// Puts `next` where `old` hangs: in its parent's `a` or `b`, or at the root.
    fn replace(&mut self, old: Ix, next: Ix) {
        match self.parent_of(old) {
            None => self.root = Some(next),
            Some(parent) => {
                let split = self.split_mut(parent);
                if split.a == old {
                    split.a = next;
                } else {
                    split.b = next;
                }
            }
        }
    }

    /// Removes the leaf of `client`; the sibling subtree takes the split's place.
    fn remove(&mut self, client: u64) {
        let Some(leaf) = self.leaf_of(client) else {
            return;
        };
        self.leaves.retain(|(id, _)| *id != client);
        self.pseudo.remove(&client);
        let Some(parent) = self.parent_of(leaf) else {
            self.root = None;
            self.release(leaf);
            return;
        };
        let split = self.split(parent).clone();
        let sibling = if split.a == leaf { split.b } else { split.a };
        let grand = self.parent_of(parent);
        self.node_mut(sibling).parent = grand;
        self.replace(parent, sibling);
        self.release(leaf);
        self.release(parent);
    }

    /// Two windows trade places in the tree without changing the places.
    fn swapped(&mut self, a: u64, b: u64) {
        let (Some(first), Some(second)) = (self.leaf_of(a), self.leaf_of(b)) else {
            // One of them has not been placed yet: the next compute puts it where the order says.
            return;
        };
        self.node_mut(first).kind = TreeKind::Leaf(b);
        self.node_mut(second).kind = TreeKind::Leaf(a);
        self.set_leaf(a, second);
        self.set_leaf(b, first);
    }

    // ---- dividers -----------------------------------------------------------------------------------------------------------

    /// The divider a resize drag that pressed `client` at `point` moves. With `smart_resizing` it is the split of the last compute
    /// whose divider is nearest `point`, among the splits above `client`'s tile; a tie goes to the split nearer the tile. Without it,
    /// the split directly above the tile, and the nearest split above that which goes the other way as its cross.
    fn find_split(&mut self, client: u64, point: Point) -> Option<Split> {
        let (area, shape) = self.geometry?;
        let leaf = self.leaf_of(client)?;
        let root = self.root?;
        let mut regions = HashMap::new();
        self.cells(root, area, &shape, Some(&mut regions));
        let span = |core: &DwindleCore, at: Ix| -> f64 {
            let region = regions[&at];
            (if core.split(at).dir == Dir::H {
                region.width
            } else {
                region.height
            }) - shape.gap
        };
        let divider = |core: &DwindleCore, at: Ix| -> Divider {
            let split = core.split(at);
            Divider {
                axis: if split.dir == Dir::H {
                    SplitAxis::X
                } else {
                    SplitAxis::Y
                },
                span: span(core, at),
                direction: 1.0,
                min: MIN_RATIO,
                max: MAX_RATIO,
                ratio: clamp(
                    split.ratio.unwrap_or_else(|| inherited_share(split, shape.ratio)),
                    MIN_RATIO,
                    MAX_RATIO,
                ),
            }
        };
        let original = |core: &DwindleCore, at: Ix| (at, core.split(at).ratio);
        let (main, cross) = if self.options.smart_resizing {
            let mut best: Option<(Ix, f64)> = None;
            let mut up = self.parent_of(leaf);
            while let Some(at) = up {
                up = self.parent_of(at);
                let Some(region) = regions.get(&at) else {
                    continue;
                };
                let split = self.split(at);
                let along = if split.dir == Dir::H {
                    region.width
                } else {
                    region.height
                };
                let share = clamp(
                    split.ratio.unwrap_or_else(|| inherited_share(split, shape.ratio)),
                    MIN_RATIO,
                    MAX_RATIO,
                );
                let first = ((along - shape.gap) * share).max(0.0);
                let edge = (if split.dir == Dir::H { region.x } else { region.y }) + first + shape.gap / 2.0;
                let away = ((if split.dir == Dir::H { point.x } else { point.y }) - edge).abs();
                if along - shape.gap > 0.0 && best.is_none_or(|(_, least)| away < least) {
                    best = Some((at, away));
                }
            }
            (best?.0, None)
        } else {
            let above = self.parent_of(leaf)?;
            if span(self, above) <= 0.0 {
                return None;
            }
            let mut other = self.parent_of(above);
            while let Some(at) = other {
                if self.split(at).dir != self.split(above).dir {
                    break;
                }
                other = self.parent_of(at);
            }
            let cross = other.filter(|at| span(self, *at) > 0.0);
            (above, cross)
        };
        if span(self, main) <= 0.0 {
            return None;
        }
        self.next_token += 1;
        let token = self.next_token;
        self.drag = Some(DividerDrag {
            token,
            main: original(self, main),
            cross: cross.map(|at| original(self, at)),
        });
        Some(Split {
            token,
            divider: divider(self, main),
            cross: cross.map(|at| divider(self, at)),
        })
    }

    fn set_split(&mut self, token: u64, ratio: f64, cross_ratio: Option<f64>) {
        let Some(drag) = self.drag.filter(|drag| drag.token == token) else {
            return;
        };
        self.split_mut(drag.main.0).ratio = Some(clamp(ratio, MIN_RATIO, MAX_RATIO));
        if let (Some((at, _)), Some(cross)) = (drag.cross, cross_ratio) {
            self.split_mut(at).ratio = Some(clamp(cross, MIN_RATIO, MAX_RATIO));
        }
        self.bump();
    }

    fn reset_split(&mut self, token: u64) {
        let Some(drag) = self.drag.filter(|drag| drag.token == token) else {
            return;
        };
        self.split_mut(drag.main.0).ratio = drag.main.1;
        if let Some((at, original)) = drag.cross {
            self.split_mut(at).ratio = original;
        }
        self.bump();
    }

    // ---- actions ------------------------------------------------------------------------------------------------------------

    /// The split above the leaf of `client`.
    fn split_above(&self, client: Option<u64>) -> Option<Ix> {
        self.parent_of(self.leaf_of(client?)?)
    }

    fn toggle_split(&mut self, client: Option<u64>) -> bool {
        let Some(at) = self.split_above(client) else {
            return false;
        };
        let split = self.split_mut(at);
        split.dir = if split.dir == Dir::H { Dir::V } else { Dir::H };
        self.bump();
        true
    }

    fn swap_split(&mut self, client: Option<u64>) -> bool {
        let Some(at) = self.split_above(client) else {
            return false;
        };
        let split = self.split_mut(at);
        std::mem::swap(&mut split.a, &mut split.b);
        self.bump();
        true
    }

    fn rotate_split(&mut self, client: Option<u64>, turns: i64) -> bool {
        let Some(at) = self.split_above(client) else {
            return false;
        };
        let turns = turns.rem_euclid(4);
        let flip = turns == 1 || turns == 3;
        let split = self.split_mut(at);
        if (turns == 1 && split.dir == Dir::V) || turns == 2 || (turns == 3 && split.dir == Dir::H) {
            std::mem::swap(&mut split.a, &mut split.b);
        }
        if flip {
            split.dir = if split.dir == Dir::H { Dir::V } else { Dir::H };
        }
        self.bump();
        true
    }

    fn move_to_root(&mut self, client: Option<u64>, stable: bool) -> bool {
        let Some(leaf) = client.and_then(|client| self.leaf_of(client)) else {
            return false;
        };
        let Some(above) = self.parent_of(leaf) else {
            return false;
        };
        let Some(mut root) = self.parent_of(above) else {
            return false;
        };
        let mut ancestor = above;
        while let Some(up) = self.parent_of(root) {
            ancestor = root;
            root = up;
        }
        let other = if self.split(root).a == ancestor {
            self.split(root).b
        } else {
            self.split(root).a
        };
        {
            let split = self.split_mut(above);
            if split.a == leaf {
                split.a = other;
            } else {
                split.b = other;
            }
        }
        {
            let split = self.split_mut(root);
            if split.a == other {
                split.a = leaf;
            } else {
                split.b = leaf;
            }
        }
        self.node_mut(other).parent = Some(above);
        self.node_mut(leaf).parent = Some(root);
        if stable {
            let split = self.split_mut(root);
            std::mem::swap(&mut split.a, &mut split.b);
        }
        self.bump();
        true
    }

    fn split_ratio(&mut self, client: Option<u64>, amount: f64, exact: bool) -> bool {
        let (Some(at), Some((_, shape))) = (self.split_above(client), self.geometry) else {
            return false;
        };
        let split = self.split_mut(at);
        let current = split.ratio.unwrap_or_else(|| inherited_share(split, shape.ratio));
        split.ratio = Some(clamp(
            if exact { amount } else { current + amount },
            MIN_RATIO,
            MAX_RATIO,
        ));
        self.bump();
        true
    }

    /// Toggles pseudotiling of `client`: whether it is pseudotiled after the call.
    fn pseudotile(&mut self, client: Option<u64>, size: Option<(f64, f64)>) -> bool {
        let Some(client) = client.filter(|client| self.leaf_of(*client).is_some()) else {
            return false;
        };
        let on = !self.pseudo.contains_key(&client);
        if on {
            self.pseudo.insert(client, size);
        } else {
            self.pseudo.remove(&client);
        }
        self.bump();
        on
    }
}

// ---- the layout ------------------------------------------------------------------------------------------------------------

/// A dwindle tiling layout: each new window splits the cell it arrives in. See the [module](self) documentation.
///
/// A clone is another handle on the same layout: give one to the workspace and keep one for the actions.
#[derive(Clone)]
pub struct DwindleLayout {
    core: Rc<RefCell<DwindleCore>>,
}

impl Default for DwindleLayout {
    fn default() -> DwindleLayout {
        DwindleLayout::new()
    }
}

impl std::fmt::Debug for DwindleLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DwindleLayout")
            .field("options", &self.core.borrow().options)
            .finish_non_exhaustive()
    }
}

/// The id of the window that has the keyboard among `workspace`'s clients.
fn focused_id(workspace: &Workspace) -> Result<Option<u64>> {
    for client in workspace.clients()?.to_vec()? {
        if client.window().is_focused().unwrap_or(false) {
            return Ok(Some(client.window().id().0));
        }
    }
    Ok(None)
}

impl DwindleLayout {
    /// A dwindle layout with Hyprland's defaults.
    pub fn new() -> DwindleLayout {
        DwindleLayout {
            core: Rc::new(RefCell::new(DwindleCore::new(DwindleOptions::default()))),
        }
    }

    /// A dwindle layout with `options`.
    ///
    /// # Errors
    ///
    /// The ratio or the width multiplier is not a finite number, or the gap is negative or not finite.
    pub fn with_options(options: DwindleOptions) -> Result<DwindleLayout> {
        options.check()?;
        Ok(DwindleLayout {
            core: Rc::new(RefCell::new(DwindleCore::new(options))),
        })
    }

    /// The layout's options now.
    pub fn options(&self) -> DwindleOptions {
        self.core.borrow().options.clone()
    }

    /// Replaces the options. The workspace computes again; the tree is kept, so splits already made keep their direction and ratio.
    ///
    /// # Errors
    ///
    /// As for [`with_options`](DwindleLayout::with_options).
    pub fn set_options(&self, options: DwindleOptions) -> Result<()> {
        options.check()?;
        let workspace = {
            let mut core = self.core.borrow_mut();
            core.options = options;
            core.bump();
            core.workspace.clone()
        };
        if let Some(workspace) = workspace {
            workspace.invalidate()?;
        }
        Ok(())
    }

    /// Tells `workspace` to compute again when an action changed the tree.
    fn changed(workspace: &Workspace, did: bool) -> Result<bool> {
        if did {
            workspace.invalidate()?;
        }
        Ok(did)
    }

    /// The direction [`preselect`](DwindleLayout::preselect) set for the next window, or [`Preselect::None`].
    pub fn preselected(&self) -> Preselect {
        self.core.borrow().preselect
    }

    /// Sets where the next window opens: `Left` or `Up` puts it first (left or above) with a split that goes that way, `Right` or
    /// `Down` second. The split keeps its direction whatever its cell's shape. The choice is used once, unless
    /// `permanent_direction_override` is set; `None` clears it. Nothing is changed in the tree, and the first window of a workspace
    /// does not use it.
    pub fn preselect(&self, direction: Preselect) {
        self.core.borrow_mut().preselect = direction;
    }

    /// Turns the split above the focused window of `workspace` from side by side to top and bottom or back. A split that follows the
    /// shape of its cell takes its direction from the cell again at once, so this needs `preserve_split` (or a split that is fixed),
    /// as in Hyprland. `false` when no window with a split above it has focus.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn toggle_split(&self, workspace: &Workspace) -> Result<bool> {
        let focus = focused_id(workspace)?;
        let did = self.core.borrow_mut().toggle_split(focus);
        DwindleLayout::changed(workspace, did)
    }

    /// Swaps the two halves of the split above the focused window. `false` when there is none.
    ///
    /// # Errors
    ///
    /// As for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn swap_split(&self, workspace: &Workspace) -> Result<bool> {
        let focus = focused_id(workspace)?;
        let did = self.core.borrow_mut().swap_split(focus);
        DwindleLayout::changed(workspace, did)
    }

    /// Turns the split above the focused window by `angle` degrees clockwise, a multiple of 90 (negative turns the other way): the
    /// first half goes from the left to the top to the right to the bottom. 180 swaps the halves. Like
    /// [`toggle_split`](DwindleLayout::toggle_split), the direction part needs `preserve_split`. `false` when there is no split above
    /// the focused window.
    ///
    /// # Errors
    ///
    /// `angle` is not a multiple of 90, the workspace is gone, or the generation retired.
    pub fn rotate_split(&self, workspace: &Workspace, angle: f64) -> Result<bool> {
        if !angle.is_finite() || angle % 90.0 != 0.0 {
            return Err(Error::new(
                ErrorKind::Type,
                format!("DwindleLayout rotateSplit angle must be a multiple of 90, got {angle}"),
            ));
        }
        let focus = focused_id(workspace)?;
        let turns = (angle / 90.0).trunc() as i64;
        let did = self.core.borrow_mut().rotate_split(focus, turns);
        DwindleLayout::changed(workspace, did)
    }

    /// Makes the focused window a direct child of the root split: it takes the place of the root's other half, which goes where the
    /// window was. `stable` keeps the window on the same side of the screen; otherwise it goes to the other. `false` when the window
    /// is alone or already a child of the root.
    ///
    /// # Errors
    ///
    /// As for [`toggle_split`](DwindleLayout::toggle_split).
    pub fn move_to_root(&self, workspace: &Workspace, stable: bool) -> Result<bool> {
        let focus = focused_id(workspace)?;
        let did = self.core.borrow_mut().move_to_root(focus, stable);
        DwindleLayout::changed(workspace, did)
    }

    /// Changes the share of the split above the focused window by `amount` (a fraction, so 0.1 moves the divider a tenth of the
    /// split), held to 0.1 to 0.9. The split keeps the ratio, whatever the layout's `ratio` becomes. `false` when the window has no
    /// split above it.
    ///
    /// # Errors
    ///
    /// `amount` is not finite, the workspace is gone, or the generation retired.
    pub fn split_ratio(&self, workspace: &Workspace, amount: f64) -> Result<bool> {
        self.change_ratio(workspace, amount, false)
    }

    /// Sets the share of the split above the focused window to `ratio`, held to 0.1 to 0.9.
    ///
    /// # Errors
    ///
    /// As for [`split_ratio`](DwindleLayout::split_ratio).
    pub fn set_split_ratio(&self, workspace: &Workspace, ratio: f64) -> Result<bool> {
        self.change_ratio(workspace, ratio, true)
    }

    fn change_ratio(&self, workspace: &Workspace, amount: f64, exact: bool) -> Result<bool> {
        if !amount.is_finite() {
            return Err(Error::new(
                ErrorKind::Type,
                format!("DwindleLayout splitRatio needs a finite number, got {amount}"),
            ));
        }
        let focus = focused_id(workspace)?;
        let did = self.core.borrow_mut().split_ratio(focus, amount, exact);
        DwindleLayout::changed(workspace, did)
    }

    /// Toggles pseudotiling of the focused window: it keeps its own size, centred in its cell and held to the cell's size, instead of
    /// filling the cell. The size is `size`, else the window's own size with its frame, else the size its cell has at the next
    /// compute. Returns whether the window is pseudotiled after the call; `false` also when no window with a tile has focus.
    ///
    /// # Errors
    ///
    /// `size` is not two finite numbers above 0, the workspace is gone, or the generation retired.
    pub fn pseudotile(&self, workspace: &Workspace, size: Option<(f64, f64)>) -> Result<bool> {
        if let Some((width, height)) = size
            && !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0)
        {
            return Err(Error::new(
                ErrorKind::Type,
                format!("DwindleLayout pseudotile size must be finite numbers above 0, got {width} by {height}"),
            ));
        }
        let focus = workspace
            .clients()?
            .to_vec()?
            .into_iter()
            .find(|client| client.window().is_focused().unwrap_or(false));
        let Some(client) = focus else {
            return Ok(false);
        };
        let id = client.window().id().0;
        // The window's own size with its frame is read only when it becomes pseudotiled.
        let pseudotiled = self.core.borrow().pseudo.contains_key(&id);
        let own = match size {
            Some(size) => Some(size),
            None if !pseudotiled => own_size(&client),
            None => None,
        };
        let on = self.core.borrow_mut().pseudotile(Some(id), own);
        workspace.invalidate()?;
        Ok(on)
    }

    /// Whether `client` is pseudotiled.
    pub fn is_pseudotiled(&self, client: impl AsClient) -> bool {
        self.core.borrow().pseudo.contains_key(&client.client_id().0)
    }
}

/// The own size of `client` with its frame, when it has a width and a height; else `None`.
fn own_size(client: &Client) -> Option<(f64, f64)> {
    let bounds = client.bounds().ok()?;
    if !(bounds.width > 0.0 && bounds.height > 0.0) {
        return None;
    }
    let outer = client
        .frame_inset()
        .unwrap_or_default()
        .grow(Rect::new(0.0, 0.0, bounds.width, bounds.height));
    Some((outer.width, outer.height))
}

impl Layout for DwindleLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let (area, gap) = {
            let own = self.core.borrow().options.gap;
            geometry(own, cx)
        };
        let ids: Vec<u64> = cx.children.iter().map(|client| client.window().id().0).collect();
        let focused = cx.focused.as_ref().map(|client| client.window().id().0);
        let mut core = self.core.borrow_mut();
        core.workspace = Some(cx.workspace.clone());
        let rects = core.compute(&ids, focused, area, gap);
        if rects.is_empty() {
            return Vec::new();
        }
        let by_id: HashMap<u64, Rect> = rects.into_iter().collect();
        let mut placements = Vec::with_capacity(cx.children.len());
        for (client, id) in cx.children.iter().zip(&ids) {
            let Some(cell) = by_id.get(id).copied() else {
                continue;
            };
            match core.pseudo.get(id).copied() {
                None => placements.push(Placement::tile(client, cell.x, cell.y, cell.width, cell.height)),
                Some(kept) => {
                    let own = kept.unwrap_or((cell.width, cell.height));
                    core.pseudo.insert(*id, Some(own));
                    let width = own.0.min(cell.width);
                    let height = own.1.min(cell.height);
                    placements.push(Placement::tile(
                        client,
                        cell.x + (cell.width - width) / 2.0,
                        cell.y + (cell.height - height) / 2.0,
                        width,
                        height,
                    ));
                }
            }
        }
        placements
    }

    fn clients_swapped(&mut self, a: &Client, b: &Client) {
        self.core.borrow_mut().swapped(a.window().id().0, b.window().id().0);
    }

    fn revision(&self) -> u64 {
        self.core.borrow().revision
    }

    fn find_split(&mut self, client: &Client, point: Point) -> Option<Split> {
        self.core.borrow_mut().find_split(client.window().id().0, point)
    }

    fn set_split(&mut self, token: u64, ratio: f64, cross_ratio: Option<f64>) {
        self.core.borrow_mut().set_split(token, ratio, cross_ratio);
    }

    fn reset_split(&mut self, token: u64) {
        self.core.borrow_mut().reset_split(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 800.0)
    }

    /// Windows `1..=n` arrive one at a time, each taking the focus as it does; the cells after the last.
    fn arrive(core: &mut DwindleCore, n: u64) -> Vec<(u64, Rect)> {
        let mut out = Vec::new();
        for i in 1..=n {
            let ids: Vec<u64> = (1..=i).collect();
            out = core.compute(&ids, Some(i), area(), 0.0);
        }
        out
    }

    fn text(cells: &[(u64, Rect)]) -> String {
        let mut sorted = cells.to_vec();
        sorted.sort_by_key(|(id, _)| *id);
        sorted
            .iter()
            .map(|(_, r)| format!("{},{},{},{}", r.x, r.y, r.width, r.height))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn windows_dwindle_into_the_corner_by_the_shape_of_their_cell() {
        let mut core = DwindleCore::new(DwindleOptions::default());
        assert_eq!(text(&arrive(&mut core, 1)), "0,0,1000,800");
        // 1000 by 800 is wider than tall: side by side.
        assert_eq!(text(&arrive(&mut core, 2)), "0,0,500,800 500,0,500,800");
        // The second window's cell, 500 by 800, is taller than wide: one above the other.
        assert_eq!(text(&arrive(&mut core, 3)), "0,0,500,800 500,0,500,400 500,400,500,400");
        assert_eq!(
            text(&arrive(&mut core, 4)),
            "0,0,500,800 500,0,500,400 500,400,250,400 750,400,250,400"
        );
    }

    #[test]
    fn a_window_that_leaves_gives_its_cell_to_its_sibling() {
        let mut core = DwindleCore::new(DwindleOptions::default());
        arrive(&mut core, 3);
        let cells = core.compute(&[1, 3], Some(1), area(), 0.0);
        assert_eq!(text(&cells), "0,0,500,800 500,0,500,800");
        let cells = core.compute(&[3], Some(3), area(), 0.0);
        assert_eq!(text(&cells), "0,0,1000,800");
    }

    #[test]
    fn preselect_chooses_the_side_once_and_fixes_the_direction() {
        let mut core = DwindleCore::new(DwindleOptions::default());
        arrive(&mut core, 1);
        core.preselect = Preselect::Up;
        // Above, though the 1000 by 800 cell would split side by side, and the new window is first.
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "0,400,1000,400 0,0,1000,400"
        );
        assert_eq!(core.preselect, Preselect::None);
    }

    #[test]
    fn a_drag_moves_the_nearest_divider_and_a_cancel_puts_it_back() {
        let mut core = DwindleCore::new(DwindleOptions::default());
        arrive(&mut core, 2);
        let split = core.find_split(1, Point { x: 505.0, y: 100.0 }).expect("a divider");
        assert_eq!(split.divider.axis, SplitAxis::X);
        assert_eq!(split.divider.span, 1000.0);
        assert_eq!(split.divider.ratio, 0.5);
        core.set_split(split.token, 0.7, None);
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "0,0,700,800 700,0,300,800"
        );
        core.reset_split(split.token);
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "0,0,500,800 500,0,500,800"
        );
    }

    #[test]
    fn the_tree_turns_over_when_the_split_above_the_focus_is_rotated_or_swapped() {
        let mut core = DwindleCore::new(DwindleOptions {
            preserve_split: true,
            ..DwindleOptions::default()
        });
        arrive(&mut core, 2);
        assert!(core.swap_split(Some(2)));
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "500,0,500,800 0,0,500,800"
        );
        assert!(core.toggle_split(Some(2)));
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "0,400,1000,400 0,0,1000,400"
        );
        assert!(!core.toggle_split(None));
    }
}
