//! [`StripLayout`]: one column per window in a horizontal strip, with the focus kept in view.
//!
//! A port of the `StripLayout` of `data/js/gnoblin-std/layouts/strip.ts`, which is the reference. Columns sit edge to edge with no gap
//! or margin and the layout does not tile its windows. The view follows the focus unless [`scroll_to`](StripLayout::scroll_to) pinned
//! it; a pin ends when the focused window changes.
//!
//! The layout keeps its own column order, because a new window opens right after the focused column rather than at the end, and so it
//! hands the keyboard to a neighbour itself when a window closes. One layout serves one workspace. Like the dwindle it is a handle:
//! clones share the strip, so a shell keeps one for [`focus_left`](StripLayout::focus_left) and its kin.

use std::cell::RefCell;
use std::rc::Rc;

use gnoblin_core::Rect;

use crate::client::Client;
use crate::error::{Error, ErrorKind, Result};
use crate::layout::{Layout, LayoutContext, Placement};
use crate::workspace::Workspace;

const MIN_COLUMN: f64 = 0.1;
const MAX_COLUMN: f64 = 1.0;

struct StripCore {
    /// A column's width as a fraction of the strip's, held to 0.1 to 1.
    column: f64,
    /// The view offset in pixels `scroll_to` pinned, or `None` while the view follows the focused column.
    pin: Option<f64>,
    order: Vec<u64>,
    view: f64,
    /// The focus the previous compute saw.
    focus: Option<u64>,
    revision: u64,
    /// The workspace the layout computed for last: the one a change of the strip asks to compute again.
    workspace: Option<Workspace>,
}

/// A column's width as a fraction of the strip's width, held to 0.1 to 1. NaN is an error.
fn check_column(value: f64) -> Result<f64> {
    if value.is_nan() {
        return Err(Error::new(ErrorKind::Type, "column must be a number, got NaN"));
    }
    Ok(value.clamp(MIN_COLUMN, MAX_COLUMN))
}

impl StripCore {
    /// The x offset and width of each column, in strip order: `(id, x, width)`.
    fn compute(&mut self, children: &[u64], focused: Option<u64>, area: Rect) -> Vec<(u64, Rect)> {
        let column_width = self.column.clamp(MIN_COLUMN, MAX_COLUMN) * area.width;
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

/// A strip of columns. See the [module](self) documentation.
#[derive(Clone)]
pub struct StripLayout {
    core: Rc<RefCell<StripCore>>,
}

impl Default for StripLayout {
    fn default() -> StripLayout {
        StripLayout::new()
    }
}

impl std::fmt::Debug for StripLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let core = self.core.borrow();
        f.debug_struct("StripLayout")
            .field("column", &core.column)
            .field("pin", &core.pin)
            .finish_non_exhaustive()
    }
}

impl StripLayout {
    /// A strip whose columns are half the strip's width.
    pub fn new() -> StripLayout {
        StripLayout::with_column(0.5).unwrap_or_else(|_| unreachable!("0.5 is a number"))
    }

    /// A strip whose columns are `column` of the strip's width, held to 0.1 to 1.
    ///
    /// # Errors
    ///
    /// `column` is NaN.
    pub fn with_column(column: f64) -> Result<StripLayout> {
        let column = check_column(column)?;
        Ok(StripLayout {
            core: Rc::new(RefCell::new(StripCore {
                column,
                pin: None,
                order: Vec::new(),
                view: 0.0,
                focus: None,
                revision: 0,
                workspace: None,
            })),
        })
    }

    /// A column's width as a fraction of the strip's, 0.1 to 1.
    pub fn column(&self) -> f64 {
        self.core.borrow().column
    }

    /// Sets a column's width as a fraction of the strip's, held to 0.1 to 1. The workspace computes again.
    ///
    /// # Errors
    ///
    /// `column` is NaN.
    pub fn set_column(&self, column: f64) -> Result<()> {
        let column = check_column(column)?;
        let workspace = {
            let mut core = self.core.borrow_mut();
            core.column = column;
            core.revision += 1;
            core.workspace.clone()
        };
        workspace.map_or(Ok(()), |workspace| workspace.invalidate())
    }

    /// The view offset in pixels that [`scroll_to`](StripLayout::scroll_to) pinned, or `None` while the view follows the focus.
    pub fn pin(&self) -> Option<f64> {
        self.core.borrow().pin
    }

    /// Scrolls the view to `offset` pixels and keeps it there until the focused window changes.
    ///
    /// # Errors
    ///
    /// `offset` is not a finite number.
    pub fn scroll_to(&self, offset: f64) -> Result<()> {
        if !offset.is_finite() {
            return Err(Error::new(
                ErrorKind::Type,
                format!("scrollTo needs a finite number, got {offset}"),
            ));
        }
        let workspace = {
            let mut core = self.core.borrow_mut();
            core.pin = Some(offset);
            core.revision += 1;
            core.workspace.clone()
        };
        workspace.map_or(Ok(()), |workspace| workspace.invalidate())
    }

    /// Moves the keyboard one column left; at the first column nothing happens.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn focus_left(&self, workspace: &Workspace) -> Result<()> {
        self.step(workspace, -1)
    }

    /// Moves the keyboard one column right; at the last column nothing happens.
    ///
    /// # Errors
    ///
    /// The workspace is gone, or the generation retired.
    pub fn focus_right(&self, workspace: &Workspace) -> Result<()> {
        self.step(workspace, 1)
    }

    fn step(&self, workspace: &Workspace, delta: isize) -> Result<()> {
        let clients = workspace.clients()?.to_vec()?;
        let held: Vec<u64> = clients.iter().map(|client| client.window().id().0).collect();
        let focused = clients
            .iter()
            .find(|client| client.window().is_focused().unwrap_or(false))
            .map(|client| client.window().id().0);
        let next = self.core.borrow().neighbour(&held, focused, delta);
        if let Some(client) = next.and_then(|id| clients.iter().find(|client| client.window().id().0 == id)) {
            client.window().focus()?;
        }
        Ok(())
    }
}

impl Layout for StripLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let ids: Vec<u64> = cx.children.iter().map(|client| client.window().id().0).collect();
        let focused = cx.focused.as_ref().map(|client| client.window().id().0);
        let mut core = self.core.borrow_mut();
        core.workspace = Some(cx.workspace.clone());
        let columns = core.compute(&ids, focused, cx.area);
        columns
            .into_iter()
            .filter_map(|(id, rect)| {
                let client = cx.children.iter().find(|client| client.window().id().0 == id)?;
                Some(Placement::new(client).bounds(rect))
            })
            .collect()
    }

    /// Focus moves to a neighbour of a focused column that went, so the strip keeps a focus.
    fn client_removed(&mut self, client: &Client, cx: &LayoutContext) {
        let core = self.core.borrow();
        let id = client.window().id().0;
        let Some(at) = core.order.iter().position(|one| *one == id) else {
            return;
        };
        if core.focus != Some(id) {
            return;
        }
        let live = |one: &u64| cx.children.iter().any(|child| child.window().id().0 == *one);
        let after = core.order.get(at + 1).filter(|one| live(one));
        let before = at
            .checked_sub(1)
            .and_then(|before| core.order.get(before))
            .filter(|one| live(one));
        if let Some(next) = after.or(before)
            && let Some(child) = cx.children.iter().find(|child| child.window().id().0 == *next)
        {
            let _ = child.window().focus();
        }
    }

    fn hands_focus_on_removal(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.core.borrow().revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(column: f64) -> StripCore {
        StripCore {
            column,
            pin: None,
            order: Vec::new(),
            view: 0.0,
            focus: None,
            revision: 0,
            workspace: None,
        }
    }

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 1000.0, 800.0)
    }

    #[test]
    fn columns_sit_edge_to_edge_and_a_new_one_opens_after_the_focused_column() {
        let mut strip = core(0.5);
        let first = strip.compute(&[1, 2, 3], Some(1), area());
        assert_eq!(first.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 2, 3]);
        // The focus stays on 1; window 4 arrives and opens right after it.
        let ids: Vec<u64> = strip
            .compute(&[1, 2, 3, 4], Some(1), area())
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids, vec![1, 4, 2, 3]);
    }

    #[test]
    fn the_view_centres_the_focused_column_once_it_is_out_of_sight() {
        let mut strip = core(0.5);
        strip.compute(&[1, 2, 3, 4], Some(1), area());
        let columns = strip.compute(&[1, 2, 3, 4], Some(4), area());
        // Four 500 wide columns in a 1000 wide view: the last is centred (1750 - 500) and the view is held to 2000 - 1000.
        let x: Vec<f64> = columns.iter().map(|(_, rect)| rect.x).collect();
        assert_eq!(x, vec![-1000.0, -500.0, 0.0, 500.0]);
    }

    #[test]
    fn a_pin_holds_the_view_until_the_focus_changes() {
        let mut strip = core(0.5);
        strip.compute(&[1, 2, 3, 4], Some(1), area());
        strip.pin = Some(300.0);
        let pinned = strip.compute(&[1, 2, 3, 4], Some(1), area());
        assert_eq!(pinned[0].1.x, -300.0);
        let moved = strip.compute(&[1, 2, 3, 4], Some(2), area());
        assert!(strip.pin.is_none());
        assert_eq!(moved[1].1.x, 500.0 - strip.view);
    }
}
