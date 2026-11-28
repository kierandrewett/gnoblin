//! The drags of the library's own pointer bindings, as state machines over values only: move, resize, tile swap and split resize.
//!
//! A machine is told the four steps of a drag (start, update, end, cancel) with what the caller knows of the window the press landed
//! on, and answers what the binding wants done (`DragAnswer`): a rectangle for the window, a tile to trade places with, a ratio for a
//! divider. It reads and writes nothing else, so one body serves two callers:
//!
//! - the library's own bindings (`drags.rs`), which match the chord, read the window from the compositor's tree and write the answer
//!   back to it (`binding_builtin`);
//! - a shell that matches its own drags (the TypeScript engine, with its tiers and scopes) and keeps its own window objects, which
//!   steps a machine through `drag_step` and writes the answer to them.
//!
//! The rules are those of `data/js/gnoblin-std/bindings/{move,resize,tiling,drag}.ts` before they became a layer over this module
//! (design 22.7 and 25.4). What they do beyond a plain drag: a moved window keeps its whole frame (the content and the frame inset
//! round it) inside the work area; a drop within a few pixels of a screen edge snaps the window to half or all of the work area when
//! the seat's `edge_tiling` is on; a resize stops at the window's own minimum and maximum size; a move or resize remembers the
//! rectangle the window floats at; and the split resize drags the divider of a tiling layout that has one.

use gnoblin_core::host::Rect;
use gnoblin_std_abi::{
    GnoblinStdSplit, STD_AXIS_Y, STD_BINDING_MOVE, STD_BINDING_RESIZE, STD_BINDING_SPLIT_RESIZE, STD_BINDING_SWAP,
    STD_CONSTRAINT_MINIMUM_STRIP, STD_DRAG_CANCEL, STD_DRAG_CLIENT_DIALOG, STD_DRAG_CLIENT_FLOATING,
    STD_DRAG_CLIENT_FULLSCREEN, STD_DRAG_CLIENT_HIDDEN, STD_DRAG_CLIENT_MAXIMISED, STD_DRAG_CLIENT_MINIMISED,
    STD_DRAG_CLIENT_ON_WORKSPACE, STD_DRAG_CLIENT_TILED, STD_DRAG_END, STD_DRAG_START, STD_DRAG_STRIP, STD_DRAG_UPDATE,
    STD_SIDE_BOTTOM, STD_SIDE_LEFT, STD_SIDE_RIGHT, STD_SIDE_TOP, STD_SNAP_LEFT, STD_SNAP_MARGIN, STD_SNAP_RIGHT,
    STD_SNAP_TOP, STD_SPLIT_CROSS,
};

use super::DragCtx;
use crate::builtin::{OptionSet, Pt};
use crate::floating::{Inset, grow, shrink};
use crate::state::NodeId;

/// How much of a window stays on screen under `STD_CONSTRAINT_MINIMUM_STRIP`.
const DRAG_STRIP: f64 = STD_DRAG_STRIP as f64;

/// The smallest size a resize leaves a window that has no limit of its own.
pub(crate) const MIN_SIZE: (f64, f64) = (100.0, 80.0);

/// How near an edge the pointer must be to name it, in pixels.
pub(super) const SNAP_MARGIN: f64 = STD_SNAP_MARGIN as f64;

/// `pos` held inside `[start, start + span]` for a window of `size`: a window larger than the span goes to the start.
pub(crate) fn clamp_axis(pos: f64, size: f64, start: f64, span: f64, constraint: u32) -> f64 {
    if constraint == STD_CONSTRAINT_MINIMUM_STRIP {
        let visible = DRAG_STRIP.min(size);
        let low = start - (size - visible);
        let high = start + span - visible;
        pos.max(low).min(high)
    } else if size >= span {
        start
    } else {
        pos.max(start).min(start + span - size)
    }
}

/// `rect` moved to where `constraint` allows inside `area`; the size is kept.
fn constrain_rect(rect: Rect, area: Rect, constraint: u32) -> Rect {
    Rect::new(
        clamp_axis(rect.x, rect.width, area.x, area.width, constraint),
        clamp_axis(rect.y, rect.height, area.y, area.height, constraint),
        rect.width,
        rect.height,
    )
}

/// The zone of the edge of `area` the pointer at (`x`, `y`) is within `margin` pixels of, if any. Left and right win over top at the
/// corners.
pub(crate) fn edge_zone(x: f64, y: f64, area: Rect, margin: f64) -> Option<u32> {
    if x - area.x <= margin {
        Some(STD_SNAP_LEFT)
    } else if area.x + area.width - x <= margin {
        Some(STD_SNAP_RIGHT)
    } else if y - area.y <= margin {
        Some(STD_SNAP_TOP)
    } else {
        None
    }
}

/// The sides of a window a resize moves: a corner moves two, an edge one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sides {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Sides {
    /// The sides `STD_SIDE_*` flags name.
    pub(crate) fn from_flags(flags: u32) -> Sides {
        Sides {
            left: flags & STD_SIDE_LEFT != 0,
            right: flags & STD_SIDE_RIGHT != 0,
            top: flags & STD_SIDE_TOP != 0,
            bottom: flags & STD_SIDE_BOTTOM != 0,
        }
    }

    /// The `STD_SIDE_*` flags of the sides.
    pub(crate) fn flags(self) -> u32 {
        [
            (self.left, STD_SIDE_LEFT),
            (self.right, STD_SIDE_RIGHT),
            (self.top, STD_SIDE_TOP),
            (self.bottom, STD_SIDE_BOTTOM),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |flags, (_, flag)| flags | flag)
    }

    fn corner(left: bool, top: bool) -> Sides {
        Sides {
            left,
            right: !left,
            top,
            bottom: !top,
        }
    }
}

/// `start` resized by `delta` with the `grip` sides following the pointer, never below `min` or above `max`; a side that is stopped
/// by a limit leaves the opposite side where it was. An axis no side moves on keeps its size, whatever the limits say.
pub(crate) fn resized_rect(start: Rect, grip: Sides, delta: Pt, min: (f64, f64), max: Option<(f64, f64)>) -> Rect {
    let mut r = start;
    if grip.left {
        r.x += delta.x;
        r.width -= delta.x;
    } else if grip.right {
        r.width += delta.x;
    }
    if grip.top {
        r.y += delta.y;
        r.height -= delta.y;
    } else if grip.bottom {
        r.height += delta.y;
    }
    if grip.left || grip.right {
        let limited = r.width.min(max.map_or(f64::INFINITY, |max| max.0)).max(min.0);
        if grip.left {
            r.x += r.width - limited;
        }
        r.width = limited;
    }
    if grip.top || grip.bottom {
        let limited = r.height.min(max.map_or(f64::INFINITY, |max| max.1)).max(min.1);
        if grip.top {
            r.y += r.height - limited;
        }
        r.height = limited;
    }
    r
}

/// Which side or corner a resize takes hold of (`RESIZE_GRIP`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GripChoice {
    Quadrant,
    Nearest,
    Corner(Sides),
}

impl GripChoice {
    pub(crate) fn from_member(member: u32) -> GripChoice {
        match member {
            1 => GripChoice::Nearest,
            2 => GripChoice::Corner(Sides::corner(true, true)),
            3 => GripChoice::Corner(Sides::corner(false, true)),
            4 => GripChoice::Corner(Sides::corner(true, false)),
            5 => GripChoice::Corner(Sides::corner(false, false)),
            _ => GripChoice::Quadrant,
        }
    }

    /// The sides a press at `at` (in the window's own coordinates) takes hold of in a window of `width` by `height`. A tie between
    /// the edges goes to left, then top.
    fn sides_at(self, at: Pt, width: f64, height: f64) -> Sides {
        match self {
            GripChoice::Quadrant => Sides::corner(at.x < width / 2.0, at.y < height / 2.0),
            GripChoice::Corner(sides) => sides,
            GripChoice::Nearest => {
                let edge = |flag: u32| Sides::from_flags(flag);
                let gaps = [
                    (edge(STD_SIDE_LEFT), at.x),
                    (edge(STD_SIDE_TOP), at.y),
                    (edge(STD_SIDE_RIGHT), width - at.x),
                    (edge(STD_SIDE_BOTTOM), height - at.y),
                ];
                gaps.into_iter()
                    .reduce(|best, gap| if gap.1 < best.1 { gap } else { best })
                    .map_or(gaps[0].0, |(sides, _)| sides)
            }
        }
    }
}

/// One divider of a drag, as a layout described it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DragDivider {
    y_axis: bool,
    direction: f64,
    span: f64,
    min: f64,
    max: f64,
    ratio: f64,
}

impl DragDivider {
    fn new(axis: u32, direction: i32, span: f64, min: f64, max: f64, ratio: f64) -> DragDivider {
        DragDivider {
            y_axis: axis == STD_AXIS_Y,
            direction: f64::from(direction.signum()),
            span,
            min,
            max,
            ratio,
        }
    }

    /// The ratio the pointer's travel from `start` to `at` carries the divider to, from `start_ratio`, held to `min..=max`.
    fn follow(&self, start_ratio: f64, start: Pt, at: Pt) -> f64 {
        let travel = if self.y_axis { at.y - start.y } else { at.x - start.x };
        (start_ratio + self.direction * travel / self.span).clamp(self.min, self.max)
    }
}

/// The dividers a layout's `find_split` described: the one a drag moves, the second one on the other axis when the layout offers it
/// (and it has a span), and the token that names them to the layout.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SplitView {
    pub(crate) token: u64,
    divider: DragDivider,
    cross: Option<DragDivider>,
}

impl SplitView {
    pub(crate) fn of(raw: &GnoblinStdSplit) -> SplitView {
        let divider = DragDivider::new(raw.axis, raw.direction, raw.span, raw.min, raw.max, raw.ratio);
        let cross = (raw.flags & STD_SPLIT_CROSS != 0)
            .then(|| {
                DragDivider::new(
                    raw.cross_axis,
                    raw.cross_direction,
                    raw.cross_span,
                    raw.cross_min,
                    raw.cross_max,
                    raw.cross_ratio,
                )
            })
            .filter(|cross| cross.span > 0.0);
        SplitView {
            token: raw.token,
            divider,
            cross,
        }
    }
}

/// What a step knows of the window the press landed on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientView {
    /// `STD_DRAG_CLIENT_*`.
    pub(crate) flags: u32,
    /// The window's content rectangle now.
    pub(crate) bounds: Rect,
    /// The room its frame takes round the content.
    pub(crate) inset: Inset,
    /// The least and the most it may be resized to, when it says.
    pub(crate) min: Option<(f64, f64)>,
    pub(crate) max: Option<(f64, f64)>,
}

/// A window that is not fullscreen and not maximised.
fn draggable(flags: u32) -> bool {
    flags & (STD_DRAG_CLIENT_FULLSCREEN | STD_DRAG_CLIENT_MAXIMISED) == 0
}

/// Whether a client sits in a tile: tiled by its layout, not floating, not a dialog, and draggable.
fn in_tile(flags: u32) -> bool {
    flags & STD_DRAG_CLIENT_TILED != 0
        && flags & (STD_DRAG_CLIENT_FLOATING | STD_DRAG_CLIENT_DIALOG) == 0
        && draggable(flags)
}

/// One tile of the dragged window's workspace.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tile {
    pub(crate) id: NodeId,
    pub(crate) flags: u32,
    pub(crate) rect: Rect,
}

/// What a rule is given for one step.
pub(crate) struct DragInput<'a> {
    /// The pointer, where the press was, and the work area.
    pub(crate) event: &'a DragCtx,
    /// The window the press landed on; `None` when it landed on nothing (the event's `client` is then `None` too).
    pub(crate) client: Option<ClientView>,
    /// The window's rectangle when the drag started: what an update moves it from and a cancel puts back.
    pub(crate) start: Rect,
    /// A resize's update: the sides the drag moves, as the start's outcome gave them.
    pub(crate) sides: Sides,
    /// A split resize: the divider the layout found for the press, with the ratios it had then.
    pub(crate) split: Option<SplitView>,
    /// A swap's update and end: the tiles of the dragged window's workspace, in order.
    pub(crate) tiles: &'a [Tile],
}

/// What a rule asks the caller to do after a step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DragAnswer {
    /// A start: the binding took the drag.
    pub(crate) took: bool,
    /// The rectangle the window is to have.
    pub(crate) bounds: Option<Rect>,
    /// The rectangle the window floats at from now on.
    pub(crate) floating: Option<Rect>,
    /// The window is not snapped any more and stays where it is.
    pub(crate) forget_snap: bool,
    /// The window is to be tiled to this zone.
    pub(crate) snap_zone: Option<u32>,
    /// The tile the dragged window trades places with (on an update, the one the pointer is over).
    pub(crate) swap: Option<NodeId>,
    /// The ratio of the divider, and of the second one.
    pub(crate) ratio: Option<f64>,
    pub(crate) cross_ratio: Option<f64>,
    /// The dividers go back to where the drag found them.
    pub(crate) reset_split: bool,
    /// A resize start: the sides the drag moves.
    pub(crate) sides: Option<Sides>,
}

impl DragAnswer {
    fn took() -> DragAnswer {
        DragAnswer {
            took: true,
            ..DragAnswer::default()
        }
    }

    fn bounds(bounds: Rect) -> DragAnswer {
        DragAnswer {
            bounds: Some(bounds),
            ..DragAnswer::default()
        }
    }
}

/// One of the library's own drags. It holds its options and nothing of a drag in flight: that is in the caller's hands and comes
/// back with every step.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DragRule {
    /// Drags the window under the pointer with it. `snap` is how near a screen edge a drop snaps, or `None` for never.
    Move { snap: Option<f64> },
    /// Resizes the window under the pointer by the side or corner the grip names.
    Resize { grip: GripChoice },
    /// Swaps a tiled window with the tile it is dropped on: the pointer takes hold of a tile and the release decides, not the
    /// last motion.
    Swap,
    /// Moves the divider of a tiling layout nearest the press by as far as the pointer travels.
    Split,
}

impl DragRule {
    /// The rule of the binding `kind` (`STD_BINDING_*`) with its options checked already, or `None` for an unknown kind.
    pub(crate) fn of(kind: u32, options: &OptionSet) -> Option<DragRule> {
        Some(match kind {
            STD_BINDING_MOVE => DragRule::Move {
                snap: options
                    .boolean("snap")
                    .then(|| options.maybe("snap_margin").unwrap_or(SNAP_MARGIN)),
            },
            STD_BINDING_RESIZE => DragRule::Resize {
                grip: GripChoice::from_member(options.member("grip")),
            },
            STD_BINDING_SWAP => DragRule::Swap,
            STD_BINDING_SPLIT_RESIZE => DragRule::Split,
            _ => return None,
        })
    }

    /// Whether a drag of this kind asks a layout for the divider under the press when it starts.
    pub(crate) fn wants_split(&self) -> bool {
        matches!(self, DragRule::Split)
    }

    /// Gives the drag its next step. `phase` is `STD_DRAG_START`, `STD_DRAG_UPDATE`, `STD_DRAG_END` or `STD_DRAG_CANCEL`.
    pub(crate) fn step(&self, phase: u32, step: &DragInput) -> DragAnswer {
        match *self {
            DragRule::Move { snap } => move_step(snap, phase, step),
            DragRule::Resize { grip } => resize_step(grip, phase, step),
            DragRule::Swap => swap_step(phase, step),
            DragRule::Split => split_step(phase, step),
        }
    }
}

/// The window the press landed on, when the step has one that may be dragged.
fn draggable_client(step: &DragInput) -> Option<ClientView> {
    step.client
        .filter(|client| step.event.client.is_some() && draggable(client.flags))
}

fn move_step(snap: Option<f64>, phase: u32, step: &DragInput) -> DragAnswer {
    let event = step.event;
    match phase {
        STD_DRAG_START => {
            if draggable_client(step).is_none() {
                return DragAnswer::default();
            }
            // A snapped window the user takes hold of is no longer tiled to its zone.
            DragAnswer {
                forget_snap: true,
                ..DragAnswer::took()
            }
        }
        STD_DRAG_UPDATE => {
            let start = step.start;
            let target = Rect::new(
                start.x + (event.position.x - event.origin.x),
                start.y + (event.position.y - event.origin.y),
                start.width,
                start.height,
            );
            // The frame (the content with its frame around it) is what stays in the work area: a titlebar dragged off the top could
            // not be grabbed again. Without a work area there is nothing to keep it in.
            DragAnswer::bounds(event.work_area.map_or(target, |area| {
                let inset = step.client.map(|client| client.inset).unwrap_or_default();
                let content = shrink(&constrain_rect(grow(&target, &inset), area, event.constraint), &inset);
                Rect::new(content.x, content.y, target.width, target.height)
            }))
        }
        STD_DRAG_END => DragAnswer {
            floating: step.client.map(|client| client.bounds),
            snap_zone: match (snap, event.edge_tiling, event.work_area) {
                (Some(margin), true, Some(area)) => edge_zone(event.position.x, event.position.y, area, margin),
                _ => None,
            },
            ..DragAnswer::default()
        },
        STD_DRAG_CANCEL => DragAnswer::bounds(step.start),
        _ => DragAnswer::default(),
    }
}

fn resize_step(grip: GripChoice, phase: u32, step: &DragInput) -> DragAnswer {
    let event = step.event;
    match phase {
        STD_DRAG_START => {
            let Some(client) = draggable_client(step) else {
                return DragAnswer::default();
            };
            let bounds = client.bounds;
            let at = event.local.unwrap_or(Pt {
                x: event.position.x - bounds.x,
                y: event.position.y - bounds.y,
            });
            DragAnswer {
                sides: Some(grip.sides_at(at, bounds.width, bounds.height)),
                ..DragAnswer::took()
            }
        }
        STD_DRAG_UPDATE => {
            let delta = Pt {
                x: event.position.x - event.origin.x,
                y: event.position.y - event.origin.y,
            };
            // The window's own limits, else the smallest size every resize leaves.
            let min = step.client.and_then(|client| client.min).unwrap_or(MIN_SIZE);
            let max = step.client.and_then(|client| client.max);
            DragAnswer::bounds(resized_rect(step.start, step.sides, delta, min, max))
        }
        STD_DRAG_END => DragAnswer {
            floating: step.client.map(|client| client.bounds),
            ..DragAnswer::default()
        },
        STD_DRAG_CANCEL => DragAnswer::bounds(step.start),
        _ => DragAnswer::default(),
    }
}

/// The tile that holds `at`, other than `except`: the first in the workspace's order that is shown.
fn tile_at(tiles: &[Tile], at: Pt, except: Option<NodeId>) -> Option<NodeId> {
    tiles
        .iter()
        .find(|tile| {
            Some(tile.id) != except
                && in_tile(tile.flags)
                && tile.flags & (STD_DRAG_CLIENT_MINIMISED | STD_DRAG_CLIENT_HIDDEN) == 0
                && at.x >= tile.rect.x
                && at.x < tile.rect.x + tile.rect.width
                && at.y >= tile.rect.y
                && at.y < tile.rect.y + tile.rect.height
        })
        .map(|tile| tile.id)
}

fn swap_step(phase: u32, step: &DragInput) -> DragAnswer {
    let event = step.event;
    match phase {
        STD_DRAG_START => {
            if step
                .client
                .is_some_and(|client| in_tile(client.flags) && client.flags & STD_DRAG_CLIENT_ON_WORKSPACE != 0)
                && event.client.is_some()
            {
                DragAnswer::took()
            } else {
                DragAnswer::default()
            }
        }
        // The tile the pointer is over, for a caller that shows it; the order changes on the release, which decides, not the
        // last motion: the pointer may have moved since the last sample.
        STD_DRAG_UPDATE | STD_DRAG_END => DragAnswer {
            swap: tile_at(step.tiles, event.position, event.client),
            ..DragAnswer::default()
        },
        _ => DragAnswer::default(),
    }
}

fn split_step(phase: u32, step: &DragInput) -> DragAnswer {
    let event = step.event;
    match phase {
        STD_DRAG_START => {
            if event.client.is_none() || !step.client.is_some_and(|client| in_tile(client.flags)) {
                return DragAnswer::default();
            }
            if step.split.is_some_and(|split| split.divider.span > 0.0) {
                DragAnswer::took()
            } else {
                DragAnswer::default()
            }
        }
        STD_DRAG_UPDATE => {
            let Some(split) = step.split else {
                return DragAnswer::default();
            };
            DragAnswer {
                ratio: Some(split.divider.follow(split.divider.ratio, event.origin, event.position)),
                cross_ratio: split
                    .cross
                    .map(|cross| cross.follow(cross.ratio, event.origin, event.position)),
                ..DragAnswer::default()
            }
        }
        STD_DRAG_CANCEL => DragAnswer {
            reset_split: true,
            ..DragAnswer::default()
        },
        _ => DragAnswer::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_std_abi::STD_CONSTRAINT_WORK_AREA;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(x, y, width, height)
    }

    fn pt(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }

    fn event(client: Option<NodeId>, at: Pt) -> DragCtx {
        DragCtx {
            client,
            position: at,
            origin: at,
            local: None,
            button: 1,
            work_area: Some(rect(0.0, 0.0, 1000.0, 800.0)),
            constraint: STD_CONSTRAINT_WORK_AREA,
            edge_tiling: false,
        }
    }

    fn view(flags: u32, bounds: Rect) -> ClientView {
        ClientView {
            flags,
            bounds,
            inset: Inset::default(),
            min: None,
            max: None,
        }
    }

    #[test]
    fn a_window_larger_than_the_work_area_goes_to_its_start_and_a_smaller_one_stays_inside() {
        assert_eq!(clamp_axis(-50.0, 300.0, 0.0, 1000.0, 0), 0.0);
        assert_eq!(clamp_axis(900.0, 300.0, 0.0, 1000.0, 0), 700.0);
        assert_eq!(clamp_axis(100.0, 2000.0, 0.0, 1000.0, 0), 0.0);
        // A strip of 75 pixels stays reachable under the minimum-strip rule.
        assert_eq!(
            clamp_axis(5000.0, 300.0, 0.0, 1000.0, STD_CONSTRAINT_MINIMUM_STRIP),
            925.0
        );
        assert_eq!(
            clamp_axis(-5000.0, 300.0, 0.0, 1000.0, STD_CONSTRAINT_MINIMUM_STRIP),
            -225.0
        );
    }

    #[test]
    fn the_edge_nearest_the_pointer_names_the_zone_and_left_and_right_win_the_corners() {
        let area = rect(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(edge_zone(2.0, 400.0, area, SNAP_MARGIN), Some(STD_SNAP_LEFT));
        assert_eq!(edge_zone(998.0, 400.0, area, SNAP_MARGIN), Some(STD_SNAP_RIGHT));
        assert_eq!(edge_zone(500.0, 3.0, area, SNAP_MARGIN), Some(STD_SNAP_TOP));
        assert_eq!(edge_zone(1.0, 1.0, area, SNAP_MARGIN), Some(STD_SNAP_LEFT));
        assert_eq!(edge_zone(500.0, 400.0, area, SNAP_MARGIN), None);
    }

    #[test]
    fn a_resize_stops_at_the_limits_and_the_opposite_side_stays_put() {
        let start = rect(300.0, 200.0, 400.0, 300.0);
        let grip = Sides::from_flags(STD_SIDE_LEFT | STD_SIDE_BOTTOM);
        // Pulled left by 100: the left side follows. Pushed right by 1000: the left side stops at the minimum width.
        let wider = resized_rect(start, grip, pt(-100.0, 0.0), MIN_SIZE, None);
        assert_eq!((wider.x, wider.width), (200.0, 500.0));
        let narrow = resized_rect(start, grip, pt(1000.0, 0.0), MIN_SIZE, None);
        assert_eq!((narrow.x, narrow.width), (600.0, 100.0));
        let capped = resized_rect(start, grip, pt(-500.0, 0.0), MIN_SIZE, Some((600.0, 400.0)));
        assert_eq!((capped.x, capped.width), (100.0, 600.0));
    }

    #[test]
    fn an_edge_grip_moves_one_side_and_the_quadrant_picks_the_corner() {
        let size = (300.0, 200.0);
        assert_eq!(
            GripChoice::Quadrant.sides_at(pt(10.0, 10.0), size.0, size.1),
            Sides::from_flags(STD_SIDE_LEFT | STD_SIDE_TOP)
        );
        assert_eq!(
            GripChoice::Quadrant.sides_at(pt(290.0, 190.0), size.0, size.1),
            Sides::from_flags(STD_SIDE_RIGHT | STD_SIDE_BOTTOM)
        );
        // Nearest: the right edge is 5 away, the others further; a tie goes to left, then top.
        assert_eq!(
            GripChoice::Nearest.sides_at(pt(295.0, 100.0), size.0, size.1),
            Sides::from_flags(STD_SIDE_RIGHT)
        );
        assert_eq!(
            GripChoice::Nearest.sides_at(pt(5.0, 5.0), size.0, size.1),
            Sides::from_flags(STD_SIDE_LEFT)
        );
    }

    #[test]
    fn a_divider_follows_the_pointer_along_its_axis_and_stays_in_its_range() {
        let divider = DragDivider::new(0, 1, 1000.0, 0.1, 0.9, 0.5);
        let start = pt(500.0, 100.0);
        assert_eq!(divider.follow(0.5, start, pt(700.0, 900.0)), 0.7);
        assert_eq!(divider.follow(0.5, start, pt(9000.0, 0.0)), 0.9);
        let back = DragDivider::new(0, -1, 1000.0, 0.1, 0.9, 0.5);
        assert_eq!(back.follow(0.5, start, pt(700.0, 0.0)), 0.3);
    }

    fn step<'a>(event: &'a DragCtx, client: Option<ClientView>, start: Rect) -> DragInput<'a> {
        DragInput {
            event,
            client,
            start,
            sides: Sides::from_flags(0),
            split: None,
            tiles: &[],
        }
    }

    #[test]
    fn a_move_keeps_the_pointer_offset_declines_a_maximised_window_and_goes_back_on_a_cancel() {
        let rule = DragRule::Move { snap: None };
        let window = view(0, rect(50.0, 60.0, 300.0, 200.0));
        let press = event(Some(3), pt(100.0, 100.0));
        let declined = rule.step(
            STD_DRAG_START,
            &step(
                &press,
                Some(view(STD_DRAG_CLIENT_MAXIMISED, window.bounds)),
                window.bounds,
            ),
        );
        assert!(!declined.took);
        let started = rule.step(STD_DRAG_START, &step(&press, Some(window), window.bounds));
        assert!(started.took && started.forget_snap);
        // The pointer is at (130, 90) and the press was at (100, 100): the window is 30 right and 10 up.
        let mut now = press.clone();
        now.position = pt(130.0, 90.0);
        let moved = rule.step(STD_DRAG_UPDATE, &step(&now, Some(window), window.bounds));
        assert_eq!(moved.bounds, Some(rect(80.0, 50.0, 300.0, 200.0)));
        // Far off the screen: held inside the work area.
        now.position = pt(5000.0, 5000.0);
        let held = rule.step(STD_DRAG_UPDATE, &step(&now, Some(window), window.bounds));
        assert_eq!(held.bounds, Some(rect(700.0, 600.0, 300.0, 200.0)));
        let cancelled = rule.step(STD_DRAG_CANCEL, &step(&now, Some(window), window.bounds));
        assert_eq!(cancelled.bounds, Some(window.bounds));
    }

    #[test]
    fn a_move_dropped_on_an_edge_snaps_only_with_edge_tiling_on() {
        let rule = DragRule::Move {
            snap: Some(SNAP_MARGIN),
        };
        let window = view(0, rect(50.0, 60.0, 300.0, 200.0));
        let mut at = event(Some(3), pt(100.0, 100.0));
        at.position = pt(2.0, 400.0);
        let plain = rule.step(STD_DRAG_END, &step(&at, Some(window), window.bounds));
        assert_eq!(plain.snap_zone, None);
        assert_eq!(plain.floating, Some(window.bounds));
        at.edge_tiling = true;
        let snapped = rule.step(STD_DRAG_END, &step(&at, Some(window), window.bounds));
        assert_eq!(snapped.snap_zone, Some(STD_SNAP_LEFT));
    }

    #[test]
    fn a_resize_picks_its_sides_at_the_start_and_keeps_them_for_the_updates() {
        let rule = DragRule::Resize {
            grip: GripChoice::Quadrant,
        };
        let window = view(0, rect(100.0, 100.0, 400.0, 300.0));
        // A press in the bottom right quadrant: the right and bottom sides follow.
        let press = event(Some(3), pt(450.0, 380.0));
        let started = rule.step(STD_DRAG_START, &step(&press, Some(window), window.bounds));
        let sides = started.sides.expect("a resize that took the drag says its sides");
        assert_eq!(sides, Sides::from_flags(STD_SIDE_RIGHT | STD_SIDE_BOTTOM));
        let mut now = press.clone();
        now.position = pt(500.0, 400.0);
        let mut update = step(&now, Some(window), window.bounds);
        update.sides = sides;
        assert_eq!(
            rule.step(STD_DRAG_UPDATE, &update).bounds,
            Some(rect(100.0, 100.0, 450.0, 320.0))
        );
    }

    #[test]
    fn a_swap_is_decided_by_the_release_and_skips_the_hidden_and_the_untiled() {
        let rule = DragRule::Swap;
        let tiled = STD_DRAG_CLIENT_TILED | STD_DRAG_CLIENT_ON_WORKSPACE;
        let mine = view(tiled, rect(0.0, 0.0, 500.0, 800.0));
        let tiles = [
            Tile {
                id: 3,
                flags: tiled,
                rect: rect(0.0, 0.0, 500.0, 800.0),
            },
            Tile {
                id: 4,
                flags: tiled | STD_DRAG_CLIENT_HIDDEN,
                rect: rect(500.0, 0.0, 500.0, 800.0),
            },
            Tile {
                id: 5,
                flags: tiled,
                rect: rect(500.0, 0.0, 500.0, 800.0),
            },
        ];
        let start = event(Some(3), pt(100.0, 100.0));
        assert!(rule.step(STD_DRAG_START, &step(&start, Some(mine), mine.bounds)).took);
        let mut release = event(Some(3), pt(700.0, 100.0));
        release.origin = start.origin;
        let mut end = step(&release, Some(mine), mine.bounds);
        end.tiles = &tiles;
        assert_eq!(rule.step(STD_DRAG_END, &end).swap, Some(5));
        // Dropped on its own tile: nothing to trade with.
        release.position = pt(100.0, 100.0);
        let mut home = step(&release, Some(mine), mine.bounds);
        home.tiles = &tiles;
        assert_eq!(rule.step(STD_DRAG_END, &home).swap, None);
        // A floating window never starts a swap.
        let floated = view(tiled | STD_DRAG_CLIENT_FLOATING, mine.bounds);
        assert!(
            !rule
                .step(STD_DRAG_START, &step(&start, Some(floated), mine.bounds))
                .took
        );
    }

    #[test]
    fn a_split_resize_declines_without_a_divider_and_follows_the_pointer_from_the_press() {
        let rule = DragRule::Split;
        let tiled = view(STD_DRAG_CLIENT_TILED, rect(0.0, 0.0, 500.0, 800.0));
        let press = event(Some(3), pt(500.0, 100.0));
        assert!(!rule.step(STD_DRAG_START, &step(&press, Some(tiled), tiled.bounds)).took);
        let raw = GnoblinStdSplit {
            size: size_of::<GnoblinStdSplit>() as u32,
            flags: 0,
            token: 7,
            axis: 0,
            direction: 1,
            span: 1000.0,
            min: 0.1,
            max: 0.9,
            ratio: 0.5,
            cross_axis: 0,
            cross_direction: 0,
            cross_span: 0.0,
            cross_min: 0.0,
            cross_max: 0.0,
            cross_ratio: 0.0,
        };
        let split = SplitView::of(&raw);
        let mut with = step(&press, Some(tiled), tiled.bounds);
        with.split = Some(split);
        assert!(rule.step(STD_DRAG_START, &with).took);
        let mut now = press.clone();
        now.position = pt(700.0, 100.0);
        let mut update = step(&now, Some(tiled), tiled.bounds);
        update.split = Some(split);
        assert_eq!(rule.step(STD_DRAG_UPDATE, &update).ratio, Some(0.7));
        assert!(rule.step(STD_DRAG_CANCEL, &update).reset_split);
    }
}
