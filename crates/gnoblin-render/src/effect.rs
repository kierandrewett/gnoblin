//! Effects: an ordered list of named passes with typed uniforms.
//!
//! The plan gives the renderer effects as GLES passes (Renderer card). This
//! module is the scene-side description of those passes: a pass names what it
//! is, carries the values the shader or effect needs, and is validated where it
//! enters the scene so the renderer never has to guess. A user shader declares
//! its uniforms in a [`UniformSchema`]; every value set on it is checked against
//! that declaration, so a float written into a `vec4` slot is refused before a
//! frame is drawn rather than silently reading as the wrong component.
//!
//! Order is the pass order: a list is applied in the order it was pushed, which
//! is what makes "blur then recolour" a different description from "recolour
//! then blur".
//!
//! Shadow offsets are [`Vec3`] even though this build only accepts `z == 0`, for
//! the same reason transforms are: a depth value is refused with the named
//! `unsupported-3d` error instead of being silently dropped.

use std::collections::BTreeMap;

use gnoblin_types::{Color, RectF, Unsupported3d, Unsupported3dField, Vec3};

/// The largest number of passes a node may carry.
///
/// Each pass is a render target tap, so an unbounded list is an unbounded frame
/// cost. The cap is a renderer bound, not a preference: a node over it is
/// refused and the caller can simplify the chain.
pub const MAX_PASSES: usize = 32;

/// The largest number of uniforms one user shader may declare.
pub const MAX_UNIFORMS: usize = 32;

/// The largest blur radius a pass accepts, in logical pixels.
pub const MAX_BLUR_RADIUS: f32 = 100.0;

/// A uniform's declared type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UniformType {
    /// One float.
    Float,
    /// Two floats.
    Vec2,
    /// Three floats.
    Vec3,
    /// Four floats.
    Vec4,
    /// One signed integer.
    Int,
}

impl UniformType {
    /// The type's spelling in a uniform declaration.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Vec4 => "vec4",
            Self::Int => "int",
        }
    }

    /// The value a shader reads when the caller set nothing.
    ///
    /// The shader guide documents unset custom uniforms as zero; keeping the
    /// default here means a declaration alone is enough to render.
    #[must_use]
    pub const fn default_value(self) -> UniformValue {
        match self {
            Self::Float => UniformValue::Float(0.0),
            Self::Vec2 => UniformValue::Vec2([0.0; 2]),
            Self::Vec3 => UniformValue::Vec3([0.0; 3]),
            Self::Vec4 => UniformValue::Vec4([0.0; 4]),
            Self::Int => UniformValue::Int(0),
        }
    }
}

/// A uniform's value, carrying its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UniformValue {
    /// One float.
    Float(f32),
    /// Two floats.
    Vec2([f32; 2]),
    /// Three floats.
    Vec3([f32; 3]),
    /// Four floats.
    Vec4([f32; 4]),
    /// One signed integer.
    Int(i32),
}

impl UniformValue {
    /// The value's type.
    #[must_use]
    pub const fn kind(self) -> UniformType {
        match self {
            Self::Float(_) => UniformType::Float,
            Self::Vec2(_) => UniformType::Vec2,
            Self::Vec3(_) => UniformType::Vec3,
            Self::Vec4(_) => UniformType::Vec4,
            Self::Int(_) => UniformType::Int,
        }
    }

    /// Whether every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        match self {
            Self::Float(value) => value.is_finite(),
            Self::Vec2(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec3(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec4(values) => values.iter().all(|value| value.is_finite()),
            Self::Int(_) => true,
        }
    }
}

/// The uniforms a user shader declares, with their types.
///
/// The schema is a name and type list, not a map, so the renderer can build the
/// shader's uniform block in declaration order and the refusal for a duplicate
/// name is a property of the declaration rather than of insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UniformSchema {
    entries: Vec<(String, UniformType)>,
}

impl UniformSchema {
    /// A schema from its declarations.
    pub fn new(entries: Vec<(&str, UniformType)>) -> Result<Self, EffectError> {
        if entries.len() > MAX_UNIFORMS {
            return Err(EffectError::TooManyUniforms {
                max: MAX_UNIFORMS,
                got: entries.len(),
            });
        }
        let mut declared: Vec<(String, UniformType)> = Vec::with_capacity(entries.len());
        for (name, kind) in entries {
            validate_uniform_name(name)?;
            if declared.iter().any(|(existing, _)| existing == name) {
                return Err(EffectError::DuplicateUniform { name: name.to_owned() });
            }
            declared.push((name.to_owned(), kind));
        }
        Ok(Self { entries: declared })
    }

    /// The declarations, in order.
    #[must_use]
    pub fn entries(&self) -> &[(String, UniformType)] {
        &self.entries
    }

    /// The declared type of a name.
    #[must_use]
    pub fn kind_of(&self, name: &str) -> Option<UniformType> {
        self.entries
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, kind)| *kind)
    }

    /// The number of declarations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the shader declares no uniform.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Check a uniform name against the shader guide's rules.
fn validate_uniform_name(name: &str) -> Result<(), EffectError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if !valid {
        return Err(EffectError::InvalidUniformName { name: name.to_owned() });
    }
    if name.starts_with("gnoblin_") {
        return Err(EffectError::ReservedUniform { name: name.to_owned() });
    }
    Ok(())
}

/// A resolved uniform, in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedUniform {
    /// The declared name.
    pub name: String,
    /// The declared type.
    pub kind: UniformType,
    /// The value, or the type's zero when the caller set none.
    pub value: UniformValue,
}

/// A user shader pass with its declared uniforms and current values.
#[derive(Clone, Debug, PartialEq)]
pub struct Shader {
    schema: UniformSchema,
    values: BTreeMap<String, UniformValue>,
}

impl Shader {
    /// A shader from a validated schema.
    #[must_use]
    pub fn new(schema: UniformSchema) -> Self {
        Self {
            schema,
            values: BTreeMap::new(),
        }
    }

    /// The shader's declarations.
    #[must_use]
    pub fn schema(&self) -> &UniformSchema {
        &self.schema
    }

    /// Set a declared uniform.
    ///
    /// An undeclared name is refused rather than ignored: the shader never reads
    /// it, so accepting it would make a typo look like a working configuration.
    pub fn set_uniform(&mut self, name: &str, value: UniformValue) -> Result<(), EffectError> {
        let expected = self
            .schema
            .kind_of(name)
            .ok_or_else(|| EffectError::UnknownUniform { name: name.to_owned() })?;
        if expected != value.kind() {
            return Err(EffectError::WrongUniformType {
                name: name.to_owned(),
                expected,
                got: value.kind(),
            });
        }
        if !value.is_finite() {
            return Err(EffectError::NonFinite { field: "uniform" });
        }
        self.values.insert(name.to_owned(), value);
        Ok(())
    }

    /// The value set for a name, if any.
    #[must_use]
    pub fn uniform(&self, name: &str) -> Option<UniformValue> {
        self.values.get(name).copied()
    }

    /// Every declared uniform with its value, in declaration order.
    #[must_use]
    pub fn resolved(&self) -> Vec<ResolvedUniform> {
        self.schema
            .entries()
            .iter()
            .map(|(name, kind)| ResolvedUniform {
                name: name.clone(),
                kind: *kind,
                value: self.values.get(name).copied().unwrap_or(kind.default_value()),
            })
            .collect()
    }
}

/// The most blur passes one blur effect may run.
pub const MAX_BLUR_PASSES: u32 = 8;

/// The smallest fraction of full resolution a blur samples its backdrop at.
pub const MIN_BLUR_DOWNSCALE: f32 = 0.125;

/// The largest brightness, contrast or saturation multiplier a blur accepts.
///
/// The multipliers are plain gains: 1 leaves the backdrop unchanged, 0 removes the
/// channel, and values above 1 boost it. The bound keeps a gain from blowing the
/// backdrop out to white in one step.
pub const MAX_BLUR_GRADE: f32 = 2.0;

/// A blur pass over the node and how its blurred backdrop is graded.
///
/// Every value has a documented range. [`BlurParams::clamped`] brings any value into
/// its range, and [`Pass::validate`] refuses a value outside it, so a caller that
/// wants clamping rather than an error clamps first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlurParams {
    /// Radius in logical pixels, 0 to [`MAX_BLUR_RADIUS`].
    pub radius: f32,
    /// Blur passes, 1 to [`MAX_BLUR_PASSES`]. Each pass sweeps horizontally then
    /// vertically, so more passes blur more at the same radius.
    pub passes: u32,
    /// Fraction of full resolution the backdrop is sampled at, from
    /// [`MIN_BLUR_DOWNSCALE`] to 1. A smaller fraction is cheaper and blurrier.
    pub downscale: f32,
    /// A tint mixed over the blurred backdrop. Its alpha is the mix amount, so alpha 0
    /// means no tint.
    pub tint: Color,
    /// Brightness gain, 0 to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub brightness: f32,
    /// Contrast gain about mid grey, 0 to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub contrast: f32,
    /// Saturation gain, 0 (grey) to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub saturation: f32,
    /// Grain added after blurring, 0 to 1; 0 adds none.
    pub noise: f32,
}

impl Default for BlurParams {
    /// The schema's defaults: a 12 pixel radius, three passes at half resolution (the
    /// schema's divisor 2), no tint, no grading and no grain.
    fn default() -> Self {
        Self {
            radius: 12.0,
            passes: 3,
            downscale: 0.5,
            tint: Color::TRANSPARENT,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            noise: 0.0,
        }
    }
}

impl BlurParams {
    /// These parameters with every value brought into its documented range.
    ///
    /// Out-of-range values clamp to the nearest bound. A non-finite value takes the
    /// default for its field, because a NaN has no nearest bound. Clamping never fails.
    #[must_use]
    pub fn clamped(self) -> Self {
        let default = Self::default();
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        Self {
            radius: finite(self.radius, default.radius).clamp(0.0, MAX_BLUR_RADIUS),
            passes: self.passes.clamp(1, MAX_BLUR_PASSES),
            downscale: finite(self.downscale, default.downscale).clamp(MIN_BLUR_DOWNSCALE, 1.0),
            tint: Color {
                r: finite(self.tint.r, 0.0).clamp(0.0, 1.0),
                g: finite(self.tint.g, 0.0).clamp(0.0, 1.0),
                b: finite(self.tint.b, 0.0).clamp(0.0, 1.0),
                a: finite(self.tint.a, 0.0).clamp(0.0, 1.0),
            },
            brightness: finite(self.brightness, default.brightness).clamp(0.0, MAX_BLUR_GRADE),
            contrast: finite(self.contrast, default.contrast).clamp(0.0, MAX_BLUR_GRADE),
            saturation: finite(self.saturation, default.saturation).clamp(0.0, MAX_BLUR_GRADE),
            noise: finite(self.noise, default.noise).clamp(0.0, 1.0),
        }
    }
}

/// A drop shadow pass drawn behind the node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    /// Offset of the shadow in the node's local space.
    pub offset: Vec3,
    /// Blur radius in logical pixels, 0 to [`MAX_BLUR_RADIUS`].
    pub blur: f32,
    /// Growth of the shadow past the node's edge before it is blurred, in logical
    /// pixels, from -[`MAX_BLUR_RADIUS`] to [`MAX_BLUR_RADIUS`]. Positive grows the
    /// shadow; negative shrinks it, so a shadow can sit inside the node's outline.
    pub spread: f32,
    /// Shadow opacity, 0 to 1.
    pub opacity: f32,
    /// Shadow colour; its alpha multiplies `opacity`.
    pub color: Color,
}

/// One named pass in an effect chain.
#[derive(Clone, Debug, PartialEq)]
pub enum Pass {
    /// A blur.
    Blur(BlurParams),
    /// A drop shadow.
    Shadow(Shadow),
    /// A user fragment shader.
    Shader(Shader),
}

impl Pass {
    /// The pass's name, as inspection tools report it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Blur(_) => "blur",
            Self::Shadow(_) => "shadow",
            Self::Shader(_) => "shader",
        }
    }

    /// Validate the pass's values.
    ///
    /// Called by [`Effects::push`]; keeping it public lets a caller re-check a
    /// pass it built earlier without reconstructing the chain.
    pub fn validate(&self) -> Result<(), EffectError> {
        match self {
            Self::Blur(blur) => validate_blur_params(blur),
            Self::Shadow(shadow) => {
                validate_blur(shadow.blur)?;
                if !shadow.spread.is_finite() {
                    return Err(EffectError::NonFinite { field: "shadow.spread" });
                }
                if !(-MAX_BLUR_RADIUS..=MAX_BLUR_RADIUS).contains(&shadow.spread) {
                    return Err(EffectError::OutOfRange {
                        field: "shadow.spread",
                        min: -MAX_BLUR_RADIUS,
                        max: MAX_BLUR_RADIUS,
                    });
                }
                if shadow.offset.z != 0.0 {
                    return Err(EffectError::Unsupported3d(Unsupported3d::new(
                        Unsupported3dField::OffsetZ,
                    )));
                }
                if !shadow.offset.is_finite() {
                    return Err(EffectError::NonFinite { field: "shadow.offset" });
                }
                if !shadow.opacity.is_finite() {
                    return Err(EffectError::NonFinite {
                        field: "shadow.opacity",
                    });
                }
                if !(0.0..=1.0).contains(&shadow.opacity) {
                    return Err(EffectError::OutOfRange {
                        field: "shadow.opacity",
                        min: 0.0,
                        max: 1.0,
                    });
                }
                if !shadow.color.is_in_unit_range() {
                    return Err(EffectError::OutOfRange {
                        field: "shadow.color",
                        min: 0.0,
                        max: 1.0,
                    });
                }
                Ok(())
            }
            Self::Shader(shader) => {
                for resolved in shader.resolved() {
                    if !resolved.value.is_finite() {
                        return Err(EffectError::NonFinite { field: "uniform" });
                    }
                }
                Ok(())
            }
        }
    }

    /// How far outside the node's bounds the pass can draw.
    ///
    /// Only a shadow draws outside its node today; a blur samples within the
    /// node's own rectangle. The value is conservative on purpose: damage that
    /// is slightly too large costs a few pixels, damage that is too small leaves
    /// a stale shadow on screen.
    #[must_use]
    pub fn padding(&self) -> EffectExtent {
        match self {
            Self::Shadow(shadow) => {
                // Growth is the spread (when positive) plus the blur. A negative
                // spread shrinks the shadow, so it never adds padding.
                let blur = f64::from(shadow.blur.max(0.0) + shadow.spread.max(0.0));
                EffectExtent {
                    left: (blur - shadow.offset.x).max(0.0),
                    top: (blur - shadow.offset.y).max(0.0),
                    right: (blur + shadow.offset.x).max(0.0),
                    bottom: (blur + shadow.offset.y).max(0.0),
                }
            }
            Self::Blur(_) | Self::Shader(_) => EffectExtent::ZERO,
        }
    }
}

/// Checks a blur radius before a pass is built: it must be finite and lie within
/// the largest radius the renderer accepts.
fn validate_blur(radius: f32) -> Result<(), EffectError> {
    if !radius.is_finite() {
        return Err(EffectError::NonFinite { field: "blur.radius" });
    }
    if !(0.0..=MAX_BLUR_RADIUS).contains(&radius) {
        return Err(EffectError::OutOfRange {
            field: "blur.radius",
            min: 0.0,
            max: MAX_BLUR_RADIUS,
        });
    }
    Ok(())
}

/// Checks every value of a blur pass before the pass is built.
///
/// Each value must be finite and inside its documented range; a refusal names the
/// first field out of range. [`BlurParams::clamped`] is the lenient alternative.
fn validate_blur_params(blur: &BlurParams) -> Result<(), EffectError> {
    validate_blur(blur.radius)?;
    if !(1..=MAX_BLUR_PASSES).contains(&blur.passes) {
        return Err(EffectError::OutOfRange {
            field: "blur.passes",
            min: 1.0,
            max: MAX_BLUR_PASSES as f32,
        });
    }
    for (field, value, min, max) in [
        ("blur.downscale", blur.downscale, MIN_BLUR_DOWNSCALE, 1.0),
        ("blur.brightness", blur.brightness, 0.0, MAX_BLUR_GRADE),
        ("blur.contrast", blur.contrast, 0.0, MAX_BLUR_GRADE),
        ("blur.saturation", blur.saturation, 0.0, MAX_BLUR_GRADE),
        ("blur.noise", blur.noise, 0.0, 1.0),
    ] {
        if !value.is_finite() {
            return Err(EffectError::NonFinite { field });
        }
        if !(min..=max).contains(&value) {
            return Err(EffectError::OutOfRange { field, min, max });
        }
    }
    let tint = blur.tint;
    if ![tint.r, tint.g, tint.b, tint.a].iter().all(|v| v.is_finite()) {
        return Err(EffectError::NonFinite { field: "blur.tint" });
    }
    if !tint.is_in_unit_range() {
        return Err(EffectError::OutOfRange {
            field: "blur.tint",
            min: 0.0,
            max: 1.0,
        });
    }
    Ok(())
}

/// An ordered effect chain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    passes: Vec<Pass>,
}

impl Effects {
    /// An empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a pass, validating it first.
    ///
    /// A refused pass leaves the chain unchanged, so a partly built chain never
    /// contains a value the renderer would have to defend against.
    pub fn push(&mut self, pass: Pass) -> Result<(), EffectError> {
        if self.passes.len() >= MAX_PASSES {
            return Err(EffectError::TooManyPasses {
                max: MAX_PASSES,
                got: self.passes.len() + 1,
            });
        }
        pass.validate()?;
        self.passes.push(pass);
        Ok(())
    }

    /// The passes, in application order.
    #[must_use]
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// The number of passes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    /// Whether the chain is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    /// The largest padding any pass needs, on each side.
    #[must_use]
    pub fn padding(&self) -> EffectExtent {
        self.passes
            .iter()
            .map(Pass::padding)
            .fold(EffectExtent::ZERO, EffectExtent::max)
    }

    /// The node's local bounds expanded by the chain's padding.
    #[must_use]
    pub fn expand(&self, bounds: RectF) -> RectF {
        self.padding().expand(bounds)
    }
}

/// Space an effect draws outside a node's own bounds, in continuous logical
/// pixels.
///
/// This is not `gnoblin_types::Insets`: insets are whole-pixel reservations
/// counted inwards, while a shadow's extent can be fractional and grows
/// outwards.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectExtent {
    /// Pixels added on the left.
    pub left: f64,
    /// Pixels added above.
    pub top: f64,
    /// Pixels added on the right.
    pub right: f64,
    /// Pixels added below.
    pub bottom: f64,
}

impl EffectExtent {
    /// No padding.
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// The component-wise larger padding.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Grow a rectangle by this padding.
    #[must_use]
    pub fn expand(self, rect: RectF) -> RectF {
        RectF::new(
            rect.x - self.left,
            rect.y - self.top,
            rect.width + self.left + self.right,
            rect.height + self.top + self.bottom,
        )
    }
}

/// A refused effect or uniform.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectError {
    /// A value needs depth support.
    Unsupported3d(Unsupported3d),
    /// The chain already has the maximum number of passes.
    TooManyPasses {
        /// The cap.
        max: usize,
        /// The size including the refused pass.
        got: usize,
    },
    /// The shader declares too many uniforms.
    TooManyUniforms {
        /// The cap.
        max: usize,
        /// The size including the refused declaration.
        got: usize,
    },
    /// A uniform name is not a legal identifier.
    InvalidUniformName {
        /// The refused name.
        name: String,
    },
    /// A uniform name uses the reserved prefix.
    ReservedUniform {
        /// The refused name.
        name: String,
    },
    /// A name is declared twice.
    DuplicateUniform {
        /// The repeated name.
        name: String,
    },
    /// A value was set for a name the shader does not declare.
    UnknownUniform {
        /// The undeclared name.
        name: String,
    },
    /// A value's type differs from the declaration.
    WrongUniformType {
        /// The name.
        name: String,
        /// The declared type.
        expected: UniformType,
        /// The supplied type.
        got: UniformType,
    },
    /// A component is NaN or infinite.
    NonFinite {
        /// The value's path.
        field: &'static str,
    },
    /// A value lies outside its accepted range.
    OutOfRange {
        /// The value's path.
        field: &'static str,
        /// The low end.
        min: f32,
        /// The high end.
        max: f32,
    },
}

impl std::fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported3d(error) => write!(formatter, "{error}"),
            Self::TooManyPasses { max, got } => {
                write!(formatter, "an effect chain allows {max} passes, got {got}")
            }
            Self::TooManyUniforms { max, got } => {
                write!(formatter, "a shader allows {max} uniforms, got {got}")
            }
            Self::InvalidUniformName { name } => write!(formatter, "`{name}` is not a uniform name"),
            Self::ReservedUniform { name } => write!(formatter, "`{name}` uses the reserved `gnoblin_` prefix"),
            Self::DuplicateUniform { name } => write!(formatter, "`{name}` is declared twice"),
            Self::UnknownUniform { name } => write!(formatter, "the shader declares no uniform `{name}`"),
            Self::WrongUniformType { name, expected, got } => write!(
                formatter,
                "`{name}` is declared {} and was given {}",
                expected.name(),
                got.name()
            ),
            Self::NonFinite { field } => write!(formatter, "`{field}` is not finite"),
            Self::OutOfRange { field, min, max } => {
                write!(formatter, "`{field}` must be between {min} and {max}")
            }
        }
    }
}

impl std::error::Error for EffectError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> UniformSchema {
        UniformSchema::new(vec![("strength", UniformType::Float), ("tint", UniformType::Vec4)]).expect("valid schema")
    }

    #[test]
    fn a_wrong_uniform_type_is_refused_and_the_value_is_unchanged() {
        let mut shader = Shader::new(schema());
        let error = shader
            .set_uniform("strength", UniformValue::Vec4([0.0; 4]))
            .expect_err("must refuse");
        assert_eq!(
            error,
            EffectError::WrongUniformType {
                name: "strength".to_owned(),
                expected: UniformType::Float,
                got: UniformType::Vec4,
            }
        );
        assert_eq!(shader.uniform("strength"), None);
    }

    #[test]
    fn an_undeclared_uniform_is_refused() {
        let mut shader = Shader::new(schema());
        let error = shader
            .set_uniform("strengh", UniformValue::Float(0.5))
            .expect_err("must refuse");
        assert_eq!(
            error,
            EffectError::UnknownUniform {
                name: "strengh".to_owned()
            }
        );
    }

    #[test]
    fn a_reserved_or_malformed_name_is_refused() {
        assert_eq!(
            UniformSchema::new(vec![("gnoblin_width", UniformType::Float)]).expect_err("reserved"),
            EffectError::ReservedUniform {
                name: "gnoblin_width".to_owned()
            }
        );
        assert!(matches!(
            UniformSchema::new(vec![("1strength", UniformType::Float)]),
            Err(EffectError::InvalidUniformName { .. })
        ));
        assert!(matches!(
            UniformSchema::new(vec![("a", UniformType::Float), ("a", UniformType::Float)]),
            Err(EffectError::DuplicateUniform { .. })
        ));
    }

    #[test]
    fn unset_uniforms_resolve_to_their_type_zero_in_declaration_order() {
        let shader = Shader::new(schema());
        let resolved = shader.resolved();
        assert_eq!(resolved[0].name, "strength");
        assert_eq!(resolved[0].value, UniformValue::Float(0.0));
        assert_eq!(resolved[1].value, UniformValue::Vec4([0.0; 4]));
    }

    #[test]
    fn a_shadow_extends_the_node_bounds_by_its_offset_and_blur() {
        let mut effects = Effects::new();
        effects
            .push(Pass::Shadow(Shadow {
                offset: Vec3::new(4.0, -6.0, 0.0),
                blur: 10.0,
                spread: 0.0,
                opacity: 0.5,
                color: Color::BLACK,
            }))
            .expect("valid");
        let padding = effects.padding();
        assert_eq!(padding.left, 6.0);
        assert_eq!(padding.right, 14.0);
        assert_eq!(padding.top, 16.0);
        assert_eq!(padding.bottom, 4.0);
    }

    #[test]
    fn a_3d_shadow_offset_is_refused_by_name() {
        let mut effects = Effects::new();
        let error = effects
            .push(Pass::Shadow(Shadow {
                offset: Vec3::new(0.0, 0.0, 2.0),
                blur: 0.0,
                spread: 0.0,
                opacity: 1.0,
                color: Color::BLACK,
            }))
            .expect_err("must refuse");
        assert!(matches!(error, EffectError::Unsupported3d(_)));
        assert!(effects.is_empty());
    }

    #[test]
    fn the_pass_cap_refuses_extra_passes_without_changing_the_chain() {
        let mut effects = Effects::new();
        let blur = Pass::Blur(BlurParams {
            radius: 1.0,
            ..BlurParams::default()
        });
        for _ in 0..MAX_PASSES {
            effects.push(blur.clone()).expect("within cap");
        }
        assert!(matches!(effects.push(blur), Err(EffectError::TooManyPasses { .. })));
        assert_eq!(effects.len(), MAX_PASSES);
    }

    #[test]
    fn clamping_brings_every_blur_value_into_its_documented_range() {
        let wild = BlurParams {
            radius: 500.0,
            passes: 0,
            downscale: 0.0,
            tint: Color {
                r: 2.0,
                g: -1.0,
                b: 0.5,
                a: 9.0,
            },
            brightness: 9.0,
            contrast: -3.0,
            saturation: 1.5,
            noise: 4.0,
        };
        let clamped = wild.clamped();
        assert_eq!(clamped.radius, MAX_BLUR_RADIUS);
        assert_eq!(clamped.passes, 1);
        assert_eq!(clamped.downscale, MIN_BLUR_DOWNSCALE);
        assert_eq!(clamped.tint.r, 1.0);
        assert_eq!(clamped.tint.g, 0.0);
        assert_eq!(clamped.tint.b, 0.5);
        assert_eq!(clamped.tint.a, 1.0);
        assert_eq!(clamped.brightness, MAX_BLUR_GRADE);
        assert_eq!(clamped.contrast, 0.0);
        assert_eq!(clamped.saturation, 1.5);
        assert_eq!(clamped.noise, 1.0);
        // A clamped value is accepted by the pass validator.
        assert!(Pass::Blur(clamped).validate().is_ok());
    }

    #[test]
    fn clamping_replaces_a_non_finite_value_with_its_default() {
        let broken = BlurParams {
            radius: f32::NAN,
            downscale: f32::INFINITY,
            ..BlurParams::default()
        };
        assert_eq!(broken.clamped(), BlurParams::default());
    }

    #[test]
    fn a_blur_refuses_an_out_of_range_value_by_name() {
        let mut effects = Effects::new();
        let error = effects
            .push(Pass::Blur(BlurParams {
                passes: 9,
                ..BlurParams::default()
            }))
            .expect_err("must refuse");
        assert!(matches!(
            error,
            EffectError::OutOfRange {
                field: "blur.passes",
                ..
            }
        ));
        assert!(effects.is_empty());
    }

    #[test]
    fn a_positive_shadow_spread_grows_the_extent_and_a_negative_one_does_not() {
        let grown = Shadow {
            offset: Vec3::ZERO,
            blur: 4.0,
            spread: 6.0,
            opacity: 1.0,
            color: Color::BLACK,
        };
        let shrunk = Shadow { spread: -3.0, ..grown };
        assert_eq!(Pass::Shadow(grown).padding().left, 10.0);
        assert_eq!(Pass::Shadow(shrunk).padding().left, 4.0);
    }
}
