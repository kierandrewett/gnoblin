//! `floating_place`: where a new window goes among the windows that are already there, for a shell that keeps its own windows.
//!
//! The floating layout of a workspace places a window once when it joins (`layout::place_floating`); this is the same arithmetic
//! (`floating::place_rect`, `floating::apart_from`) with the windows and the area handed in, so a shell that does its own workspace
//! gets the library's placement, once, for every language. A call reads nothing of the library's state and never starts its desktop.
#![allow(unsafe_code)]

use gnoblin_core::host::Rect;
use gnoblin_core_abi::{self as abi, GnoblinCtx};
use gnoblin_std_abi::{GnoblinStdFloatingQuery, GnoblinStdRect, STD_FLOAT_APART};

use crate::error::Fail;
use crate::floating::{PlaceMode, apart_from, place_rect};
use crate::table::{call, put, read_ext};

/// The cascade step a query of zero asks for.
const DEFAULT_STEP: f64 = 32.0;

fn rect_in(raw: &GnoblinStdRect) -> Rect {
    Rect::new(raw.x, raw.y, raw.width, raw.height)
}

pub(crate) unsafe extern "C" fn floating_place(
    _ctx: GnoblinCtx,
    query: *const GnoblinStdFloatingQuery,
    out: *mut GnoblinStdRect,
) -> i32 {
    call("floating_place", || {
        // SAFETY: the caller guarantees `query` is null or a `GnoblinStdFloatingQuery` whose `occupied` points at `n_occupied` records.
        let raw = unsafe { read_ext(query, "floating query") }?;
        let mode = PlaceMode::from_abi(raw.mode)
            .ok_or_else(|| Fail::type_error(format!("unknown placement {}: the placements are 0 to 2", raw.mode)))?;
        let occupied: Vec<Rect> = if raw.occupied.is_null() || raw.n_occupied == 0 {
            Vec::new()
        } else {
            // SAFETY: the caller guarantees `n_occupied` readable rectangles at `occupied`.
            unsafe { std::slice::from_raw_parts(raw.occupied, raw.n_occupied) }
                .iter()
                .map(rect_in)
                .collect()
        };
        let area = rect_in(&raw.area);
        let numbers = [raw.step, raw.width, raw.height, area.x, area.y, area.width, area.height];
        let finite = numbers.iter().all(|n| n.is_finite())
            && occupied
                .iter()
                .all(|r| [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite()));
        if !finite {
            return Err(Fail::type_error(
                "a placement's size, step, area and neighbours must be finite numbers",
            ));
        }
        let step = if raw.step == 0.0 { DEFAULT_STEP } else { raw.step };
        let spot = place_rect(mode, raw.width, raw.height, &area, &occupied, step);
        let placed = if raw.flags & STD_FLOAT_APART != 0 {
            apart_from(&spot, &occupied, &area, step)
        } else {
            spot
        };
        // SAFETY: the caller guarantees `out` is null or writable.
        unsafe {
            put(
                out,
                GnoblinStdRect {
                    x: placed.x,
                    y: placed.y,
                    width: placed.width,
                    height: placed.height,
                },
            )
        }?;
        Ok(abi::E_OK)
    })
}
