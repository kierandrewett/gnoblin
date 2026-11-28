//! Fitting a window to the work area it is in: after a workspace or output change. A maximised window fills the area and a floating
//! one shrinks to fit and moves inside it, its frame (the content with the room the client's frame takes round it) staying inside; a
//! snapped window tiles to its zone again; a fullscreen window covers its output. This is `host/fitting.ts`.

use std::cell::RefCell;
use std::collections::HashMap;

use gnoblin_core::host::Rect;

use crate::client_state::fit_content;
use crate::desks::output_rect;
use crate::floating::{Inset, shrink};
use crate::nodes;
use crate::state::{NodeId, State};

thread_local! {
    /// The room each window's frame took when it was last fitted, by window, kept across generations. A generation keeps its own
    /// record of a window, so without this a reload's first pass would take the content's place for the frame's: the new frame
    /// would move the content by all its room instead of by what it gained or lost, and a reload that drops the frames would leave
    /// the old frame's offset. The table is the shell thread's, as every generation of the shell is.
    static LAST_FITTED: RefCell<HashMap<NodeId, Inset>> = RefCell::new(HashMap::new());
}

/// The room `client`'s frame took when an earlier generation last fitted it, if one did.
pub(crate) fn carried_fit(client: NodeId) -> Option<Inset> {
    LAST_FITTED.with(|fitted| fitted.borrow().get(&client).copied())
}

/// `client` is gone: no later generation fits it.
pub(crate) fn forget_fit(client: NodeId) {
    LAST_FITTED.with(|fitted| fitted.borrow_mut().remove(&client));
}

/// A rectangle that moved to another output: shrunk to the area when larger, then moved inside it.
pub(crate) fn fit_into(rect: &Rect, area: &Rect) -> Rect {
    let width = rect.width.min(area.width);
    let height = rect.height.min(area.height);
    Rect::new(
        rect.x.max(area.x).min(area.x + area.width - width),
        rect.y.max(area.y).min(area.y + area.height - height),
        width,
        height,
    )
}

impl State {
    /// The output `client` is on: the one its workspace is shown on (for a span, the one that holds the middle of the client), or
    /// `None` while no output shows its workspace.
    fn output_of_client(&self, client: NodeId) -> Option<NodeId> {
        let desk = self
            .rec
            .get(&client)
            .and_then(|rec| rec.workspace)
            .and_then(|workspace| self.desk_showing(workspace))
            .and_then(|id| self.desk(id))?;
        let own = desk.output?;
        if desk.parked {
            return None;
        }
        let Some(spans) = &desk.spans else {
            return Some(own);
        };
        let rect = nodes::rect_of(&self.g, client);
        let (x, y) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        Some(
            spans
                .iter()
                .copied()
                .find(|output| {
                    output_rect(&self.g, *output)
                        .is_some_and(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
                })
                .unwrap_or(own),
        )
    }

    /// The output rectangle `client` is on, or the primary output's when its workspace is on none.
    pub(crate) fn output_rect_of(&self, client: NodeId) -> Option<Rect> {
        match self.output_of_client(client) {
            Some(output) => output_rect(&self.g, output),
            None => self.primary_output().and_then(|output| output_rect(&self.g, output)),
        }
    }

    /// The work area of the output `client` is on, or the primary output's when its workspace is on none.
    pub(crate) fn work_area_of_client(&self, client: NodeId) -> Option<Rect> {
        match self.output_of_client(client) {
            Some(output) => self.area_of(output),
            None => self.screen_area(),
        }
    }

    /// A frame came, went or changed (a reload toggled decorations, or the shell gave a client's frame another size): the content
    /// does not move by itself, so the rule is that the FRAME keeps its place: the content moves in by what the top and left gained
    /// and out by what they lost, and the result is clamped so the frame stays inside `area`. A maximised window fills the area
    /// again, a snapped one tiles to its zone again, and the rectangles a window returns to move the same way, so they do not bring
    /// the old offset back. A fullscreen window is left alone and fitted when it leaves. Returns whether the frame changed since
    /// the client was last fitted, so a layout places its windows again. This is `followFrame` of `host/fitting.ts`.
    pub(crate) fn follow_frame(&mut self, client: NodeId, area: &Rect) -> bool {
        let g = self.g.clone();
        if nodes::flag(&g, client, "fullscreen") {
            return false;
        }
        let inset = self.inset_of(client);
        let Some(rec) = self.rec.get_mut(&client) else {
            return false;
        };
        let previous = rec.fitted.replace(inset);
        LAST_FITTED.with(|fitted| fitted.borrow_mut().insert(client, inset));
        let Some(was) = previous else {
            return false;
        };
        if was == inset {
            return false;
        }
        let (dx, dy) = (inset.left - was.left, inset.top - was.top);
        let moved = |rect: &Rect| {
            fit_content(
                &Rect::new(rect.x + dx, rect.y + dy, rect.width, rect.height),
                &inset,
                area,
            )
        };
        let mut floats = false;
        if let Some(rec) = self.rec.get_mut(&client) {
            if let Some(snap) = rec.snap.as_mut() {
                snap.floating = moved(&snap.floating);
            }
            if let Some(floating) = rec.float_rect {
                rec.float_rect = Some(moved(&floating));
            }
            floats = rec.floating;
        }
        let zone = self.rec.get(&client).and_then(|rec| rec.snap).map(|snap| snap.zone);
        if nodes::flag(&g, client, "maximised") {
            nodes::write_rect(&g, client, &shrink(area, &inset));
        } else if let Some(zone) = zone {
            self.snap_client(client, zone, area, true);
        } else if !nodes::flag(&g, client, "tiled") || floats {
            let own = nodes::rect_of(&g, client);
            nodes::write_rect(&g, client, &moved(&own));
        }
        true
    }

    /// Fits a window to the work area it is now in: a maximised window fills it, a snapped one tiles to its zone again, a floating one
    /// shrinks to fit and moves inside it (its frame stays inside), and the rectangle it floats at follows. A fullscreen window
    /// covers its output.
    pub(crate) fn refit_client(&mut self, client: NodeId, area: &Rect) {
        let g = self.g.clone();
        if nodes::flag(&g, client, "fullscreen") {
            if let Some(output) = self.output_rect_of(client) {
                nodes::write_rect(&g, client, &output);
            }
            return;
        }
        let inset = self.inset_of(client);
        let zone = self.rec.get(&client).and_then(|rec| rec.snap).map(|snap| snap.zone);
        if nodes::flag(&g, client, "maximised") {
            nodes::write_rect(&g, client, &shrink(area, &inset));
        } else if let Some(zone) = zone {
            self.snap_client(client, zone, area, true);
        } else {
            let own = nodes::rect_of(&g, client);
            nodes::write_rect(&g, client, &fit_content(&own, &inset, area));
        }
        if let Some(rec) = self.rec.get_mut(&client)
            && let Some(floating) = rec.float_rect
        {
            rec.float_rect = Some(fit_content(&floating, &inset, area));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rectangle_larger_than_the_area_shrinks_to_it() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let fitted = fit_into(&Rect::new(100.0, 100.0, 2000.0, 900.0), &area);
        assert_eq!(
            (fitted.x, fitted.y, fitted.width, fitted.height),
            (0.0, 0.0, 1000.0, 800.0)
        );
    }

    #[test]
    fn a_rectangle_past_the_edge_moves_inside() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let fitted = fit_into(&Rect::new(900.0, -50.0, 300.0, 200.0), &area);
        assert_eq!((fitted.x, fitted.y), (700.0, 0.0));
    }

    #[test]
    fn a_rectangle_inside_the_area_stays() {
        let area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let own = Rect::new(10.0, 20.0, 300.0, 200.0);
        assert_eq!(fit_into(&own, &area), own);
    }
}
