//! The arc: windows around the viewer on an arc, the focused window at the front.
//!
//! A port of `data/js/gnoblin-std/layouts/arc.ts`, which is the reference. It answers with transform placements: positions on a circle
//! in the x-z plane and a turn about the vertical axis that faces each window towards the centre. On a flat workspace the placement is
//! the window's offset from the centre of the work area (`radius * sin(angle)` across), its depth `z`, its turn about Y and its scale.
//!
//! Window `i` is at the angle `(i - selected) * spacing`, where `selected` is the focused window's index (0 when none is focused). Its
//! position is `(sin(angle) * radius, 0, -cos(angle) * radius)` and it is turned `-angle` about Y, so the arc is concave towards a
//! viewer at its centre.

use super::{Imp, OptionSet, plain};
use crate::layout::{LayoutContext, Placement};

/// The built-in arc layout.
pub(super) struct Arc {
    opts: OptionSet,
}

impl Arc {
    pub(super) fn new(opts: OptionSet) -> Arc {
        Arc { opts }
    }
}

impl Imp for Arc {
    fn options(&self) -> &OptionSet {
        &self.opts
    }

    fn options_mut(&mut self) -> &mut OptionSet {
        &mut self.opts
    }

    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let radius = self.opts.num("radius");
        let spacing = self.opts.num("spacing");
        let selected = cx
            .focused
            .and_then(|focused| cx.children.iter().position(|child| *child == focused))
            .unwrap_or(0);
        cx.children
            .iter()
            .enumerate()
            .map(|(i, client)| {
                let angle = (i as f64 - selected as f64) * spacing;
                Placement {
                    offset: Some((angle.sin() * radius, 0.0)),
                    z: -angle.cos() * radius,
                    rotation: [0.0, -angle, 0.0],
                    ..plain(*client)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_core::Rect;

    #[test]
    fn the_focused_window_is_at_the_front_and_the_others_turn_towards_the_centre() {
        let kind = gnoblin_std_abi::layouts::layout_kind(gnoblin_std_abi::STD_BUILTIN_ARC).expect("arc is declared");
        let mut arc = Arc::new(OptionSet::defaults(kind));
        let cx = LayoutContext {
            workspace: 1,
            children: vec![1, 2, 3],
            area: Rect::new(0.0, 0.0, 1000.0, 800.0),
            gap_inner: 0.0,
            gap_outer: 0.0,
            focused: Some(2),
            pointer: None,
        };
        let out = arc.compute(&cx);
        assert_eq!(out[1].offset, Some((0.0, 0.0)));
        assert_eq!(out[1].z, -1200.0);
        assert!(out[0].offset.is_some_and(|(x, _)| x < 0.0));
        assert!(out[2].offset.is_some_and(|(x, _)| x > 0.0));
        assert_eq!(out[0].rotation[1], 0.35);
    }
}
