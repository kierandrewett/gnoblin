//! The built-in layout used when a Lua layout declines.

use super::placement::{Camera, LayoutEffect, Origin, Params, Placement};
use super::settle::Layout;

/// Distance between successive cascaded windows, in logical units.
const CASCADE_STEP: f64 = 32.0;

/// Centres each window on the output and offsets each one from the last.
///
/// Every window keeps its current size and is placed, with no float effects,
/// through an orthographic camera. The result is sorted by window id, and the
/// offset depends only on the window's position in `params`, so the same
/// params always give the same layout.
pub fn fallback(params: &Params) -> Layout {
    let mut placements: Vec<Placement> = params
        .windows()
        .iter()
        .enumerate()
        .map(|(index, window)| {
            let step = CASCADE_STEP * index as f64;
            let x = (params.output_width() - window.width) / 2.0 + step;
            let y = (params.output_height() - window.height) / 2.0 + step;
            Placement::rect(window.id, x, y, window.width, window.height)
        })
        .collect();
    placements.sort_by_key(|placement| placement.window);
    Layout {
        placements,
        camera: Camera::Orthographic,
        effects: Vec::<LayoutEffect>::new(),
        origin: Origin::Fallback,
    }
}
