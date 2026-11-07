//! Colours as linear components from 0 to 1.
//!
//! The public configuration format accepts `#RRGGBB` and `#RRGGBBAA`; parsing
//! belongs to the configuration layer, so this crate carries the value it
//! produces instead of a second parser. Components are the stored display
//! values, which is what tinting and per-channel interpolation need: an
//! overshooting easing can leave a channel outside 0 to 1 during a frame, and
//! the renderer clamps at draw time without the tween having to know.

/// A colour with an alpha component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red, 0 to 1.
    pub r: f32,
    /// Green, 0 to 1.
    pub g: f32,
    /// Blue, 0 to 1.
    pub b: f32,
    /// Alpha, 0 to 1, where 0 is transparent.
    pub a: f32,
}

impl Color {
    /// Opaque white, the identity for a tint.
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    /// Transparent black, the identity for "draws nothing".
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
    /// Opaque black.
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    /// A colour from its components.
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// A colour from 8-bit channels, the form a colour picker produces.
    #[must_use]
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    }

    /// The same colour at a different alpha.
    #[must_use]
    pub const fn with_alpha(self, a: f32) -> Self {
        Self::new(self.r, self.g, self.b, a)
    }

    /// Whether every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }

    /// Whether every component lies in the 0 to 1 range a declaration may use.
    #[must_use]
    pub fn is_in_unit_range(self) -> bool {
        self.is_finite()
            && (0.0..=1.0).contains(&self.r)
            && (0.0..=1.0).contains(&self.g)
            && (0.0..=1.0).contains(&self.b)
            && (0.0..=1.0).contains(&self.a)
    }

    /// Linear interpolation towards `other` at progress `t`.
    ///
    /// Channels interpolate independently and are not premultiplied: a tint is
    /// a multiplier, not a composited layer, so interpolating a transparent
    /// colour towards an opaque one must not darken it on the way.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_bit_conversion_round_trips_the_endpoints() {
        assert_eq!(
            Color::from_rgba8(255, 0, 128, 255),
            Color::new(1.0, 0.0, 128.0 / 255.0, 1.0)
        );
    }

    #[test]
    fn a_component_outside_the_range_is_not_a_declaration_value() {
        assert!(!Color::new(1.5, 0.0, 0.0, 1.0).is_in_unit_range());
        assert!(!Color::new(0.0, 0.0, 0.0, f32::NAN).is_finite());
    }
}
