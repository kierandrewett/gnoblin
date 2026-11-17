//! Fitting a window to the work area it is in: after a workspace or output change. A maximised window fills the area and a floating
//! one shrinks to fit and moves inside it; a fullscreen window covers its output. This is `host/fitting.ts` less the snapped-window
//! and frame-inset cases, which belong to the window-state and decoration libraries that come with the later steps.

use gnoblin_core::Rect;

use crate::desks::output_rect;
use crate::nodes;
use crate::state::{NodeId, State};

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
    /// The output rectangle `client` is on, or the primary output's when its workspace is on none.
    pub(crate) fn output_rect_of(&self, client: NodeId) -> Option<Rect> {
        let desk = self
            .rec
            .get(&client)
            .and_then(|rec| rec.workspace)
            .and_then(|workspace| self.desk_showing(workspace))
            .and_then(|id| self.desk(id));
        let output = desk.and_then(|desk| {
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
        });
        match output {
            Some(output) => output_rect(&self.g, output),
            None => self.primary_output().and_then(|output| output_rect(&self.g, output)),
        }
    }

    /// Fits a window to the work area it is now in: a maximised window fills it, a floating one shrinks to fit and moves inside it.
    /// A fullscreen window covers its output.
    pub(crate) fn refit_client(&mut self, client: NodeId, area: &Rect) {
        let g = self.g.clone();
        if nodes::flag(&g, client, "fullscreen") {
            if let Some(output) = self.output_rect_of(client) {
                nodes::write_rect(&g, client, &output);
            }
            return;
        }
        if nodes::flag(&g, client, "maximised") {
            nodes::write_rect(&g, client, area);
        } else {
            let own = nodes::rect_of(&g, client);
            nodes::write_rect(&g, client, &fit_into(&own, area));
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
