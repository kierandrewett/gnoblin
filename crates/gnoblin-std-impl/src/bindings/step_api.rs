//! The table function of the drag rules (`steps.rs`), and the pure geometry of drags and gestures: what a shell that matches its own
//! pointer events needs to get the library's drag behaviour without handing the library the chord.
//!
//! `binding_builtin` claims a chord and runs a drag from the press; `drag_step` leaves the claim, the press, the threshold and the
//! choice among bindings to the caller (the TypeScript engine matches them in `gnoblin-core/input`, with its tiers and its layout
//! scopes) and gives it the drag's rules: it is told one step of a drag and answers what is to be written to the caller's own window,
//! tile and divider objects. It is a function of its inputs. What a drag keeps between its steps (where the window was, which sides a
//! resize moves, the divider as it was found) the caller keeps, and gives back.
//!
//! None of these needs `open`: they read nothing of the library's state and touch nothing in the compositor's tree, so a shell that
//! keeps its own workspaces does not get a second window manager beside its own by calling them.
#![allow(unsafe_code)]

use gnoblin_core::host::{GestureKind, Rect};
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_std_abi::layouts::binding_kind;
use gnoblin_std_abi::{
    GnoblinStdClientState, GnoblinStdDragEvent, GnoblinStdDragOutcome, GnoblinStdDragStep, GnoblinStdDragTile,
    GnoblinStdOption, GnoblinStdRect, STD_CLIENT_HAS_MAX, STD_CLIENT_HAS_MIN, STD_DRAG_EDGE_TILING, STD_DRAG_HAS_LOCAL,
    STD_DRAG_HAS_WORK_AREA, STD_GESTURE_PINCH, STD_GESTURE_SWIPE, STD_OUTCOME_FORGET_SNAP, STD_OUTCOME_HAS_BOUNDS,
    STD_OUTCOME_HAS_CROSS_RATIO, STD_OUTCOME_HAS_FLOATING, STD_OUTCOME_HAS_RATIO, STD_OUTCOME_HAS_SIDES,
    STD_OUTCOME_HAS_SWAP, STD_OUTCOME_RESET_SPLIT, STD_OUTCOME_TOOK, STD_SNAP_NONE,
};

use super::DragCtx;
use super::gesture::way_past;
use super::steps::{
    ClientView, DragAnswer, DragInput, DragRule, MIN_SIZE, Sides, SplitView, Tile, clamp_axis, edge_zone, resized_rect,
};
use crate::builtin::{Pt, read_set};
use crate::client_state::tile_rect;
use crate::error::{Fail, Res};
use crate::floating::Inset;
use crate::table::{call, fill_out, put, read_ext};

fn rect_in(raw: &GnoblinStdRect) -> Rect {
    Rect::new(raw.x, raw.y, raw.width, raw.height)
}

fn rect_out(rect: &Rect) -> GnoblinStdRect {
    GnoblinStdRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn check_finite(values: &[f64], what: &str) -> Res<()> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(Fail::type_error(format!("{what} must be finite numbers")))
    }
}

/// What the step says of the pointer and the press, as the library's own drag context.
fn context_of(event: &GnoblinStdDragEvent) -> Res<DragCtx> {
    check_finite(
        &[
            event.x,
            event.y,
            event.origin_x,
            event.origin_y,
            event.local_x,
            event.local_y,
            event.work_area.x,
            event.work_area.y,
            event.work_area.width,
            event.work_area.height,
        ],
        "a drag's pointer, press and work area",
    )?;
    Ok(DragCtx {
        client: (event.client != 0).then_some(event.client),
        position: Pt { x: event.x, y: event.y },
        origin: Pt {
            x: event.origin_x,
            y: event.origin_y,
        },
        local: (event.flags & STD_DRAG_HAS_LOCAL != 0).then_some(Pt {
            x: event.local_x,
            y: event.local_y,
        }),
        button: event.button,
        work_area: (event.flags & STD_DRAG_HAS_WORK_AREA != 0).then(|| rect_in(&event.work_area)),
        constraint: event.constraint,
        edge_tiling: event.flags & STD_DRAG_EDGE_TILING != 0,
    })
}

/// What the step says of the pressed window: its flags, rectangle, frame inset and limits.
fn view_of(client_flags: u32, bounds: &GnoblinStdRect, state: Option<&GnoblinStdClientState>) -> Res<ClientView> {
    check_finite(
        &[bounds.x, bounds.y, bounds.width, bounds.height],
        "a client's rectangle",
    )?;
    let mut view = ClientView {
        flags: client_flags,
        bounds: rect_in(bounds),
        inset: Inset::default(),
        min: None,
        max: None,
    };
    if let Some(state) = state {
        check_finite(
            &[
                state.inset_top,
                state.inset_right,
                state.inset_bottom,
                state.inset_left,
                state.min_width,
                state.min_height,
                state.max_width,
                state.max_height,
            ],
            "a client's frame inset and limits",
        )?;
        view.inset = Inset {
            top: state.inset_top,
            right: state.inset_right,
            bottom: state.inset_bottom,
            left: state.inset_left,
        };
        view.min = (state.flags & STD_CLIENT_HAS_MIN != 0).then_some((state.min_width, state.min_height));
        view.max = (state.flags & STD_CLIENT_HAS_MAX != 0).then_some((state.max_width, state.max_height));
    }
    Ok(view)
}

fn outcome_out(outcome: &DragAnswer) -> GnoblinStdDragOutcome {
    let mut flags = 0;
    let mut set = |on: bool, flag: u32| {
        if on {
            flags |= flag;
        }
    };
    set(outcome.took, STD_OUTCOME_TOOK);
    set(outcome.bounds.is_some(), STD_OUTCOME_HAS_BOUNDS);
    set(outcome.floating.is_some(), STD_OUTCOME_HAS_FLOATING);
    set(outcome.forget_snap, STD_OUTCOME_FORGET_SNAP);
    set(outcome.swap.is_some(), STD_OUTCOME_HAS_SWAP);
    set(outcome.ratio.is_some(), STD_OUTCOME_HAS_RATIO);
    set(outcome.cross_ratio.is_some(), STD_OUTCOME_HAS_CROSS_RATIO);
    set(outcome.reset_split, STD_OUTCOME_RESET_SPLIT);
    set(outcome.sides.is_some(), STD_OUTCOME_HAS_SIDES);
    let nothing = Rect::new(0.0, 0.0, 0.0, 0.0);
    GnoblinStdDragOutcome {
        size: size_of::<GnoblinStdDragOutcome>() as u32,
        flags,
        snap_zone: outcome.snap_zone.unwrap_or(STD_SNAP_NONE),
        reserved: 0,
        bounds: rect_out(&outcome.bounds.unwrap_or(nothing)),
        floating: rect_out(&outcome.floating.unwrap_or(nothing)),
        swap_with: outcome.swap.unwrap_or(0),
        sides: outcome.sides.map_or(0, Sides::flags),
        reserved2: 0,
        ratio: outcome.ratio.unwrap_or(0.0),
        cross_ratio: outcome.cross_ratio.unwrap_or(0.0),
    }
}

pub(crate) unsafe extern "C" fn drag_step(
    _ctx: GnoblinCtx,
    kind: u32,
    options: *const GnoblinStdOption,
    n_options: usize,
    step: *const GnoblinStdDragStep,
    out: *mut GnoblinStdDragOutcome,
) -> i32 {
    call("drag_step", || {
        let desc =
            binding_kind(kind).ok_or_else(|| Fail::type_error(format!("unknown built-in binding kind {kind}")))?;
        // SAFETY: the caller guarantees `options` points at `n_options` records.
        let options = unsafe { read_set(desc.name, desc.options, options, n_options) }?;
        let rule = DragRule::of(kind, &options)
            .ok_or_else(|| Fail::type_error(format!("unknown built-in binding kind {kind}")))?;
        // SAFETY: the caller guarantees `step` is a `GnoblinStdDragStep` whose first field is its size.
        let raw = unsafe { read_ext(step, "drag step") }?;
        // SAFETY: the caller guarantees `event` points at a `GnoblinStdDragEvent`.
        let event = unsafe { read_ext(raw.event, "drag event") }?;
        let context = context_of(&event)?;
        let state = if raw.state.is_null() {
            None
        } else {
            // SAFETY: the caller guarantees a non-null `state` is a `GnoblinStdClientState`.
            Some(unsafe { read_ext(raw.state, "client state") }?)
        };
        let client = context
            .client
            .map(|_| view_of(raw.client_flags, &raw.bounds, state.as_ref()))
            .transpose()?;
        check_finite(
            &[raw.start.x, raw.start.y, raw.start.width, raw.start.height],
            "a drag's starting rectangle",
        )?;
        let split = if raw.split.is_null() {
            None
        } else {
            // SAFETY: the caller guarantees a non-null `split` is a `GnoblinStdSplit`.
            let split = unsafe { read_ext(raw.split, "split") }?;
            // A divider that is not a number has nothing to follow: the binding declines.
            [
                split.span,
                split.min,
                split.max,
                split.ratio,
                split.cross_span,
                split.cross_min,
                split.cross_max,
                split.cross_ratio,
            ]
            .iter()
            .all(|number| number.is_finite())
            .then(|| SplitView::of(&split))
        };
        let tiles: Vec<Tile> = if raw.tiles.is_null() || raw.n_tiles == 0 {
            Vec::new()
        } else {
            // SAFETY: the caller guarantees `n_tiles` readable tiles at `tiles`.
            let raw_tiles: &[GnoblinStdDragTile] = unsafe { std::slice::from_raw_parts(raw.tiles, raw.n_tiles) };
            raw_tiles
                .iter()
                .map(|tile| Tile {
                    id: tile.id,
                    flags: tile.flags,
                    rect: rect_in(&tile.rect),
                })
                .collect()
        };
        let outcome = rule.step(
            event.phase,
            &DragInput {
                event: &context,
                client,
                start: rect_in(&raw.start),
                sides: Sides::from_flags(raw.sides),
                split,
                tiles: &tiles,
            },
        );
        // SAFETY: the caller guarantees `out` is a writable `GnoblinStdDragOutcome` whose first field is its size.
        unsafe { fill_out(out, &outcome_out(&outcome), "drag outcome") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn drag_clamp(
    _ctx: GnoblinCtx,
    pos: f64,
    size: f64,
    start: f64,
    span: f64,
    constraint: u32,
    out: *mut f64,
) -> i32 {
    call("drag_clamp", || {
        check_finite(&[pos, size, start, span], "a position, a size and an area")?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, clamp_axis(pos, size, start, span, constraint)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn drag_resize(
    _ctx: GnoblinCtx,
    start: *const GnoblinStdRect,
    sides: u32,
    dx: f64,
    dy: f64,
    limits: *const GnoblinStdClientState,
    out: *mut GnoblinStdRect,
) -> i32 {
    call("drag_resize", || {
        // SAFETY: the caller guarantees `start` is null or a readable `GnoblinStdRect`.
        let start = unsafe { start.as_ref() }.ok_or_else(|| Fail::err("drag_resize: a null rectangle"))?;
        check_finite(
            &[start.x, start.y, start.width, start.height, dx, dy],
            "a rectangle and a travel",
        )?;
        let (mut min, mut max) = (MIN_SIZE, None);
        if !limits.is_null() {
            // SAFETY: the caller guarantees a non-null `limits` is a `GnoblinStdClientState`.
            let state = unsafe { read_ext(limits, "client state") }?;
            check_finite(
                &[state.min_width, state.min_height, state.max_width, state.max_height],
                "a client's limits",
            )?;
            if state.flags & STD_CLIENT_HAS_MIN != 0 {
                min = (state.min_width, state.min_height);
            }
            if state.flags & STD_CLIENT_HAS_MAX != 0 {
                max = Some((state.max_width, state.max_height));
            }
        }
        let resized = resized_rect(rect_in(start), Sides::from_flags(sides), Pt { x: dx, y: dy }, min, max);
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, rect_out(&resized)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn drag_zone(
    _ctx: GnoblinCtx,
    x: f64,
    y: f64,
    area: *const GnoblinStdRect,
    margin: f64,
    out: *mut u32,
) -> i32 {
    call("drag_zone", || {
        // SAFETY: the caller guarantees `area` is null or a readable `GnoblinStdRect`.
        let area = unsafe { area.as_ref() }.ok_or_else(|| Fail::err("drag_zone: a null area"))?;
        check_finite(
            &[x, y, area.x, area.y, area.width, area.height, margin],
            "a point, an area and a margin",
        )?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, edge_zone(x, y, rect_in(area), margin).unwrap_or(STD_SNAP_NONE)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn drag_tile(
    _ctx: GnoblinCtx,
    zone: u32,
    area: *const GnoblinStdRect,
    out: *mut GnoblinStdRect,
) -> i32 {
    call("drag_tile", || {
        // SAFETY: the caller guarantees `area` is null or a readable `GnoblinStdRect`.
        let area = unsafe { area.as_ref() }.ok_or_else(|| Fail::err("drag_tile: a null area"))?;
        check_finite(&[area.x, area.y, area.width, area.height], "an area")?;
        let Some(tile) = tile_rect(zone, &rect_in(area)) else {
            return Ok(0);
        };
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, rect_out(&tile)) }?;
        Ok(1)
    })
}

pub(crate) unsafe extern "C" fn gesture_way(
    _ctx: GnoblinCtx,
    kind: u32,
    dx: f64,
    dy: f64,
    scale: f64,
    threshold: f64,
    out: *mut u32,
) -> i32 {
    call("gesture_way", || {
        let kind = match kind {
            STD_GESTURE_SWIPE => GestureKind::Swipe,
            STD_GESTURE_PINCH => GestureKind::Pinch,
            other => return Err(Fail::type_error(format!("unknown gesture kind {other}"))),
        };
        check_finite(&[dx, dy, scale, threshold], "a gesture's travel, scale and threshold")?;
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe { put(out, way_past(kind, dx, dy, scale, threshold)) }?;
        Ok(abi::E_OK)
    })
}
