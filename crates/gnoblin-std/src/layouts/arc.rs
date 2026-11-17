//! [`ArcLayout`]: windows around the viewer on an arc, the focused window at the front.
//!
//! A port of `data/js/gnoblin-std/layouts/arc.ts`, which is the reference. It answers with transform placements: positions on a circle
//! in the x-z plane and a turn about the vertical axis that faces each window towards the centre. On a flat workspace the placement is
//! the window's offset from the centre of the work area (`radius * sin(angle)` across), its depth `z`, its turn about Y and its scale.

use crate::error::{Error, ErrorKind, Result};
use crate::layout::{Layout, LayoutContext, Placement};

/// Window `i` at the angle `(i - selected) * spacing`, where `selected` is the focused window's index (0 when none is focused). Its
/// position is `(sin(angle) * radius, 0, -cos(angle) * radius)` and it is turned `-angle` about Y, so the arc is concave towards a
/// viewer at its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcLayout {
    radius: f64,
    spacing: f64,
}

impl Default for ArcLayout {
    fn default() -> ArcLayout {
        ArcLayout {
            radius: 1200.0,
            spacing: 0.35,
        }
    }
}

impl ArcLayout {
    /// An arc of radius 1200 with 0.35 radians between neighbours.
    pub fn new() -> ArcLayout {
        ArcLayout::default()
    }

    /// An arc of `radius` scene units with `spacing` radians between neighbours.
    ///
    /// # Errors
    ///
    /// `radius` is not a finite number above 0, or `spacing` is not finite.
    pub fn with(radius: f64, spacing: f64) -> Result<ArcLayout> {
        let bad = |name: &str, value: f64, positive: bool| {
            Error::new(
                ErrorKind::Type,
                format!(
                    "ArcLayout: {name} must be a finite number{}, got {value}",
                    if positive { " above 0" } else { "" }
                ),
            )
        };
        if !radius.is_finite() || radius <= 0.0 {
            return Err(bad("radius", radius, true));
        }
        if !spacing.is_finite() {
            return Err(bad("spacing", spacing, false));
        }
        Ok(ArcLayout { radius, spacing })
    }

    /// The distance from the centre of the arc to each window, in scene units.
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// The angle between neighbouring windows, in radians.
    pub fn spacing(&self) -> f64 {
        self.spacing
    }
}

impl Layout for ArcLayout {
    fn compute(&mut self, cx: &LayoutContext) -> Vec<Placement> {
        let at = cx
            .focused
            .as_ref()
            .and_then(|focused| cx.children.iter().position(|child| child == focused));
        let selected = at.unwrap_or(0);
        cx.children
            .iter()
            .enumerate()
            .map(|(i, client)| {
                let angle = (i as f64 - selected as f64) * self.spacing;
                Placement::new(client)
                    .offset(angle.sin() * self.radius, 0.0)
                    .z(-angle.cos() * self.radius)
                    .rotation([0.0, -angle, 0.0])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_are_checked() {
        assert!(ArcLayout::with(0.0, 0.35).is_err());
        assert!(ArcLayout::with(1200.0, f64::NAN).is_err());
        assert_eq!(ArcLayout::with(900.0, 0.2).map(|arc| arc.radius()), Ok(900.0));
        assert_eq!(ArcLayout::new().spacing(), 0.35);
    }
}
