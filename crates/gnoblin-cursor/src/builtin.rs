//! The built-in arrow: the pointer of last resort, drawn in code so the pointer
//! is never invisible, even with no theme installed.
//!
//! The outline is designed on a 24 by 24 unit grid and scaled to the physical
//! size, so one unit is one pixel at the default size. Each pixel is averaged
//! over a 4 by 4 supersample grid, which gives antialiased edges.

use crate::{CursorFrame, CursorImages, MAX_PHYSICAL_SIZE, premultiplied};

/// The edge of the design grid, in units.
const DESIGN: f64 = 24.0;

/// The width of the black outline, in units.
const OUTLINE: f64 = 1.0;

/// Supersamples along one edge of a pixel.
const SAMPLES: u32 = 4;

/// The arrow outline in design units, closed back to the first point. The
/// first point is the tip, which is the hotspot.
const OUTLINE_POINTS: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.0, 17.0),
    (4.5, 13.0),
    (7.5, 20.0),
    (10.0, 19.0),
    (7.0, 12.0),
    (13.0, 12.0),
];

/// The built-in arrow drawn at `physical` pixels square, with its hotspot at
/// the tip. The size is clamped to `1..=MAX_PHYSICAL_SIZE`.
#[must_use]
pub fn arrow(physical: u32) -> CursorImages {
    let edge = physical.clamp(1, MAX_PHYSICAL_SIZE);
    let unit = DESIGN / f64::from(edge);
    let mut pixels = Vec::with_capacity((edge * edge) as usize);
    for y in 0..edge {
        for x in 0..edge {
            pixels.push(shade(x, y, unit));
        }
    }
    // A square frame of at most MAX_PHYSICAL_SIZE pixels and one hotspot inside
    // it always satisfy the image limits, so these cannot fail.
    let frame = CursorFrame::new(edge, edge, pixels, 0).expect("the built-in arrow is a valid square frame");
    CursorImages::new((0, 0), vec![frame]).expect("the built-in arrow is a valid one-frame cursor")
}

/// The premultiplied colour of pixel `(x, y)`: white inside the outline, black
/// on it, and transparent outside, averaged over the supersamples.
fn shade(x: u32, y: u32, unit: f64) -> u32 {
    let mut covered = 0_u32;
    let mut white = 0_u32;
    for sample_y in 0..SAMPLES {
        for sample_x in 0..SAMPLES {
            let point = (
                (f64::from(x) + (f64::from(sample_x) + 0.5) / f64::from(SAMPLES)) * unit,
                (f64::from(y) + (f64::from(sample_y) + 0.5) / f64::from(SAMPLES)) * unit,
            );
            if !inside(point) {
                continue;
            }
            covered += 1;
            if edge_distance(point) >= OUTLINE {
                white += 1;
            }
        }
    }
    if covered == 0 {
        return 0;
    }
    let samples = SAMPLES * SAMPLES;
    let alpha = (covered * 255 + samples / 2) / samples;
    let grey = u8::try_from(white * 255 / covered).unwrap_or(u8::MAX);
    premultiplied(grey, grey, grey, u8::try_from(alpha).unwrap_or(u8::MAX))
}

/// Whether `point` is inside the outline, by counting edge crossings to its right.
fn inside(point: (f64, f64)) -> bool {
    let mut inside = false;
    let mut previous = OUTLINE_POINTS[OUTLINE_POINTS.len() - 1];
    for current in OUTLINE_POINTS {
        let (x1, y1) = previous;
        let (x2, y2) = current;
        if (y1 > point.1) != (y2 > point.1) && point.0 < (x2 - x1) * (point.1 - y1) / (y2 - y1) + x1 {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

/// The distance from `point` to the nearest edge of the outline.
fn edge_distance(point: (f64, f64)) -> f64 {
    let mut nearest = f64::MAX;
    let mut previous = OUTLINE_POINTS[OUTLINE_POINTS.len() - 1];
    for current in OUTLINE_POINTS {
        nearest = nearest.min(segment_distance(point, previous, current));
        previous = current;
    }
    nearest
}

/// The distance from `point` to the segment from `start` to `end`.
fn segment_distance(point: (f64, f64), start: (f64, f64), end: (f64, f64)) -> f64 {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((point.0 - start.0) * dx + (point.1 - start.1) * dy) / length_squared).clamp(0.0, 1.0)
    };
    let (closest_x, closest_y) = (start.0 + t * dx, start.1 + t * dy);
    ((point.0 - closest_x).powi(2) + (point.1 - closest_y).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_arrow_scales_with_the_physical_size() {
        let small = arrow(24);
        let large = arrow(48);
        assert_eq!((small.width(), small.height()), (24, 24));
        assert_eq!((large.width(), large.height()), (48, 48));
        assert_eq!(small.hotspot(), (0, 0));
        assert_eq!(large.hotspot(), (0, 0));
    }

    #[test]
    fn the_tip_is_opaque_and_the_corner_is_transparent() {
        let images = arrow(24);
        let frame = &images.frames()[0];
        assert_ne!(frame.pixels()[0] >> 24, 0, "the tip pixel is covered");
        assert_eq!(frame.pixels()[(24 * 24 - 1) as usize], 0, "the far corner is empty");
    }
}
