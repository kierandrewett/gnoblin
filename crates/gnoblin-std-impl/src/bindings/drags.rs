//! The pointer bindings the library runs itself: move, resize, tile swap and split resize.
//!
//! Ports of `data/js/gnoblin-std/bindings/{move,resize,tiling,drag}.ts`, which are the reference (`design/modular-framework.md`,
//! section 22.7 and 22.9). What they do beyond a plain drag: a moved window keeps its whole frame (the content and the frame inset
//! round it) inside the work area; a drop within a few pixels of a screen edge snaps the window to half or all of the work area when
//! the seat's `edge_tiling` is on; a resize stops at the window's own minimum and maximum size; a move or resize remembers the
//! rectangle the window floats at; and the split resize drags the divider of a tiling layout that has one.

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_std_abi::{
    GnoblinStdSplit, STD_AXIS_Y, STD_CONSTRAINT_MINIMUM_STRIP, STD_SNAP_LEFT, STD_SNAP_RIGHT, STD_SNAP_TOP,
    STD_SPLIT_CROSS,
};

use super::{DragCtx, DragHandler, with_state};
use crate::builtin::Pt;
use crate::floating::{grow, shrink};
use crate::nodes;
use crate::state::{NodeId, Std};

/// How much of a window stays on screen under `STD_CONSTRAINT_MINIMUM_STRIP`.
const DRAG_STRIP: f64 = 75.0;

/// The smallest size a resize leaves a window that has no limit of its own.
const MIN_SIZE: (f64, f64) = (100.0, 80.0);

/// How near an edge the pointer must be to name it, in pixels.
pub(super) const SNAP_MARGIN: f64 = 4.0;

/// `pos` held inside `[start, start + span]` for a window of `size`: a window larger than the span goes to the start.
fn clamp_axis(pos: f64, size: f64, start: f64, span: f64, constraint: u32) -> f64 {
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
fn edge_zone(x: f64, y: f64, area: Rect, margin: f64) -> Option<u32> {
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

/// A window that is not fullscreen and not maximised.
fn draggable(std: &Std, client: NodeId) -> bool {
    let st = std.st.borrow();
    !nodes::flag(&st.g, client, "fullscreen") && !nodes::flag(&st.g, client, "maximised")
}

/// Whether `client` sits in a tile: tiled by its layout, not floating, not a dialog, and draggable.
fn in_tile(std: &Std, client: NodeId) -> bool {
    let tile = {
        let st = std.st.borrow();
        nodes::flag(&st.g, client, "tiled")
            && !st.rec.get(&client).is_some_and(|rec| rec.floating)
            && nodes::node_ref(&st.g, client, "parent_window").is_none()
    };
    tile && draggable(std, client)
}

/// The content rectangle of a frame: `frame` less the inset on each side, never smaller than nothing.
fn content_of(frame: &Rect, inset: &crate::floating::Inset) -> Rect {
    let content = shrink(frame, inset);
    Rect::new(content.x, content.y, content.width.max(0.0), content.height.max(0.0))
}

/// Drags the window under the pointer with it.
pub(super) struct MoveDrag {
    /// How near a screen edge a drop snaps, or `None` for never.
    pub(super) snap: Option<f64>,
    drag: Option<(NodeId, Pt, Rect)>,
}

impl MoveDrag {
    pub(super) fn new(snap: Option<f64>) -> MoveDrag {
        MoveDrag { snap, drag: None }
    }
}

impl DragHandler for MoveDrag {
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool {
        let Some(client) = event.client.filter(|client| draggable(std, *client)) else {
            return false;
        };
        let bounds = nodes::rect_of(&std.st.borrow().g, client);
        // A snapped window the user takes hold of is no longer tiled to its zone.
        with_state(std, |st| st.unsnap_client(client, true));
        self.drag = Some((client, event.origin, bounds));
        true
    }

    fn update(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let Some((client, origin, start)) = &self.drag else {
            return;
        };
        let target = Rect::new(
            start.x + (event.position.x - origin.x),
            start.y + (event.position.y - origin.y),
            start.width,
            start.height,
        );
        with_state(std, |st| {
            // The frame (the content with its frame around it) is what stays in the work area: a titlebar dragged off the top could
            // not be grabbed again. Without a work area there is nothing to keep it in.
            let bounds = event.work_area.map_or(target, |area| {
                let inset = st.inset_of(*client);
                let content = content_of(&constrain_rect(grow(&target, &inset), area, event.constraint), &inset);
                Rect::new(content.x, content.y, target.width, target.height)
            });
            nodes::write_rect(&st.g, *client, &bounds);
        });
    }

    fn end(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let Some((client, _, _)) = self.drag.take() else {
            return;
        };
        with_state(std, |st| {
            let bounds = nodes::rect_of(&st.g, client);
            if let Some(rec) = st.rec.get_mut(&client) {
                rec.float_rect = Some(bounds);
            }
            if let (Some(margin), true, Some(area)) = (self.snap, event.edge_tiling, event.work_area)
                && let Some(zone) = edge_zone(event.position.x, event.position.y, area, margin)
            {
                st.snap_client(client, zone, &area, true);
            }
        });
    }

    fn cancel(&mut self, std: &Rc<Std>, _event: &DragCtx) {
        if let Some((client, _, start)) = self.drag.take() {
            with_state(std, |st| nodes::write_rect(&st.g, client, &start));
        }
    }
}

/// The sides of a window a resize moves: a corner moves two, an edge one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sides {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

/// `start` resized by `delta` with the sides of `grip` following the pointer, never below `min` or above `max`; a side that is stopped
/// by a limit leaves the opposite side where it was.
fn resized_rect(start: Rect, grip: Sides, delta: Pt, min: (f64, f64), max: Option<(f64, f64)>) -> Rect {
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
pub(super) enum GripChoice {
    Quadrant,
    Nearest,
    Corner(Sides),
}

impl GripChoice {
    pub(super) fn from_member(member: u32) -> GripChoice {
        let corner = |left: bool, top: bool| {
            GripChoice::Corner(Sides {
                left,
                right: !left,
                top,
                bottom: !top,
            })
        };
        match member {
            1 => GripChoice::Nearest,
            2 => corner(true, true),
            3 => corner(false, true),
            4 => corner(true, false),
            5 => corner(false, false),
            _ => GripChoice::Quadrant,
        }
    }
}

fn nearest_edge(at: Pt, width: f64, height: f64) -> Sides {
    let gaps = [
        (
            Sides {
                left: true,
                right: false,
                top: false,
                bottom: false,
            },
            at.x,
        ),
        (
            Sides {
                left: false,
                right: false,
                top: true,
                bottom: false,
            },
            at.y,
        ),
        (
            Sides {
                left: false,
                right: true,
                top: false,
                bottom: false,
            },
            width - at.x,
        ),
        (
            Sides {
                left: false,
                right: false,
                top: false,
                bottom: true,
            },
            height - at.y,
        ),
    ];
    gaps.into_iter()
        .reduce(|best, gap| if gap.1 < best.1 { gap } else { best })
        .map_or(gaps[0].0, |(grip, _)| grip)
}

/// Resizes the window under the pointer by the side or corner the grip names.
pub(super) struct ResizeDrag {
    grip: GripChoice,
    drag: Option<(NodeId, Rect, Pt, Sides)>,
}

impl ResizeDrag {
    pub(super) fn new(grip: GripChoice) -> ResizeDrag {
        ResizeDrag { grip, drag: None }
    }
}

impl DragHandler for ResizeDrag {
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool {
        let Some(client) = event.client.filter(|client| draggable(std, *client)) else {
            return false;
        };
        let start = nodes::rect_of(&std.st.borrow().g, client);
        let at = event.local.unwrap_or(Pt {
            x: event.position.x - start.x,
            y: event.position.y - start.y,
        });
        let grip = match self.grip {
            GripChoice::Quadrant => {
                let top = at.y < start.height / 2.0;
                let left = at.x < start.width / 2.0;
                Sides {
                    left,
                    right: !left,
                    top,
                    bottom: !top,
                }
            }
            GripChoice::Nearest => nearest_edge(at, start.width, start.height),
            GripChoice::Corner(grip) => grip,
        };
        self.drag = Some((client, start, event.position, grip));
        true
    }

    fn update(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let Some((client, start, origin, grip)) = &self.drag else {
            return;
        };
        let delta = Pt {
            x: event.position.x - origin.x,
            y: event.position.y - origin.y,
        };
        with_state(std, |st| {
            // The window's own limits, else the smallest size every resize leaves.
            let rec = st.rec.get(client);
            let min = rec.and_then(|rec| rec.min).unwrap_or(MIN_SIZE);
            let max = rec.and_then(|rec| rec.max);
            nodes::write_rect(&st.g, *client, &resized_rect(*start, *grip, delta, min, max));
        });
    }

    fn end(&mut self, std: &Rc<Std>, _event: &DragCtx) {
        if let Some((client, ..)) = self.drag.take() {
            with_state(std, |st| {
                let bounds = nodes::rect_of(&st.g, client);
                if let Some(rec) = st.rec.get_mut(&client) {
                    rec.float_rect = Some(bounds);
                }
            });
        }
    }

    fn cancel(&mut self, std: &Rc<Std>, _event: &DragCtx) {
        if let Some((client, start, _, _)) = self.drag.take() {
            with_state(std, |st| nodes::write_rect(&st.g, client, &start));
        }
    }
}

/// The tile of `workspace` that holds `at`, other than `except`: the first in the workspace's order that is shown.
fn tile_at(std: &Std, workspace: u64, at: Pt, except: NodeId) -> Option<NodeId> {
    let clients = std.st.borrow().ws.get(&workspace)?.clients.clone();
    clients.into_iter().find(|client| {
        let (rect, shown) = {
            let st = std.st.borrow();
            (
                nodes::rect_of(&st.g, *client),
                !nodes::flag(&st.g, *client, "minimised") && nodes::visible(&st.g, *client),
            )
        };
        *client != except
            && in_tile(std, *client)
            && shown
            && at.x >= rect.x
            && at.x < rect.x + rect.width
            && at.y >= rect.y
            && at.y < rect.y + rect.height
    })
}

/// Swaps a tiled window with the tile it is dropped on: the pointer takes hold of a tile and the release decides, not the last motion.
pub(super) struct SwapDrag {
    drag: Option<(NodeId, u64)>,
}

impl SwapDrag {
    pub(super) fn new() -> SwapDrag {
        SwapDrag { drag: None }
    }
}

impl DragHandler for SwapDrag {
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool {
        let Some(client) = event.client.filter(|client| in_tile(std, *client)) else {
            return false;
        };
        let Some(workspace) = std.st.borrow().rec.get(&client).and_then(|rec| rec.workspace) else {
            return false;
        };
        self.drag = Some((client, workspace));
        true
    }

    fn update(&mut self, _std: &Rc<Std>, _event: &DragCtx) {}

    fn end(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let Some((dragged, workspace)) = self.drag.take() else {
            return;
        };
        // The release decides: the pointer may have moved since the last sample.
        let Some(over) = tile_at(std, workspace, event.position, dragged) else {
            return;
        };
        with_state(std, |st| {
            if st.rec.get(&dragged).and_then(|rec| rec.workspace) == Some(workspace) {
                let _ = st.swap_clients(workspace, dragged, over);
            }
        });
    }

    fn cancel(&mut self, _std: &Rc<Std>, _event: &DragCtx) {
        self.drag = None;
    }
}

/// One divider of a drag, as a layout described it.
#[derive(Clone, Copy, Debug)]
struct DragDivider {
    y_axis: bool,
    direction: f64,
    span: f64,
    min: f64,
    max: f64,
    ratio: f64,
}

impl DragDivider {
    /// The ratio the pointer's travel from `start` to `at` carries the divider to, from `start_ratio`, held to `min..=max`.
    fn follow(&self, start_ratio: f64, start: Pt, at: Pt) -> f64 {
        let travel = if self.y_axis { at.y - start.y } else { at.x - start.x };
        (start_ratio + self.direction * travel / self.span).clamp(self.min, self.max)
    }
}

/// What a split-resize drag keeps from its start.
struct SplitDrag {
    workspace: u64,
    token: u64,
    divider: DragDivider,
    cross: Option<DragDivider>,
    start: Pt,
}

/// Moves the divider of a tiling layout nearest the press by as far as the pointer travels. See the `split_resize` binding kind.
pub(super) struct SplitResizeDrag {
    drag: Option<SplitDrag>,
}

impl SplitResizeDrag {
    pub(super) fn new() -> SplitResizeDrag {
        SplitResizeDrag { drag: None }
    }
}

fn divider_of(axis: u32, direction: i32, span: f64, min: f64, max: f64, ratio: f64) -> DragDivider {
    DragDivider {
        y_axis: axis == STD_AXIS_Y,
        direction: f64::from(direction.signum()),
        span,
        min,
        max,
        ratio,
    }
}

fn parse_split(raw: &GnoblinStdSplit) -> (DragDivider, Option<DragDivider>) {
    let main = divider_of(raw.axis, raw.direction, raw.span, raw.min, raw.max, raw.ratio);
    let cross = (raw.flags & STD_SPLIT_CROSS != 0).then(|| {
        divider_of(
            raw.cross_axis,
            raw.cross_direction,
            raw.cross_span,
            raw.cross_min,
            raw.cross_max,
            raw.cross_ratio,
        )
    });
    (main, cross.filter(|cross| cross.span > 0.0))
}

fn layout_of(std: &Std, workspace: u64) -> Option<Rc<crate::layout::LayoutObj>> {
    std.st.borrow().ws.get(&workspace).map(|ws| Rc::clone(&ws.layout))
}

impl DragHandler for SplitResizeDrag {
    fn start(&mut self, std: &Rc<Std>, event: &DragCtx) -> bool {
        let Some(client) = event.client.filter(|client| in_tile(std, *client)) else {
            return false;
        };
        let Some(workspace) = std.st.borrow().rec.get(&client).and_then(|rec| rec.workspace) else {
            return false;
        };
        // The layout answers outside the library's borrow of its state.
        let Some(raw) =
            layout_of(std, workspace).and_then(|layout| layout.find_split(client, event.position.x, event.position.y))
        else {
            return false;
        };
        let (divider, cross) = parse_split(&raw);
        if divider.span <= 0.0 {
            return false;
        }
        self.drag = Some(SplitDrag {
            workspace,
            token: raw.token,
            divider,
            cross,
            start: event.position,
        });
        true
    }

    fn update(&mut self, std: &Rc<Std>, event: &DragCtx) {
        let Some(drag) = &self.drag else {
            return;
        };
        let ratio = drag.divider.follow(drag.divider.ratio, drag.start, event.position);
        let cross = drag
            .cross
            .map(|cross| cross.follow(cross.ratio, drag.start, event.position));
        if let Some(layout) = layout_of(std, drag.workspace) {
            layout.set_split(drag.token, ratio, cross.unwrap_or(f64::NAN));
            with_state(std, |st| {
                if let Some(ws) = st.ws.get_mut(&drag.workspace) {
                    ws.dirty = true;
                }
            });
        }
    }

    fn end(&mut self, _std: &Rc<Std>, _event: &DragCtx) {
        self.drag = None;
    }

    fn cancel(&mut self, std: &Rc<Std>, _event: &DragCtx) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Some(layout) = layout_of(std, drag.workspace) {
            layout.reset_split(drag.token);
            with_state(std, |st| {
                if let Some(ws) = st.ws.get_mut(&drag.workspace) {
                    ws.dirty = true;
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect::new(x, y, width, height)
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
        let grip = Sides {
            left: true,
            right: false,
            top: false,
            bottom: true,
        };
        // Pulled left by 100: the left side follows. Pushed right by 1000: the left side stops at the minimum width.
        let wider = resized_rect(start, grip, Pt { x: -100.0, y: 0.0 }, MIN_SIZE, None);
        assert_eq!((wider.x, wider.width), (200.0, 500.0));
        let narrow = resized_rect(start, grip, Pt { x: 1000.0, y: 0.0 }, MIN_SIZE, None);
        assert_eq!((narrow.x, narrow.width), (600.0, 100.0));
        let capped = resized_rect(start, grip, Pt { x: -500.0, y: 0.0 }, MIN_SIZE, Some((600.0, 400.0)));
        assert_eq!((capped.x, capped.width), (100.0, 600.0));
    }

    #[test]
    fn a_divider_follows_the_pointer_along_its_axis_and_stays_in_its_range() {
        let divider = divider_of(0, 1, 1000.0, 0.1, 0.9, 0.5);
        let start = Pt { x: 500.0, y: 100.0 };
        assert_eq!(divider.follow(0.5, start, Pt { x: 700.0, y: 900.0 }), 0.7);
        assert_eq!(divider.follow(0.5, start, Pt { x: 9000.0, y: 0.0 }), 0.9);
        let back = divider_of(0, -1, 1000.0, 0.1, 0.9, 0.5);
        assert_eq!(back.follow(0.5, start, Pt { x: 700.0, y: 0.0 }), 0.3);
    }
}
