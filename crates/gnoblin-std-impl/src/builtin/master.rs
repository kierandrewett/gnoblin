//! The master-stack, grid and monocle layouts: ports of `data/js/gnoblin-std/layouts/{master_stack,grid,monocle}.ts`, which are the
//! reference. The same arithmetic, the same clamps, the same flooring of every tile edge.

use super::{DividerSpec, Imp, OptionSet, Pt, SplitSpec, Val, clamp, geometry, tile};
use crate::layout::{LayoutContext, Placement};
use crate::state::NodeId;

/// The smallest and largest share of the span the master column may take.
const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

/// A tile as `[u, v, p, q]`: `u` and `p` along the span the master and the stack divide, `v` and `q` across it.
type Frame = [f64; 4];

fn master_stack_frames(span: f64, across: f64, n: usize, master: usize, ratio: f64, gap: f64) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(n);
    let m = master.min(n);
    let s = n - m;
    let mut master_width = 0.0;
    if m > 0 {
        master_width = if s == 0 { span } else { ((span - gap) * ratio).max(0.0) };
        let h = ((across - (m - 1) as f64 * gap) / m as f64).max(0.0);
        for i in 0..m {
            frames.push([0.0, i as f64 * (h + gap), master_width, h]);
        }
    }
    if s > 0 {
        let start = if m > 0 { master_width + gap } else { 0.0 };
        let w = (span - start).max(0.0);
        let h = ((across - (s - 1) as f64 * gap) / s as f64).max(0.0);
        for i in 0..s {
            frames.push([start, i as f64 * (h + gap), w, h]);
        }
    }
    frames
}

/// One master area and a stack.
pub(super) struct MasterStack {
    opts: OptionSet,
    /// The windows of the last compute, how many are master, and the span the master and the stack divide.
    last: Option<(Vec<u64>, usize, f64)>,
    /// The drag in flight: its token and the ratio it started from.
    drag: Option<(u64, f64)>,
    token: u64,
}

impl MasterStack {
    pub(super) fn new(opts: OptionSet) -> MasterStack {
        MasterStack {
            opts,
            last: None,
            drag: None,
            token: 0,
        }
    }

    fn orientation(&self) -> u32 {
        self.opts.member("orientation")
    }

    fn sideways(&self) -> bool {
        // top, bottom
        matches!(self.orientation(), 2 | 3)
    }
}

impl Imp for MasterStack {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        if cx.children.is_empty() {
            return Vec::new();
        }
        let ratio = clamp(self.opts.num("ratio"), MIN_RATIO, MAX_RATIO);
        let master_count = self.opts.num("master_count") as usize;
        let (area, gap) = geometry(self.opts.maybe("gap"), cx);
        let sideways = self.sideways();
        let (span, across) = if sideways {
            (area.height, area.width)
        } else {
            (area.width, area.height)
        };
        self.last = Some((cx.children.clone(), master_count, span - gap));
        let orientation = self.orientation();
        let frames = master_stack_frames(span, across, cx.children.len(), master_count, ratio, gap);
        cx.children
            .iter()
            .zip(frames)
            .map(|(client, [u, v, p, q])| {
                let (rx, ry, rw, rh) = match orientation {
                    0 => (u, v, p, q),
                    1 => (area.width - u - p, v, p, q),
                    2 => (v, u, q, p),
                    _ => (v, area.height - u - p, q, p),
                };
                tile(*client, area.x + rx, area.y + ry, rw, rh)
            })
            .collect()
    }

    /// The divider between the master and the stack, for any tile of the last compute: dragging the pointer along the master's axis
    /// moves it by as far, whichever side the master is on. With no master or no stack there is none.
    fn find_split(&mut self, client: NodeId, _at: Pt) -> Option<SplitSpec> {
        let (nodes, master, span) = self.last.as_ref()?;
        if !nodes.contains(&client) || *span <= 0.0 || *master == 0 || *master >= nodes.len() {
            return None;
        }
        // The master grows as the pointer moves away from the edge it sits on.
        let direction = if matches!(self.orientation(), 0 | 2) { 1 } else { -1 };
        let divider = DividerSpec {
            x_axis: !self.sideways(),
            span: *span,
            direction,
            min: MIN_RATIO,
            max: MAX_RATIO,
            ratio: clamp(self.opts.num("ratio"), MIN_RATIO, MAX_RATIO),
        };
        self.token += 1;
        self.drag = Some((self.token, self.opts.num("ratio")));
        Some(SplitSpec {
            token: self.token,
            divider,
            cross: None,
        })
    }

    fn set_split(&mut self, token: u64, ratio: f64, _cross: Option<f64>) {
        if self.drag.is_some_and(|(held, _)| held == token) {
            self.opts.put("ratio", Val::Num(ratio));
        }
    }

    fn reset_split(&mut self, token: u64) {
        if let Some((held, original)) = self.drag
            && held == token
        {
            self.opts.put("ratio", Val::Num(original));
        }
    }
}

/// Every window the same size, in rows and columns.
pub(super) struct Grid {
    opts: OptionSet,
}

impl Grid {
    pub(super) fn new(opts: OptionSet) -> Grid {
        Grid { opts }
    }

    /// `(columns, rows)` for `n` windows.
    fn shape(&self, n: usize) -> (usize, usize) {
        let fixed = self.opts.num("columns") as usize;
        let by_column = self.opts.member("fill") == 1;
        if fixed > 0 {
            return (fixed, n.div_ceil(fixed));
        }
        let side = (n as f64).sqrt().ceil() as usize;
        if by_column {
            (n.div_ceil(side), side)
        } else {
            (side, n.div_ceil(side))
        }
    }
}

impl Imp for Grid {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let n = cx.children.len();
        if n == 0 {
            return Vec::new();
        }
        let (columns, rows) = self.shape(n);
        let (area, gap) = geometry(self.opts.maybe("gap"), cx);
        let by_column = self.opts.member("fill") == 1;
        let cell_width = ((area.width - (columns - 1) as f64 * gap) / columns as f64).max(0.0);
        let cell_height = ((area.height - (rows - 1) as f64 * gap) / rows as f64).max(0.0);
        cx.children
            .iter()
            .enumerate()
            .map(|(i, client)| {
                let (column, row) = if by_column {
                    (i / rows, i % rows)
                } else {
                    (i % columns, i / columns)
                };
                tile(
                    *client,
                    area.x + column as f64 * (cell_width + gap),
                    area.y + row as f64 * (cell_height + gap),
                    cell_width,
                    cell_height,
                )
            })
            .collect()
    }
}

/// One window fills the area and the rest are hidden: the one that has the keyboard, else the first.
pub(super) struct Monocle {
    opts: OptionSet,
}

impl Monocle {
    pub(super) fn new(opts: OptionSet) -> Monocle {
        Monocle { opts }
    }
}

impl Imp for Monocle {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let Some(first) = cx.children.first() else {
            return Vec::new();
        };
        let (area, _) = geometry(None, cx);
        let shown = cx
            .focused
            .filter(|focused| cx.children.contains(focused))
            .unwrap_or(*first);
        cx.children
            .iter()
            .map(|client| {
                let placement = tile(*client, area.x, area.y, area.width, area.height);
                if *client == shown {
                    placement
                } else {
                    Placement {
                        visible: false,
                        ..placement
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LayoutContext;
    use gnoblin_core::Rect;
    use gnoblin_std_abi::layouts::{LayoutKindDesc, layout_kind};
    use gnoblin_std_abi::{STD_BUILTIN_GRID, STD_BUILTIN_MASTER_STACK, STD_BUILTIN_MONOCLE};

    fn kind(id: u32) -> &'static LayoutKindDesc {
        layout_kind(id).expect("the kind is declared")
    }

    fn cx(children: &[u64], focused: Option<u64>) -> LayoutContext {
        LayoutContext {
            workspace: 1,
            children: children.to_vec(),
            area: Rect::new(0.0, 0.0, 1000.0, 800.0),
            gap_inner: 0.0,
            gap_outer: 0.0,
            focused,
            pointer: None,
        }
    }

    fn rects(placements: &[Placement]) -> Vec<(u64, f64, f64, f64, f64)> {
        placements
            .iter()
            .map(|p| {
                let b = p.bounds.expect("a rectangle");
                (p.node, b.x, b.y, b.width, b.height)
            })
            .collect()
    }

    #[test]
    fn a_master_takes_half_and_the_stack_shares_the_rest() {
        let mut layout = MasterStack::new(OptionSet::defaults(kind(STD_BUILTIN_MASTER_STACK)));
        let out = layout.compute(&cx(&[1, 2, 3], None));
        assert_eq!(
            rects(&out),
            vec![
                (1, 0.0, 0.0, 500.0, 800.0),
                (2, 500.0, 0.0, 500.0, 400.0),
                (3, 500.0, 400.0, 500.0, 400.0)
            ]
        );
    }

    #[test]
    fn the_divider_moves_with_the_pointer_and_a_cancel_puts_it_back() {
        let mut layout = MasterStack::new(OptionSet::defaults(kind(STD_BUILTIN_MASTER_STACK)));
        layout.compute(&cx(&[1, 2], None));
        let split = layout.find_split(1, Pt { x: 0.0, y: 0.0 }).expect("a divider");
        assert!(split.divider.x_axis);
        layout.set_split(split.token, 0.7, None);
        assert_eq!(rects(&layout.compute(&cx(&[1, 2], None)))[0].3, 700.0);
        layout.reset_split(split.token);
        assert_eq!(rects(&layout.compute(&cx(&[1, 2], None)))[0].3, 500.0);
    }

    #[test]
    fn a_grid_of_five_is_three_columns_by_two_rows() {
        let mut layout = Grid::new(OptionSet::defaults(kind(STD_BUILTIN_GRID)));
        assert_eq!(layout.shape(5), (3, 2));
        assert_eq!(layout.compute(&cx(&[1, 2, 3, 4, 5], None)).len(), 5);
    }

    #[test]
    fn a_monocle_shows_the_focused_window_only() {
        let mut layout = Monocle::new(OptionSet::defaults(kind(STD_BUILTIN_MONOCLE)));
        let out = layout.compute(&cx(&[1, 2, 3], Some(2)));
        assert_eq!(
            out.iter().map(|p| p.visible).collect::<Vec<_>>(),
            vec![false, true, false]
        );
    }
}
