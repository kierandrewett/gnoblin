//! Outputs as abstract rectangles and their work areas.

use crate::geometry::{Insets, Rect};
use crate::ids::OutputId;

/// One connected output.
///
/// The window manager only needs the monitor rectangle and the space panels have
/// reserved; mode, transform, scale and gamma belong to `gnoblin-output`. The
/// first output in connection order is the primary output, which is where a new
/// window goes when nothing is focused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Output {
    id: OutputId,
    rect: Rect,
    reserved: Insets,
}

impl Output {
    /// A connected output with nothing reserved.
    #[must_use]
    pub const fn new(id: OutputId, rect: Rect, reserved: Insets) -> Self {
        Self { id, rect, reserved }
    }

    /// The caller's identity for this output.
    #[must_use]
    pub const fn id(&self) -> OutputId {
        self.id
    }

    /// The whole monitor rectangle, which is what a fullscreen window covers.
    #[must_use]
    pub const fn rect(&self) -> Rect {
        self.rect
    }

    /// Space reserved by panels and bars.
    #[must_use]
    pub const fn reserved(&self) -> Insets {
        self.reserved
    }

    /// The monitor rectangle minus the reserved insets, never smaller than a pixel.
    ///
    /// An output whose panels ask for more space than it has still yields a valid
    /// work area, so a broken panel cannot make placement divide by zero or place
    /// windows at the origin of an empty rectangle.
    #[must_use]
    pub fn work_area(&self) -> Rect {
        Rect::new(
            self.rect.x.saturating_add(self.reserved.left),
            self.rect.y.saturating_add(self.reserved.top),
            self.rect
                .width
                .saturating_sub(self.reserved.left.saturating_add(self.reserved.right)),
            self.rect
                .height
                .saturating_sub(self.reserved.top.saturating_add(self.reserved.bottom)),
        )
    }

    /// Replaces the monitor rectangle, keeping the reservation.
    pub(crate) fn set_rect(&mut self, rect: Rect) {
        self.rect = rect;
    }

    /// Replaces the reservation, keeping the monitor rectangle.
    pub(crate) fn set_reserved(&mut self, reserved: Insets) {
        self.reserved = reserved;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_area_is_the_monitor_minus_reserved_space() {
        let output = Output::new(OutputId::new(0), Rect::new(0, 0, 1920, 1080), Insets::default());
        assert_eq!(output.work_area(), Rect::new(0, 0, 1920, 1080));
        let reserved = Insets {
            top: 32,
            right: 0,
            bottom: 48,
            left: 64,
        };
        let output = Output::new(OutputId::new(0), Rect::new(0, 0, 1920, 1080), reserved);
        assert_eq!(output.work_area(), Rect::new(64, 32, 1856, 1000));
    }

    #[test]
    fn an_oversized_reservation_still_yields_a_work_area() {
        let output = Output::new(OutputId::new(1), Rect::new(0, 0, 100, 100), Insets::uniform(500));
        assert_eq!(output.work_area(), Rect::new(500, 500, 1, 1));
    }
}
