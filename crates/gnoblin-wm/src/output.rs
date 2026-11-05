//! Outputs as abstract rectangles and their work areas.

use gnoblin_types::{Insets, OutputId, Rect, Scale};

use crate::geometry::{clamp_to_i32, extent};

/// One connected output.
///
/// The window manager needs the monitor rectangle, the space panels have reserved,
/// and the identity a layout reports to its engine: the connector name and the
/// scale. Mode, transform and gamma belong to `gnoblin-output`. The first output in
/// connection order is the primary output, which is where a new window goes when
/// nothing is focused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Output {
    id: OutputId,
    rect: Rect,
    reserved: Insets,
    name: String,
    scale: Scale,
}

impl Output {
    /// A connected output with nothing reserved.
    ///
    /// The monitor rectangle is floored at one pixel because every rectangle a
    /// client can be configured to sit in is derived from it, and a window that
    /// covers nothing is never reachable input.
    #[must_use]
    pub const fn new(id: OutputId, rect: Rect, reserved: Insets) -> Self {
        Self {
            id,
            rect: rect.at_least_one(),
            reserved,
            name: String::new(),
            scale: Scale::IDENTITY,
        }
    }

    /// The connector name, empty until the caller describes the output with
    /// [`crate::Event::OutputIdentified`].
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The output's scale, [`Scale::IDENTITY`] until the caller describes it.
    #[must_use]
    pub const fn scale(&self) -> Scale {
        self.scale
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
    /// windows at the origin of an empty rectangle. The subtraction runs in [`i64`]
    /// because an inset is signed and a rectangle's extent is not.
    #[must_use]
    pub fn work_area(&self) -> Rect {
        let left = i64::from(self.rect.x) + i64::from(self.reserved.left);
        let top = i64::from(self.rect.y) + i64::from(self.reserved.top);
        let width = i64::from(self.rect.width) - i64::from(self.reserved.left) - i64::from(self.reserved.right);
        let height = i64::from(self.rect.height) - i64::from(self.reserved.top) - i64::from(self.reserved.bottom);
        Rect::new(clamp_to_i32(left), clamp_to_i32(top), extent(width), extent(height))
    }

    /// Replaces the monitor rectangle, keeping the reservation.
    ///
    /// The same one-pixel floor [`Output::new`] applies holds here, so a mode set
    /// cannot make an output's geometry degenerate.
    pub(crate) fn set_rect(&mut self, rect: Rect) {
        self.rect = rect.at_least_one();
    }

    /// Replaces the reservation, keeping the monitor rectangle.
    pub(crate) fn set_reserved(&mut self, reserved: Insets) {
        self.reserved = reserved;
    }

    /// Replaces the connector name and scale that layouts are told about.
    pub(crate) fn set_identity(&mut self, name: String, scale: Scale) {
        self.name = name;
        self.scale = scale;
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
