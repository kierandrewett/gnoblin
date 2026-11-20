//! Where a box puts its children: a row, a column, or a grid of equal cells.

use super::widget::{Cid, Ui, WidgetKind, en};

impl Ui {
    /// Places the children of `cid` as its class does by itself: a box in a `row` or a `column` (a small flexbox: the main axis in
    /// order with `gap` between and `padding` around, a child with `grow` taking its share of what is left, every child stretched
    /// across the other axis), a grid in near-square cells. Children that are not boxes stay where they are, and a box whose direction
    /// is `none` leaves its children alone. Other canvases place nothing.
    pub(crate) fn arrange_default(&self, cid: Cid) {
        let Some((kind, direction)) = self.read(cid, |widget| (widget.kind, widget.b.direction)) else {
            return;
        };
        match kind {
            WidgetKind::Canvas => {}
            WidgetKind::Grid => self.arrange_grid(cid),
            _ if direction == en::DIRECTION_NONE => {}
            _ => self.arrange_flex(cid, direction == en::DIRECTION_COLUMN),
        }
    }

    fn arrange_flex(&self, cid: Cid, column: bool) {
        let Some((children, pad, gap)) =
            self.read(cid, |widget| (widget.children.clone(), widget.b.padding, widget.b.gap))
        else {
            return;
        };
        let kids: Vec<Cid> = children
            .into_iter()
            .filter(|kid| self.read(*kid, |widget| widget.kind.is_box()).unwrap_or(false))
            .collect();
        let size = |kid: Cid| {
            if column {
                self.height_of(kid)
            } else {
                self.width_of(kid)
            }
        };
        let mut growing = 0.0;
        let mut used = 0.0;
        for kid in &kids {
            let grow = self.read(*kid, |widget| widget.b.grow).unwrap_or(0.0);
            if grow > 0.0 {
                growing += grow;
            } else {
                used += size(*kid);
            }
        }
        let (room, across) = if column {
            (self.height_of(cid), self.width_of(cid) - 2.0 * pad)
        } else {
            (self.width_of(cid), self.height_of(cid) - 2.0 * pad)
        };
        let free = room - 2.0 * pad - gap * (kids.len().saturating_sub(1) as f64) - used;
        let mut at = pad;
        for kid in kids {
            let grow = self.read(kid, |widget| widget.b.grow).unwrap_or(0.0);
            if grow > 0.0 {
                // A growing child takes its share of the free room. A negative share is clamped to zero.
                let share = ((free * grow) / growing).floor().max(0.0);
                self.tweak(kid, |widget| {
                    if column {
                        widget.height = Some(share);
                    } else {
                        widget.width = Some(share);
                    }
                });
            }
            self.tweak(kid, |widget| {
                if column {
                    widget.y = at;
                    widget.x = pad;
                    widget.width = Some(across);
                } else {
                    widget.x = at;
                    widget.y = pad;
                    widget.height = Some(across);
                }
            });
            at += size(kid) + gap;
        }
    }

    fn arrange_grid(&self, cid: Cid) {
        let Some((kids, pad, gap)) = self.read(cid, |widget| (widget.children.clone(), widget.b.padding, widget.b.gap))
        else {
            return;
        };
        let count = kids.len() as f64;
        let cols = count.sqrt().ceil().max(1.0);
        let rows = (count / cols).ceil().max(1.0);
        let w = ((self.width_of(cid) - 2.0 * pad - (cols - 1.0) * gap) / cols).floor();
        let h = ((self.height_of(cid) - 2.0 * pad - (rows - 1.0) * gap) / rows).floor();
        for (i, kid) in kids.into_iter().enumerate() {
            let i = i as f64;
            self.tweak(kid, |widget| {
                widget.x = pad + (i % cols) * (w + gap);
                widget.y = pad + (i / cols).floor() * (h + gap);
                widget.width = Some(w);
                widget.height = Some(h);
            });
        }
    }
}
