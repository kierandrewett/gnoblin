//! The visual properties a tween can target, and their typed values.
//!
//! A property is addressed by name, not by a closure or an offset into the
//! transform, so the tween player can target every component the scene model
//! has, including the depth components. The player is axis-agnostic: the scene
//! decides only whether a resulting transform is finite and invertible. A track
//! on `scale_z` therefore needs no new player code.
//!
//! Values are typed ([`PropertyValue`]) rather than all `f32`: a tint is a
//! colour and a uniform scale is two in-plane components, and flattening them
//! into one numeric channel would make a wrong-kind assignment a runtime
//! surprise instead of a typed refusal.

use crate::transform::VisualTransform;
use gnoblin_types::Color;

/// The kind of value a property accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// One scalar.
    Number,
    /// Two scalars, used by the uniform in-plane scale.
    Vector2,
    /// Four colour components.
    Color,
}

/// A property's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropertyValue {
    /// A scalar: opacity, an offset, a rotation, one scale axis or a radius.
    Number(f32),
    /// Two in-plane scalars.
    Vector2([f32; 2]),
    /// A colour.
    Color(Color),
}

impl PropertyValue {
    /// A scalar value.
    #[must_use]
    pub const fn number(value: f32) -> Self {
        Self::Number(value)
    }

    /// The same scale on both in-plane axes, which is what a declaration's
    /// `scale` field means.
    #[must_use]
    pub const fn uniform_scale(value: f32) -> Self {
        Self::Vector2([value, value])
    }

    /// A colour value.
    #[must_use]
    pub const fn color(value: Color) -> Self {
        Self::Color(value)
    }

    /// The value's kind.
    #[must_use]
    pub const fn kind(self) -> ValueKind {
        match self {
            Self::Number(_) => ValueKind::Number,
            Self::Vector2(_) => ValueKind::Vector2,
            Self::Color(_) => ValueKind::Color,
        }
    }

    /// Whether every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        match self {
            Self::Number(value) => value.is_finite(),
            Self::Vector2([x, y]) => x.is_finite() && y.is_finite(),
            Self::Color(color) => color.is_finite(),
        }
    }

    /// Linear interpolation towards `other` at progress `t`.
    ///
    /// Returns `None` when the kinds differ; the tween player validates the
    /// kinds once at start, so a `None` here is a bug rather than a state a
    /// caller can reach.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Option<Self> {
        match (self, other) {
            (Self::Number(from), Self::Number(to)) => Some(Self::Number(from + (to - from) * t)),
            (Self::Vector2(from), Self::Vector2(to)) => Some(Self::Vector2([
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
            ])),
            (Self::Color(from), Self::Color(to)) => Some(Self::Color(from.lerp(to, t))),
            _ => None,
        }
    }
}

/// A property the tween player and the scene can address by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VisualProperty {
    /// Group opacity, 0 to 1.
    Opacity,
    /// Translation on the first axis.
    OffsetX,
    /// Translation on the second axis.
    OffsetY,
    /// Translation on the depth axis.
    OffsetZ,
    /// Rotation about the first axis.
    RotationX,
    /// Rotation about the second axis.
    RotationY,
    /// Rotation about the depth axis, the 2D rotation.
    RotationZ,
    /// The same scale on both in-plane axes.
    Scale,
    /// Scale on the first axis.
    ScaleX,
    /// Scale on the second axis.
    ScaleY,
    /// Scale on the depth axis.
    ScaleZ,
    /// Multiplicative colour.
    Tint,
    /// Corner radius in logical pixels.
    CornerRadius,
}

impl VisualProperty {
    /// Every property, in a fixed order the player and the model test use to
    /// make a tie deterministic.
    pub const ALL: [Self; 13] = [
        Self::Opacity,
        Self::OffsetX,
        Self::OffsetY,
        Self::OffsetZ,
        Self::RotationX,
        Self::RotationY,
        Self::RotationZ,
        Self::Scale,
        Self::ScaleX,
        Self::ScaleY,
        Self::ScaleZ,
        Self::Tint,
        Self::CornerRadius,
    ];

    /// The stable member name, matching the configuration and IPC spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Opacity => "opacity",
            Self::OffsetX => "offset_x",
            Self::OffsetY => "offset_y",
            Self::OffsetZ => "offset_z",
            Self::RotationX => "rotation_x",
            Self::RotationY => "rotation_y",
            Self::RotationZ => "rotation_z",
            Self::Scale => "scale",
            Self::ScaleX => "scale_x",
            Self::ScaleY => "scale_y",
            Self::ScaleZ => "scale_z",
            Self::Tint => "tint",
            Self::CornerRadius => "corner_radius",
        }
    }

    /// The kind of value this property accepts.
    #[must_use]
    pub const fn expected(self) -> ValueKind {
        match self {
            Self::Scale => ValueKind::Vector2,
            Self::Tint => ValueKind::Color,
            _ => ValueKind::Number,
        }
    }

    /// The value a transform carries for this property.
    #[must_use]
    pub fn get(self, transform: &VisualTransform) -> PropertyValue {
        match self {
            Self::Opacity => PropertyValue::Number(transform.opacity),
            Self::OffsetX => PropertyValue::Number(transform.base.offset.x as f32),
            Self::OffsetY => PropertyValue::Number(transform.base.offset.y as f32),
            Self::OffsetZ => PropertyValue::Number(transform.base.offset.z as f32),
            Self::RotationX => PropertyValue::Number(transform.base.rotation_degrees.x as f32),
            Self::RotationY => PropertyValue::Number(transform.base.rotation_degrees.y as f32),
            Self::RotationZ => PropertyValue::Number(transform.base.rotation_degrees.z as f32),
            Self::Scale => PropertyValue::Vector2([transform.base.scale.x as f32, transform.base.scale.y as f32]),
            Self::ScaleX => PropertyValue::Number(transform.base.scale.x as f32),
            Self::ScaleY => PropertyValue::Number(transform.base.scale.y as f32),
            Self::ScaleZ => PropertyValue::Number(transform.base.scale.z as f32),
            Self::Tint => PropertyValue::Color(transform.tint),
            Self::CornerRadius => PropertyValue::Number(transform.corner_radius),
        }
    }

    /// Accept a scalar where the property means "the same on both axes".
    ///
    /// The animation guide writes `scale = 0.9` for a uniform in-plane scale, so
    /// the scene layer would otherwise need a second value kind or a special
    /// case at every entry point.
    pub fn coerce(self, value: PropertyValue) -> Result<PropertyValue, PropertyError> {
        match (self, value) {
            (Self::Scale, PropertyValue::Number(value)) => Ok(PropertyValue::Vector2([value, value])),
            (property, value) if value.kind() == property.expected() => Ok(value),
            (property, value) => Err(PropertyError::WrongKind {
                property,
                expected: property.expected(),
                got: value.kind(),
            }),
        }
    }

    /// Apply a value, refusing a wrong kind or a non-finite component.
    ///
    /// The resulting transform is not checked here: [`crate::scene::Scene`]
    /// rejects a transform outside the 2D subset as one unit, after the property
    /// has been applied to a candidate copy. That keeps opacity, tint and every
    /// other 2D member working while a depth component can still be targeted by
    /// a track and refused in one place.
    pub fn apply(self, transform: &mut VisualTransform, value: PropertyValue) -> Result<(), PropertyError> {
        let value = self.coerce(value)?;
        if !value.is_finite() {
            return Err(PropertyError::NonFinite { property: self });
        }
        match (self, value) {
            (Self::Opacity, PropertyValue::Number(value)) => transform.opacity = value,
            (Self::OffsetX, PropertyValue::Number(value)) => transform.base.offset.x = f64::from(value),
            (Self::OffsetY, PropertyValue::Number(value)) => transform.base.offset.y = f64::from(value),
            (Self::OffsetZ, PropertyValue::Number(value)) => transform.base.offset.z = f64::from(value),
            (Self::RotationX, PropertyValue::Number(value)) => transform.base.rotation_degrees.x = f64::from(value),
            (Self::RotationY, PropertyValue::Number(value)) => transform.base.rotation_degrees.y = f64::from(value),
            (Self::RotationZ, PropertyValue::Number(value)) => transform.base.rotation_degrees.z = f64::from(value),
            (Self::Scale, PropertyValue::Vector2(value)) => {
                transform.base.scale.x = f64::from(value[0]);
                transform.base.scale.y = f64::from(value[1]);
            }
            (Self::ScaleX, PropertyValue::Number(value)) => transform.base.scale.x = f64::from(value),
            (Self::ScaleY, PropertyValue::Number(value)) => transform.base.scale.y = f64::from(value),
            (Self::ScaleZ, PropertyValue::Number(value)) => transform.base.scale.z = f64::from(value),
            (Self::Tint, PropertyValue::Color(value)) => transform.tint = value,
            (Self::CornerRadius, PropertyValue::Number(value)) => transform.corner_radius = value,
            // `coerce` above has already matched the kind, so these arms are
            // unreachable; the wildcard keeps the match total without a panic.
            (property, value) => {
                return Err(PropertyError::WrongKind {
                    property,
                    expected: property.expected(),
                    got: value.kind(),
                });
            }
        }
        Ok(())
    }

    /// The value a default transform carries.
    #[must_use]
    pub fn default_value(self) -> PropertyValue {
        self.get(&VisualTransform::IDENTITY)
    }

    /// Whether a track on this property excludes a track on `other`.
    ///
    /// Two tracks may coexist only when they write disjoint components. A
    /// uniform `scale` writes both in-plane axes, so it excludes `scale_x` and
    /// `scale_y` in either order; `scale_x` and `scale_y` do not exclude each
    /// other.
    #[must_use]
    pub const fn conflicts_with(self, other: Self) -> bool {
        if matches!(
            (self, other),
            (Self::Scale, Self::Scale | Self::ScaleX | Self::ScaleY) | (Self::ScaleX | Self::ScaleY, Self::Scale)
        ) {
            return true;
        }
        matches!(
            (self, other),
            (Self::Opacity, Self::Opacity)
                | (Self::OffsetX, Self::OffsetX)
                | (Self::OffsetY, Self::OffsetY)
                | (Self::OffsetZ, Self::OffsetZ)
                | (Self::RotationX, Self::RotationX)
                | (Self::RotationY, Self::RotationY)
                | (Self::RotationZ, Self::RotationZ)
                | (Self::ScaleX, Self::ScaleX)
                | (Self::ScaleY, Self::ScaleY)
                | (Self::ScaleZ, Self::ScaleZ)
                | (Self::Tint, Self::Tint)
                | (Self::CornerRadius, Self::CornerRadius)
        )
    }
}

/// A refusal while reading or writing a property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyError {
    /// The value's kind is not the property's kind.
    WrongKind {
        /// The property.
        property: VisualProperty,
        /// The kind it needs.
        expected: ValueKind,
        /// The kind supplied.
        got: ValueKind,
    },
    /// A component is NaN or infinite.
    NonFinite {
        /// The property.
        property: VisualProperty,
    },
}

impl std::fmt::Display for PropertyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongKind {
                property,
                expected,
                got,
            } => write!(
                formatter,
                "`{}` expects a {expected:?} value, got a {got:?} value",
                property.name()
            ),
            Self::NonFinite { property } => write!(formatter, "`{}` is not finite", property.name()),
        }
    }
}

impl std::error::Error for PropertyError {}

#[cfg(test)]
mod tests {
    use gnoblin_types::Vec3;

    use super::*;

    #[test]
    fn a_uniform_scale_accepts_a_scalar_and_reads_back_two_axes() {
        let mut transform = VisualTransform::IDENTITY;
        let value = VisualProperty::Scale
            .coerce(PropertyValue::number(0.5))
            .expect("coercible");
        VisualProperty::Scale.apply(&mut transform, value).expect("accepted");
        assert_eq!(transform.base.scale, Vec3::new(0.5, 0.5, 1.0));
        assert_eq!(
            VisualProperty::Scale.get(&transform),
            PropertyValue::Vector2([0.5, 0.5])
        );
    }

    #[test]
    fn a_wrong_kind_is_refused() {
        let error = VisualProperty::Opacity
            .coerce(PropertyValue::color(Color::BLACK))
            .expect_err("must refuse");
        assert_eq!(
            error,
            PropertyError::WrongKind {
                property: VisualProperty::Opacity,
                expected: ValueKind::Number,
                got: ValueKind::Color,
            }
        );
    }

    #[test]
    fn a_non_finite_value_is_refused() {
        let mut transform = VisualTransform::IDENTITY;
        let error = VisualProperty::Opacity
            .apply(&mut transform, PropertyValue::number(f32::NAN))
            .expect_err("must refuse");
        assert_eq!(
            error,
            PropertyError::NonFinite {
                property: VisualProperty::Opacity
            }
        );
        assert_eq!(transform.opacity, 1.0);
    }

    #[test]
    fn a_uniform_scale_excludes_both_axis_tracks() {
        assert!(VisualProperty::Scale.conflicts_with(VisualProperty::ScaleX));
        assert!(VisualProperty::Scale.conflicts_with(VisualProperty::ScaleY));
        assert!(VisualProperty::ScaleY.conflicts_with(VisualProperty::Scale));
        assert!(!VisualProperty::ScaleX.conflicts_with(VisualProperty::ScaleY));
        assert!(!VisualProperty::OffsetX.conflicts_with(VisualProperty::OffsetY));
    }
}
