//! The strip: one column per window in a horizontal strip, with the focus kept in view.
//!
//! A port of the `StripLayout` of `data/js/gnoblin-std/layouts/strip.ts`, which is the reference. Columns sit edge to edge with no gap
//! or margin and the layout does not tile its windows. The view follows the focus unless `scroll_to` pinned it; a pin ends when the
//! focused window changes.
//!
//! The layout keeps its own column order, because a new window opens right after the focused column rather than at the end, and so it
//! hands the keyboard to a neighbour itself when a window closes. One layout serves one workspace.

use gnoblin_core::Rect;

use super::{ActionEnv, ActionResult, Imp, OptionSet, Val, plain};
use crate::error::{Fail, Res};
use crate::layout::{LayoutContext, Placement};
use crate::state::NodeId;

const MIN_COLUMN: f64 = 0.1;
const MAX_COLUMN: f64 = 1.0;

struct StripCore {
    /// The view offset in pixels `scroll_to` pinned, or `None` while the view follows the focused column.
    pin: Option<f64>,
    order: Vec<u64>,
    view: f64,
    /// The focus the previous compute saw.
    focus: Option<u64>,
}

impl StripCore {
    /// The x offset and width of each column, in strip order: `(id, rect)`.
    fn compute(&mut self, column: f64, children: &[u64], focused: Option<u64>, area: Rect) -> Vec<(u64, Rect)> {
        let column_width = column.clamp(MIN_COLUMN, MAX_COLUMN) * area.width;
        self.order.retain(|id| children.contains(id));
        // A window opens after the column that had focus when it arrived, which the compositor has since moved to it.
        let mut at = self
            .focus
            .and_then(|focus| self.order.iter().position(|id| *id == focus));
        for id in children {
            if self.order.contains(id) {
                continue;
            }
            if let Some(index) = at {
                at = Some(index + 1);
                self.order.insert(index + 1, *id);
            } else {
                self.order.push(*id);
            }
        }
        if focused != self.focus {
            self.focus = focused.filter(|id| children.contains(id));
            self.pin = None;
        }
        if let Some(pin) = self.pin {
            self.view = pin;
        } else if let Some(index) = focused.and_then(|id| self.order.iter().position(|one| *one == id)) {
            let left = index as f64 * column_width;
            if left < self.view || left + column_width > self.view + area.width {
                self.view = left + column_width / 2.0 - area.width / 2.0; // centre the focused column
            }
        }
        self.view = self
            .view
            .max(0.0)
            .min((self.order.len() as f64 * column_width - area.width).max(0.0));
        self.order
            .iter()
            .enumerate()
            .map(|(i, id)| {
                (
                    *id,
                    Rect::new(
                        area.x + i as f64 * column_width - self.view,
                        area.y,
                        column_width,
                        area.height,
                    ),
                )
            })
            .collect()
    }

    /// The column to the `delta` side of the focused one among `held`, if there is one.
    fn neighbour(&self, held: &[u64], focused: Option<u64>, delta: isize) -> Option<u64> {
        let columns: Vec<u64> = self.order.iter().copied().filter(|id| held.contains(id)).collect();
        let at = columns.iter().position(|id| Some(*id) == focused)?;
        columns.get(at.checked_add_signed(delta)?).copied()
    }
}

/// The built-in strip layout.
pub(super) struct ColumnStrip {
    opts: OptionSet,
    core: StripCore,
}

impl ColumnStrip {
    pub(super) fn new(opts: OptionSet) -> ColumnStrip {
        ColumnStrip {
            opts,
            core: StripCore {
                pin: None,
                order: Vec::new(),
                view: 0.0,
                focus: None,
            },
        }
    }
}

impl Imp for ColumnStrip {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let column = self.opts.num("column");
        self.core
            .compute(column, &cx.children, cx.focused, cx.area)
            .into_iter()
            .map(|(id, rect)| Placement {
                bounds: Some(rect),
                ..plain(id)
            })
            .collect()
    }

    /// Focus moves to a neighbour of a focused column that went, so the strip keeps a focus.
    fn client_removed(&mut self, cx: &LayoutContext, client: NodeId) -> Option<NodeId> {
        let core = &self.core;
        let at = core.order.iter().position(|one| *one == client)?;
        if core.focus != Some(client) {
            return None;
        }
        let live = |one: &&u64| cx.children.contains(*one);
        let after = core.order.get(at + 1).filter(live);
        let before = at.checked_sub(1).and_then(|before| core.order.get(before)).filter(live);
        after.or(before).copied()
    }

    fn hands_focus(&self) -> bool {
        true
    }

    fn act(&mut self, action: &str, args: &[Val], env: &ActionEnv) -> Res<ActionResult> {
        match action {
            "scroll_to" => {
                self.core.pin = match args.first() {
                    Some(Val::Num(offset)) => Some(*offset),
                    // No offset lets the view follow the focus again.
                    Some(Val::None) | None => None,
                    Some(_) => return Err(Fail::type_error("strip.scroll_to takes an offset in pixels or none")),
                };
                Ok(ActionResult {
                    value: Val::None,
                    changed: true,
                    focus: None,
                })
            }
            "focus_left" | "focus_right" => {
                let delta = if action == "focus_left" { -1 } else { 1 };
                let focus = self.core.neighbour(&env.clients, env.focused, delta);
                Ok(ActionResult {
                    value: Val::None,
                    changed: false,
                    focus,
                })
            }
            "pin" => Ok(ActionResult::answer(self.core.pin.map_or(Val::None, Val::Num))),
            other => Err(Fail::no_such(format!("the strip layout has no action `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> StripCore {
        StripCore {
            pin: None,
            order: Vec::new(),
            view: 0.0,
            focus: None,
        }
    }

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 800.0)
    }

    #[test]
    fn columns_sit_edge_to_edge_and_a_new_one_opens_after_the_focused_column() {
        let mut strip = core();
        let first = strip.compute(0.5, &[1, 2, 3], Some(1), area());
        assert_eq!(first.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2, 3]);
        // The focus stays on 1; window 4 arrives and opens right after it.
        let ids: Vec<u64> = strip
            .compute(0.5, &[1, 2, 3, 4], Some(1), area())
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids, vec![1, 4, 2, 3]);
    }

    #[test]
    fn the_view_centres_the_focused_column_once_it_is_out_of_sight() {
        let mut strip = core();
        strip.compute(0.5, &[1, 2, 3, 4], Some(1), area());
        let columns = strip.compute(0.5, &[1, 2, 3, 4], Some(4), area());
        // Four 500 wide columns in a 1000 wide view: the last is centred (1750 - 500) and the view is held to 2000 - 1000.
        let x: Vec<f64> = columns.iter().map(|(_, rect)| rect.x).collect();
        assert_eq!(x, vec![-1000.0, -500.0, 0.0, 500.0]);
    }

    #[test]
    fn a_pin_holds_the_view_until_the_focus_changes() {
        let mut strip = core();
        strip.compute(0.5, &[1, 2, 3, 4], Some(1), area());
        strip.pin = Some(300.0);
        let pinned = strip.compute(0.5, &[1, 2, 3, 4], Some(1), area());
        assert_eq!(pinned[0].1.x, -300.0);
        let moved = strip.compute(0.5, &[1, 2, 3, 4], Some(2), area());
        assert!(strip.pin.is_none());
        assert_eq!(moved[1].1.x, 500.0 - strip.view);
    }
}
