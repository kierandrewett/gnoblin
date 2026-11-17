//! The dwindle: a binary split tree that places windows as Hyprland's dwindle layout does.
//!
//! A port of `data/js/gnoblin-std/layouts/dwindle.ts`, which is the reference: a new window splits the cell of the focused window (the
//! last window's cell when none has focus) in two. A split's direction is not fixed: every compute reads the cell the split divides,
//! wider than tall puts the halves side by side, taller than wide one above the other. `preserve_split` (or `smart_split`) keeps the
//! direction a split had when it was made, as does `split` when it is not `aspect` and a split made by a preselection. The tree
//! persists between computes: removing a window collapses its split, adding one inserts a leaf.
//!
//! Ratios are fractions of the first child, 0.1 to 0.9. Hyprland's `splitRatio` and `default_split_ratio` are twice that: its 1.0 is
//! 0.5 here.

use std::collections::HashMap;

use gnoblin_core::Rect;

use super::{ActionEnv, ActionResult, DividerSpec, Imp, OptionSet, Pt, SplitSpec, Val, clamp, geometry, tile};
use crate::error::{Fail, Res};
use crate::layout::{LayoutContext, Placement};
use crate::state::NodeId;

const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;
const MIN_MULTIPLIER: f64 = 0.1;
const MAX_MULTIPLIER: f64 = 3.0;

/// How a split picks its direction (`SPLIT_MODE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SplitDirMode {
    #[default]
    Aspect,
    Horizontal,
    Vertical,
}

/// Where a preselection puts the next window (`PRESELECT`, `None` clears it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PreselectDir {
    Left,
    Right,
    Up,
    Down,
    #[default]
    None,
}

impl PreselectDir {
    fn from_number(number: u32) -> PreselectDir {
        match number {
            0 => PreselectDir::Left,
            1 => PreselectDir::Right,
            2 => PreselectDir::Up,
            3 => PreselectDir::Down,
            _ => PreselectDir::None,
        }
    }

    fn number(self) -> f64 {
        match self {
            PreselectDir::Left => 0.0,
            PreselectDir::Right => 1.0,
            PreselectDir::Up => 2.0,
            PreselectDir::Down => 3.0,
            PreselectDir::None => 4.0,
        }
    }
}

/// Where a new window goes in the cell it splits (`FORCE_SPLIT`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ForceSide {
    #[default]
    ByPointer,
    First,
    Second,
}

/// The options of the layout, as the algorithm reads them.
#[derive(Clone, Copy, Debug)]
struct DwindleSettings {
    split: SplitDirMode,
    ratio: f64,
    gap: Option<f64>,
    force_split: ForceSide,
    preserve_split: bool,
    smart_split: bool,
    smart_resizing: bool,
    permanent_direction_override: bool,
    split_width_multiplier: f64,
    use_active_for_splits: bool,
    /// `split_bias` is "opened".
    bias_opened: bool,
}

impl DwindleSettings {
    fn read(opts: &OptionSet) -> DwindleSettings {
        DwindleSettings {
            split: match opts.member("split") {
                1 => SplitDirMode::Horizontal,
                2 => SplitDirMode::Vertical,
                _ => SplitDirMode::Aspect,
            },
            ratio: opts.num("ratio"),
            gap: opts.maybe("gap"),
            force_split: match opts.member("force_split") {
                1 => ForceSide::First,
                2 => ForceSide::Second,
                _ => ForceSide::ByPointer,
            },
            preserve_split: opts.boolean("preserve_split"),
            smart_split: opts.boolean("smart_split"),
            smart_resizing: opts.boolean("smart_resizing"),
            permanent_direction_override: opts.boolean("permanent_direction_override"),
            split_width_multiplier: opts.num("split_width_multiplier"),
            use_active_for_splits: opts.boolean("use_active_for_splits"),
            bias_opened: opts.member("split_bias") == 1,
        }
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
fn distance(rect: &Rect, point: Pt) -> f64 {
    let dx = (rect.x - point.x).max(0.0).max(point.x - (rect.x + rect.width));
    let dy = (rect.y - point.y).max(0.0).max(point.y - (rect.y + rect.height));
    dx * dx + dy * dy
}

/// The window whose cell holds `point`, else the one whose cell is nearest it; the first of equals.
fn nearest(rects: &[(u64, Rect)], point: Pt) -> Option<u64> {
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
    options: DwindleSettings,
    nodes: Vec<Option<ArenaNode>>,
    free: Vec<Ix>,
    root: Option<Ix>,
    /// The leaves in the order they were added (the order a first-of-equals tie is broken in).
    leaves: Vec<(u64, Ix)>,
    /// Pseudotiled windows and the size each keeps; `None` until the next compute gives it its cell's size.
    pseudo: HashMap<u64, Option<(f64, f64)>>,
    preselect: PreselectDir,
    /// The focus the previous compute saw: the window a new one splits, because the new one takes focus on arrival.
    focus: Option<u64>,
    geometry: Option<(Rect, Shape)>,
    drag: Option<DividerDrag>,
    next_token: u64,
    /// Where the pointer is, for the compute that is running.
    at: Option<Pt>,
}

impl DwindleCore {
    fn new(options: DwindleSettings) -> DwindleCore {
        DwindleCore {
            options,
            nodes: Vec::new(),
            free: Vec::new(),
            root: None,
            leaves: Vec::new(),
            pseudo: HashMap::new(),
            preselect: PreselectDir::None,
            focus: None,
            geometry: None,
            drag: None,
            next_token: 0,
            at: None,
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
        let at = self.at;
        let options = self.options;
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
        if self.preselect != PreselectDir::None {
            let choice = self.preselect;
            dir = if matches!(choice, PreselectDir::Left | PreselectDir::Right) {
                Dir::H
            } else {
                Dir::V
            };
            first = matches!(choice, PreselectDir::Left | PreselectDir::Up);
            fixed = true;
            if !options.permanent_direction_override {
                self.preselect = PreselectDir::None;
            }
        } else {
            if options.split != SplitDirMode::Aspect {
                dir = if options.split == SplitDirMode::Horizontal {
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
            } else if options.force_split == ForceSide::First {
                first = true;
            } else if options.force_split == ForceSide::ByPointer
                && let Some(at) = at
            {
                first = if dir == Dir::H {
                    at.x < cell.x + cell.width / 2.0
                } else {
                    at.y < cell.y + cell.height / 2.0
                };
            }
        }
        self.insert(target, client, dir, first, fixed, options.bias_opened && first);
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
    fn find_split(&mut self, client: u64, point: Pt) -> Option<SplitSpec> {
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
        let divider = |core: &DwindleCore, at: Ix| -> DividerSpec {
            let split = core.split(at);
            DividerSpec {
                x_axis: split.dir == Dir::H,
                span: span(core, at),
                direction: 1,
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
        Some(SplitSpec {
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
    }

    fn reset_split(&mut self, token: u64) {
        let Some(drag) = self.drag.filter(|drag| drag.token == token) else {
            return;
        };
        self.split_mut(drag.main.0).ratio = drag.main.1;
        if let Some((at, original)) = drag.cross {
            self.split_mut(at).ratio = original;
        }
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
        true
    }

    fn swap_split(&mut self, client: Option<u64>) -> bool {
        let Some(at) = self.split_above(client) else {
            return false;
        };
        let split = self.split_mut(at);
        std::mem::swap(&mut split.a, &mut split.b);
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
        on
    }
}

// ---- the layout --------------------------------------------------------------------------------------------------------------

/// The built-in dwindle layout.
pub(super) struct Dwindle {
    opts: OptionSet,
    core: DwindleCore,
    /// Where the options say the pointer is (`fixed`), or `None`.
    pointer: PointerFrom,
}

#[derive(Clone, Copy, Debug)]
enum PointerFrom {
    Nowhere,
    Seat,
    Fixed(Option<Pt>),
}

impl Dwindle {
    pub(super) fn new(opts: OptionSet) -> Dwindle {
        let options = DwindleSettings::read(&opts);
        Dwindle {
            opts,
            core: DwindleCore::new(options),
            pointer: PointerFrom::Nowhere,
        }
    }
}

fn finite(point: Pt) -> Option<Pt> {
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

impl Imp for Dwindle {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn refresh(&mut self) {
        self.core.options = DwindleSettings::read(&self.opts);
        self.pointer = match self.opts.member("pointer") {
            1 => PointerFrom::Seat,
            2 => PointerFrom::Fixed(
                self.opts
                    .maybe("pointer_x")
                    .zip(self.opts.maybe("pointer_y"))
                    .map(|(x, y)| Pt { x, y }),
            ),
            _ => PointerFrom::Nowhere,
        };
    }

    fn wants_pointer(&self) -> bool {
        matches!(self.pointer, PointerFrom::Seat)
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let (area, gap) = geometry(self.core.options.gap, cx);
        self.core.at = match self.pointer {
            PointerFrom::Nowhere => None,
            PointerFrom::Seat => cx.pointer.map(|(x, y)| Pt { x, y }),
            PointerFrom::Fixed(point) => point,
        }
        .and_then(finite);
        let focused = cx.focused;
        let rects = self.core.compute(&cx.children, focused, area, gap);
        if rects.is_empty() {
            return Vec::new();
        }
        let by_id: HashMap<u64, Rect> = rects.into_iter().collect();
        let mut placements = Vec::with_capacity(cx.children.len());
        for client in &cx.children {
            let Some(cell) = by_id.get(client).copied() else {
                continue;
            };
            match self.core.pseudo.get(client).copied() {
                None => placements.push(tile(*client, cell.x, cell.y, cell.width, cell.height)),
                Some(kept) => {
                    let own = kept.unwrap_or((cell.width, cell.height));
                    self.core.pseudo.insert(*client, Some(own));
                    let width = own.0.min(cell.width);
                    let height = own.1.min(cell.height);
                    placements.push(tile(
                        *client,
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

    fn clients_swapped(&mut self, a: NodeId, b: NodeId) {
        self.core.swapped(a, b);
    }

    fn find_split(&mut self, client: NodeId, at: Pt) -> Option<SplitSpec> {
        self.core.find_split(client, at)
    }

    fn set_split(&mut self, token: u64, ratio: f64, cross: Option<f64>) {
        self.core.set_split(token, ratio, cross);
    }

    fn reset_split(&mut self, token: u64) {
        self.core.reset_split(token);
    }

    fn act(&mut self, action: &str, args: &[Val], env: &ActionEnv) -> Res<ActionResult> {
        let number = |at: usize| match args.get(at) {
            Some(Val::Num(value)) => *value,
            _ => 0.0,
        };
        let focus = env.focused;
        Ok(match action {
            "toggle_split" => ActionResult::did(self.core.toggle_split(focus)),
            "swap_split" => ActionResult::did(self.core.swap_split(focus)),
            "rotate_split" => {
                let angle = number(0);
                if angle % 90.0 != 0.0 {
                    return Err(Fail::type_error(format!(
                        "dwindle.rotate_split angle must be a multiple of 90, got {angle}"
                    )));
                }
                ActionResult::did(self.core.rotate_split(focus, (angle / 90.0).trunc() as i64))
            }
            "move_to_root" => ActionResult::did(
                self.core
                    .move_to_root(focus, matches!(args.first(), Some(Val::Bool(true)))),
            ),
            "split_ratio" => ActionResult::did(self.core.split_ratio(focus, number(0), false)),
            "set_split_ratio" => ActionResult::did(self.core.split_ratio(focus, number(0), true)),
            "preselect" => {
                self.core.preselect = PreselectDir::from_number(number(0) as u32);
                ActionResult::nothing()
            }
            "preselected" => ActionResult::answer(Val::Num(self.core.preselect.number())),
            "pseudotile" => {
                let size = match (args.first(), args.get(1)) {
                    (Some(Val::Num(width)), Some(Val::Num(height))) => {
                        if !(*width > 0.0 && *height > 0.0) {
                            return Err(Fail::type_error(format!(
                                "dwindle.pseudotile size must be numbers above 0, got {width} by {height}"
                            )));
                        }
                        Some((*width, *height))
                    }
                    (Some(Val::None) | None, Some(Val::None) | None) => None,
                    _ => {
                        return Err(Fail::type_error(
                            "dwindle.pseudotile takes both `width` and `height`, or neither",
                        ));
                    }
                };
                let Some(id) = focus else {
                    return Ok(ActionResult::answer(Val::Bool(false)));
                };
                // The window's own size with its frame is read only when it becomes pseudotiled.
                let pseudotiled = self.core.pseudo.contains_key(&id);
                let own = match size {
                    Some(size) => Some(size),
                    None if !pseudotiled => env.focused_size,
                    None => None,
                };
                let on = self.core.pseudotile(Some(id), own);
                ActionResult {
                    value: Val::Bool(on),
                    changed: true,
                    focus: None,
                }
            }
            "is_pseudotiled" => {
                let Some(Val::Node(client)) = args.first() else {
                    return Err(Fail::type_error("dwindle.is_pseudotiled needs a window"));
                };
                ActionResult::answer(Val::Bool(self.core.pseudo.contains_key(client)))
            }
            other => return Err(Fail::no_such(format!("the dwindle layout has no action `{other}`"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 800.0)
    }

    fn core(options: DwindleSettings) -> DwindleCore {
        DwindleCore::new(options)
    }

    fn defaults() -> DwindleSettings {
        DwindleSettings::read(&OptionSet::defaults(
            gnoblin_std_abi::layouts::layout_kind(gnoblin_std_abi::STD_BUILTIN_DWINDLE).expect("dwindle is declared"),
        ))
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
        let mut core = core(defaults());
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
        let mut core = core(defaults());
        arrive(&mut core, 3);
        let cells = core.compute(&[1, 3], Some(1), area(), 0.0);
        assert_eq!(text(&cells), "0,0,500,800 500,0,500,800");
        let cells = core.compute(&[3], Some(3), area(), 0.0);
        assert_eq!(text(&cells), "0,0,1000,800");
    }

    #[test]
    fn preselect_chooses_the_side_once_and_fixes_the_direction() {
        let mut core = core(defaults());
        arrive(&mut core, 1);
        core.preselect = PreselectDir::Up;
        // Above, though the 1000 by 800 cell would split side by side, and the new window is first.
        assert_eq!(
            text(&core.compute(&[1, 2], Some(2), area(), 0.0)),
            "0,400,1000,400 0,0,1000,400"
        );
        assert_eq!(core.preselect, PreselectDir::None);
    }

    #[test]
    fn a_drag_moves_the_nearest_divider_and_a_cancel_puts_it_back() {
        let mut core = core(defaults());
        arrive(&mut core, 2);
        let split = core.find_split(1, Pt { x: 505.0, y: 100.0 }).expect("a divider");
        assert!(split.divider.x_axis);
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
        let mut core = core(DwindleSettings {
            preserve_split: true,
            ..defaults()
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
