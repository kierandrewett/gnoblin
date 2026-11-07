//! Which edges of a tiled window are tiled, for the `xdg_toplevel` `tiled_*` states.
//!
//! A tiled edge is one that sits on the work-area boundary, or that faces a neighbouring
//! tile. A neighbour faces an edge when it lies entirely beyond that edge and overlaps the
//! window along the perpendicular axis. Gaps between tiles do not break the rule: a tile
//! with a gap to its neighbour still has a tiled edge there, as niri and sway treat it. An
//! outer gap does break it, because the edge then touches neither the work area nor a tile.
//!
//! The function is pure: it takes the rectangles and returns the edges. Callers pass only
//! the other tiled windows of the same run; floating, maximised, fullscreen and minimised
//! windows are not neighbours.

use gnoblin_types::Rect;

/// How far, in logical pixels, an edge may sit from the work-area boundary and still count
/// as on it. The slack absorbs rounding in the placement arithmetic.
pub const WORK_AREA_SLACK: i64 = 1;

/// The four edges of a window, each true when that edge is tiled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TiledEdges {
    /// The left edge is tiled.
    pub left: bool,
    /// The right edge is tiled.
    pub right: bool,
    /// The top edge is tiled.
    pub top: bool,
    /// The bottom edge is tiled.
    pub bottom: bool,
}

impl TiledEdges {
    /// No edge is tiled: the state of a floating, kiosk or otherwise untiled window.
    pub const NONE: Self = Self {
        left: false,
        right: false,
        top: false,
        bottom: false,
    };

    /// Every edge is tiled: a window that fills a full-coverage layout.
    pub const ALL: Self = Self {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };
}

/// The tiled edges of `rect`, given the other tiled windows of its layout run and the work
/// area of its output.
///
/// `tiling` is true when the window is placed by a tiling layout. A window that is not
/// tiled gets [`TiledEdges::NONE`] whatever its geometry, so the caller can pass the flag
/// it already has without a separate branch.
#[must_use]
pub fn tiled_edges(tiling: bool, rect: Rect, neighbours: &[Rect], work_area: Rect) -> TiledEdges {
    if !tiling {
        return TiledEdges::NONE;
    }
    let (left, top) = (i64::from(rect.x), i64::from(rect.y));
    let (right, bottom) = (rect.right(), rect.bottom());
    let (work_left, work_top) = (i64::from(work_area.x), i64::from(work_area.y));
    let (work_right, work_bottom) = (work_area.right(), work_area.bottom());
    // Whether an edge at `edge` sits on the work-area boundary at `boundary`.
    let on_work_area = |edge: i64, boundary: i64| (edge - boundary).abs() <= WORK_AREA_SLACK;
    // Whether a neighbour overlaps the window along the vertical axis.
    let rows_overlap = |other: &Rect| i64::from(other.y) < bottom && other.bottom() > top;
    // Whether a neighbour overlaps the window along the horizontal axis.
    let columns_overlap = |other: &Rect| i64::from(other.x) < right && other.right() > left;
    TiledEdges {
        left: on_work_area(left, work_left)
            || neighbours
                .iter()
                .any(|other| other.right() <= left && rows_overlap(other)),
        right: on_work_area(right, work_right)
            || neighbours
                .iter()
                .any(|other| i64::from(other.x) >= right && rows_overlap(other)),
        top: on_work_area(top, work_top)
            || neighbours
                .iter()
                .any(|other| other.bottom() <= top && columns_overlap(other)),
        bottom: on_work_area(bottom, work_bottom)
            || neighbours
                .iter()
                .any(|other| i64::from(other.y) >= bottom && columns_overlap(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1000 by 800 work area at the origin.
    fn work() -> Rect {
        Rect::new(0, 0, 1000, 800)
    }

    /// One table row: a name, the window, its neighbours, the work area, and the answer.
    struct Case {
        name: &'static str,
        tiling: bool,
        rect: Rect,
        neighbours: Vec<Rect>,
        work: Rect,
        expected: TiledEdges,
    }

    /// Builds the edges set from the four flags, in left, right, top, bottom order.
    fn edges(left: bool, right: bool, top: bool, bottom: bool) -> TiledEdges {
        TiledEdges {
            left,
            right,
            top,
            bottom,
        }
    }

    #[test]
    fn tiled_edges_table() {
        let cases = [
            Case {
                name: "single window fills the work area",
                tiling: true,
                rect: Rect::new(0, 0, 1000, 800),
                neighbours: vec![],
                work: work(),
                expected: TiledEdges::ALL,
            },
            Case {
                name: "single window with an outer gap touches no edge",
                tiling: true,
                rect: Rect::new(8, 8, 984, 784),
                neighbours: vec![],
                work: work(),
                expected: TiledEdges::NONE,
            },
            Case {
                name: "a window on the work-area left edge only",
                tiling: true,
                rect: Rect::new(0, 8, 300, 784),
                neighbours: vec![],
                work: work(),
                expected: edges(true, false, false, false),
            },
            Case {
                name: "left half beside its right-hand neighbour",
                tiling: true,
                rect: Rect::new(0, 0, 500, 800),
                neighbours: vec![Rect::new(500, 0, 500, 800)],
                work: work(),
                expected: TiledEdges::ALL,
            },
            Case {
                name: "right half beside its left-hand neighbour",
                tiling: true,
                rect: Rect::new(500, 0, 500, 800),
                neighbours: vec![Rect::new(0, 0, 500, 800)],
                work: work(),
                expected: TiledEdges::ALL,
            },
            Case {
                name: "a gap between two columns keeps the facing edges tiled",
                tiling: true,
                rect: Rect::new(0, 0, 496, 800),
                neighbours: vec![Rect::new(504, 0, 496, 800)],
                work: work(),
                expected: TiledEdges::ALL,
            },
            Case {
                name: "a neighbour in another row does not tile the side",
                tiling: true,
                rect: Rect::new(0, 0, 500, 400),
                neighbours: vec![Rect::new(500, 400, 500, 400)],
                work: work(),
                expected: edges(true, false, true, false),
            },
            Case {
                name: "a 2x2 grid with gaps tiles every edge",
                tiling: true,
                rect: Rect::new(0, 0, 496, 396),
                neighbours: vec![
                    Rect::new(504, 0, 496, 396),
                    Rect::new(0, 404, 496, 396),
                    Rect::new(504, 404, 496, 396),
                ],
                work: work(),
                expected: TiledEdges::ALL,
            },
            Case {
                name: "the bottom-right tile of a grid touches the work area on two sides",
                tiling: true,
                rect: Rect::new(504, 404, 496, 396),
                neighbours: vec![
                    Rect::new(0, 0, 496, 396),
                    Rect::new(504, 0, 496, 396),
                    Rect::new(0, 404, 496, 396),
                ],
                work: work(),
                expected: edges(true, true, true, true),
            },
            Case {
                name: "a window on a work area with a reserved top strip",
                tiling: true,
                rect: Rect::new(1920, 32, 960, 1048),
                neighbours: vec![],
                work: Rect::new(1920, 32, 1920, 1048),
                expected: edges(true, false, true, true),
            },
            Case {
                name: "one pixel of rounding still counts as the work-area edge",
                tiling: true,
                rect: Rect::new(1, 0, 999, 800),
                neighbours: vec![],
                work: work(),
                expected: edges(true, true, true, true),
            },
            Case {
                name: "a floating window gets no tiled edges",
                tiling: false,
                rect: Rect::new(0, 0, 1000, 800),
                neighbours: vec![Rect::new(500, 0, 500, 800)],
                work: work(),
                expected: TiledEdges::NONE,
            },
        ];
        for case in cases {
            let got = tiled_edges(case.tiling, case.rect, &case.neighbours, case.work);
            assert_eq!(got, case.expected, "case: {}", case.name);
        }
    }
}
