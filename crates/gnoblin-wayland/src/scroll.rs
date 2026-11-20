//! One scroll frame, as the seat routes it.
//!
//! The backend's `InputEvent::PointerAxis` states a wheel: whole and fractional steps in 1/120. A device that
//! is not a wheel says more than that: a finger or a continuous source, a pixel distance with no step count,
//! and an axis stop that ends a kinetic scroll. A virtual pointer is such a device, so its frames reach the
//! seat through [`Scroll`], which a wheel step is also expressed in: there is one routing path for both
//! (`State::route_scroll`), and the wheel is the case where the source is [`AxisSource::Wheel`] and every
//! amount carries its steps.

use smithay::backend::input::AxisSource;

/// What happened on one axis in a scroll frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollAxis {
    /// The distance in logical pixels, as `wl_pointer.axis` states it.
    pub(crate) pixels: f64,
    /// The step count in 1/120 of a step, when the device counts steps (`wl_pointer.axis_value120`).
    pub(crate) steps: Option<i32>,
    /// Whether the axis stopped: a finger lifted, so a client ends its kinetic scroll.
    pub(crate) stop: bool,
}

impl ScrollAxis {
    /// Whether the axis moved nothing and did not stop, so it sends nothing.
    pub(crate) fn is_still(&self) -> bool {
        !self.stop && self.pixels == 0.0 && self.steps.unwrap_or(0) == 0
    }
}

/// One scroll frame: a source and what each axis did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scroll {
    /// What produced the scroll.
    pub(crate) source: AxisSource,
    /// The horizontal axis, `None` when the frame did not touch it.
    pub(crate) horizontal: Option<ScrollAxis>,
    /// The vertical axis, `None` when the frame did not touch it.
    pub(crate) vertical: Option<ScrollAxis>,
}

/// Logical pixels a whole wheel step scrolls.
pub(crate) const PIXELS_PER_STEP: f64 = 15.0;

impl Scroll {
    /// A wheel scroll of `horizontal` and `vertical` in 1/120 of a step; an axis that did not move is untouched.
    pub(crate) fn wheel(horizontal: i32, vertical: i32) -> Self {
        let axis = |steps: i32| {
            (steps != 0).then_some(ScrollAxis {
                pixels: f64::from(steps) / 120.0 * PIXELS_PER_STEP,
                steps: Some(steps),
                stop: false,
            })
        };
        Self {
            source: AxisSource::Wheel,
            horizontal: axis(horizontal),
            vertical: axis(vertical),
        }
    }

    /// Whether the frame moves nothing and stops nothing, so it is never sent.
    pub(crate) fn is_empty(&self) -> bool {
        [self.horizontal, self.vertical]
            .into_iter()
            .flatten()
            .all(|axis| axis.is_still())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_step_is_fifteen_pixels_and_120_units() {
        let scroll = Scroll::wheel(0, 120);
        assert_eq!(scroll.horizontal, None);
        let vertical = scroll.vertical.expect("the vertical axis moved");
        assert_eq!(vertical.steps, Some(120));
        assert!((vertical.pixels - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_scroll_that_moved_nothing_is_empty() {
        assert!(Scroll::wheel(0, 0).is_empty());
    }

    #[test]
    fn a_stop_is_not_empty() {
        let scroll = Scroll {
            source: AxisSource::Finger,
            horizontal: None,
            vertical: Some(ScrollAxis {
                pixels: 0.0,
                steps: None,
                stop: true,
            }),
        };
        assert!(!scroll.is_empty());
    }
}
