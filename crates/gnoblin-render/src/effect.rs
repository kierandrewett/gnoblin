//! Effects: an ordered list of named passes: blur, shadow, border, background, filter and the shell's own shader.
//!
//! The plan gives the renderer effects as GLES passes (Renderer card). This
//! module is the scene-side description of those passes: a pass names what it
//! is, carries the values the effect needs, and is validated where it enters
//! the scene, so the renderer never has to guess. A shader the shell wrote is a pass too
//! ([`ShaderPass`]); the compositor ships blur, shadow (outer or inset), border, background and filter, and the tree
//! refuses any other kind.
//!
//! Order is the pass order: a list is applied in the order it was pushed, which
//! is what makes "blur then recolour" a different description from "recolour
//! then blur".
//!
//! Shadow offsets are [`Vec3`] even though this build only accepts `z == 0`, for
//! the same reason transforms are: a depth value is refused with the named
//! `unsupported-3d` error instead of being silently dropped.

use std::sync::Arc;

use gnoblin_types::{Color, RectF, Unsupported3d, Unsupported3dField, Vec3};

/// The largest number of passes a node may carry.
///
/// Each pass is a render target tap, so an unbounded list is an unbounded frame
/// cost. The cap is a renderer bound, not a preference: a node over it is
/// refused and the caller can simplify the chain.
pub const MAX_PASSES: usize = 32;

/// The most borders, the most shadows and the most inset shadows one node may carry.
///
/// A border is a ring and a shadow a falloff, each drawn once per element and per damaged
/// rectangle, so the count is a frame cost like [`MAX_PASSES`]. The kinds are counted apart (an inset shadow is a kind
/// of its own, [`Pass::name`]), so a style of eight shadows, eight inset shadows and eight borders still fits. The style model of the JS runtime
/// caps its lists at the same number.
pub const MAX_STACKED: usize = 8;

/// The most backgrounds, and the most filters, one node may carry, each counted apart.
///
/// A background is one colour under the content and a filter one grade and blur of everything the node paints. A second of
/// either would only be a different value of the first, so the tree keeps to one of each.
pub const MAX_SINGLE: usize = 1;

/// The most shader passes one node may carry, counted apart from the other kinds and inside [`MAX_PASSES`].
///
/// A shader pass is an arbitrary program run for every pixel of the node, so the count is a frame cost; four is what a layered
/// look needs.
pub const MAX_SHADERS: usize = 4;

/// The longest shader source, in bytes.
pub const MAX_SHADER_SOURCE_BYTES: usize = 16 * 1024;

/// The most numbers a shader pass takes, read in the shader as `p[0..8]`.
pub const MAX_SHADER_PARAMS: usize = 8;

/// The largest blur radius a pass accepts, in logical pixels.
pub const MAX_BLUR_RADIUS: f32 = 100.0;

/// The most blur passes one blur effect may run.
pub const MAX_BLUR_PASSES: u32 = 8;

/// The smallest fraction of full resolution a blur samples its backdrop at.
pub const MIN_BLUR_DOWNSCALE: f32 = 0.125;

/// The largest brightness, contrast or saturation multiplier a blur or a filter accepts.
///
/// The multipliers are plain gains: 1 leaves the picture unchanged, 0 removes the
/// channel, and values above 1 boost it. The bound keeps a gain from blowing the
/// picture out to white in one step.
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

/// A shadow pass: a drop shadow drawn behind the node, or with `inset` a shadow cast inwards from its edge.
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
    /// Whether the shadow follows the node's whole frame rather than its own rectangle.
    ///
    /// A decorated window's frame is its content and the decoration children that sit
    /// outside it, the titlebar above all: a shadow that follows only the content leaves the
    /// bar unshadowed and its top edge floating. The frame is resolved where a layer is built
    /// (it needs the node's children), so a renderer never reads this flag; it reads the
    /// layer's silhouette. An inset shadow always follows the layer's own shape and ignores the flag.
    pub frame: bool,
    /// Whether the shadow lies inside the node's rounded shape instead of outside it.
    ///
    /// An inset shadow is the outer falloff reflected: full strength at the shape's edge, fading inwards over `blur`. `offset`
    /// moves the hole it is cast from and `spread` shrinks that hole on every side. It draws nothing outside the shape, so
    /// it adds no padding. Its [`Pass::name`] is `inset-shadow`.
    pub inset: bool,
}

/// A stroke around the node's silhouette, drawn over its content.
///
/// The silhouette is the node's rounded rectangle, or its frame's when it has decoration
/// children. Several borders stack in list order, each laid where its own `offset` puts it, so
/// two borders with different offsets make concentric rings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// Stroke width in logical pixels, 0 to [`MAX_BLUR_RADIUS`]. A width of 0 draws nothing.
    pub width: f32,
    /// Distance in logical pixels from the silhouette's edge to the stroke's edge nearest it,
    /// from -[`MAX_BLUR_RADIUS`] to [`MAX_BLUR_RADIUS`]. Positive moves the stroke away from the
    /// silhouette (outwards, or inwards when `inset`); negative moves it back across the edge.
    pub offset: f32,
    /// Whether the stroke lies inside the silhouette (`true`) or outside it (`false`).
    pub inset: bool,
    /// Stroke colour, straight alpha.
    pub color: Color,
}

/// A grade and a blur of everything the node paints, the layer's own pixels and not the backdrop behind it.
///
/// This is CSS `filter`: `blur` softens the painted picture and lets it spill past the node's edge, and the three gains
/// grade it in a fixed order, saturation, then brightness, then contrast. A `Pass::Filter` that changes nothing
/// ([`FilterParams::is_identity`]) is allowed and draws like no filter. Every value has a documented range;
/// [`FilterParams::clamped`] brings any value into it and [`Pass::validate`] refuses one outside.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterParams {
    /// Blur radius in logical pixels, 0 to [`MAX_BLUR_RADIUS`]; 0 does not blur. The radius means what a backdrop blur's does:
    /// the Gaussian's sigma is half of it.
    pub blur: f32,
    /// Saturation gain, 0 (grey) to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub saturate: f32,
    /// Brightness gain, 0 to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub brightness: f32,
    /// Contrast gain about mid grey, 0 to [`MAX_BLUR_GRADE`]; 1 is unchanged.
    pub contrast: f32,
}

impl Default for FilterParams {
    /// No blur and no grading.
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl FilterParams {
    /// The filter that changes nothing.
    pub const IDENTITY: Self = Self {
        blur: 0.0,
        saturate: 1.0,
        brightness: 1.0,
        contrast: 1.0,
    };

    /// Whether the filter leaves every pixel as it was.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Whether the filter grades the colours, whatever its blur.
    #[must_use]
    pub fn grades(&self) -> bool {
        self.saturate != 1.0 || self.brightness != 1.0 || self.contrast != 1.0
    }

    /// These parameters with every value brought into its documented range, as [`BlurParams::clamped`] does: a value
    /// out of range takes the nearest bound and a non-finite one takes the identity's.
    #[must_use]
    pub fn clamped(self) -> Self {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        Self {
            blur: finite(self.blur, Self::IDENTITY.blur).clamp(0.0, MAX_BLUR_RADIUS),
            saturate: finite(self.saturate, Self::IDENTITY.saturate).clamp(0.0, MAX_BLUR_GRADE),
            brightness: finite(self.brightness, Self::IDENTITY.brightness).clamp(0.0, MAX_BLUR_GRADE),
            contrast: finite(self.contrast, Self::IDENTITY.contrast).clamp(0.0, MAX_BLUR_GRADE),
        }
    }
}

/// The smallest key that names a shader source: the hash of its text.
///
/// The compiled-shader cache of a renderer is keyed by this (`gnoblin_render_frame::shader::ShaderCache`), so two passes with the
/// same source share one compiled program whatever their params are. It is a 64-bit hash of the text, not a name a script chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShaderKey(u64);

impl ShaderKey {
    /// The key of `source`.
    #[must_use]
    pub fn of(source: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        source.hash(&mut hasher);
        Self(hasher.finish())
    }

    /// The hash, for a log line or a strike list.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A user shader pass: the WGSL the shell wrote, the numbers it reads as `p[0..8]`, and whether it samples the backdrop.
///
/// The scene stores what the wire carried after the compositor checked the source with
/// `gnoblin_render_frame::shader::user::validate` (the validator needs `naga`, which this crate does not link), and `backdrop` is
/// that check's answer. A renderer compiles the source again through the same function, so a pass built some other way is refused
/// there and drawn as skipped; it is never run unchecked.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderPass {
    source: Arc<str>,
    key: ShaderKey,
    params: Vec<f32>,
    backdrop: bool,
}

impl ShaderPass {
    /// A pass of `source` with `params`, which reads the backdrop when `backdrop` is set.
    #[must_use]
    pub fn new(source: &str, params: &[f32], backdrop: bool) -> Self {
        Self {
            key: ShaderKey::of(source),
            source: Arc::from(source),
            params: params.to_vec(),
            backdrop,
        }
    }

    /// The WGSL source, as written.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The source, shared: a frame clones the pass for every damaged output and must not copy 16 KiB each time.
    #[must_use]
    pub fn shared_source(&self) -> &Arc<str> {
        &self.source
    }

    /// The compiled-shader cache key.
    #[must_use]
    pub const fn key(&self) -> ShaderKey {
        self.key
    }

    /// The numbers the shell wrote, at most [`MAX_SHADER_PARAMS`].
    #[must_use]
    pub fn params(&self) -> &[f32] {
        &self.params
    }

    /// The params as the shader reads them, `p[0..8]`: the ones that were not written are zero.
    #[must_use]
    pub fn params_padded(&self) -> [f32; MAX_SHADER_PARAMS] {
        let mut padded = [0.0; MAX_SHADER_PARAMS];
        for (slot, value) in padded.iter_mut().zip(&self.params) {
            *slot = *value;
        }
        padded
    }

    /// Whether the source samples the backdrop, so a renderer has to produce one for the layer
    /// (`gnoblin_render_frame::frame::OutputFrame::backdrop_input`).
    #[must_use]
    pub const fn backdrop(&self) -> bool {
        self.backdrop
    }

    fn validate(&self) -> Result<(), EffectError> {
        if self.source.is_empty() || self.source.len() > MAX_SHADER_SOURCE_BYTES {
            return Err(EffectError::OutOfRange {
                field: "shader.source",
                min: 1.0,
                max: MAX_SHADER_SOURCE_BYTES as f32,
            });
        }
        if self.params.len() > MAX_SHADER_PARAMS {
            return Err(EffectError::OutOfRange {
                field: "shader.params",
                min: 0.0,
                max: MAX_SHADER_PARAMS as f32,
            });
        }
        if self.params.iter().any(|value| !value.is_finite()) {
            return Err(EffectError::NonFinite { field: "shader.params" });
        }
        Ok(())
    }
}

/// One named pass in an effect chain.
#[derive(Clone, Debug, PartialEq)]
pub enum Pass {
    /// A blur.
    Blur(BlurParams),
    /// A drop shadow.
    Shadow(Shadow),
    /// A border.
    Border(Border),
    /// One colour filling the node's rounded shape, below its content and above a backdrop blur. The colour has straight
    /// alpha, so a translucent background tints whatever a blur shows under it.
    Background {
        /// The fill, straight alpha.
        color: Color,
    },
    /// A grade and a blur of everything the node paints.
    Filter(FilterParams),
    /// A shader the shell wrote.
    Shader(ShaderPass),
}

impl Pass {
    /// Every kind of pass, by [`Pass::name`]. A renderer that draws all of them reports exactly this list.
    pub const NAMES: &'static [&'static str] = &[
        "blur",
        "shadow",
        "inset-shadow",
        "border",
        "background",
        "filter",
        "shader",
    ];

    /// The pass's name, as inspection tools report it. An inset shadow is a kind of its own, `inset-shadow`, because a
    /// renderer may draw one without the other; on the wire both are a `shadow` entry and the entry's `inset` tells them apart.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Blur(_) => "blur",
            Self::Shadow(shadow) if shadow.inset => "inset-shadow",
            Self::Shadow(_) => "shadow",
            Self::Border(_) => "border",
            Self::Background { .. } => "background",
            Self::Filter(_) => "filter",
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
            Self::Border(border) => {
                for (field, value) in [("border.width", border.width), ("border.offset", border.offset)] {
                    if !value.is_finite() {
                        return Err(EffectError::NonFinite { field });
                    }
                }
                if !(0.0..=MAX_BLUR_RADIUS).contains(&border.width) {
                    return Err(EffectError::OutOfRange {
                        field: "border.width",
                        min: 0.0,
                        max: MAX_BLUR_RADIUS,
                    });
                }
                if !(-MAX_BLUR_RADIUS..=MAX_BLUR_RADIUS).contains(&border.offset) {
                    return Err(EffectError::OutOfRange {
                        field: "border.offset",
                        min: -MAX_BLUR_RADIUS,
                        max: MAX_BLUR_RADIUS,
                    });
                }
                let color = border.color;
                if ![color.r, color.g, color.b, color.a]
                    .iter()
                    .all(|value| value.is_finite())
                {
                    return Err(EffectError::NonFinite { field: "border.color" });
                }
                if !color.is_in_unit_range() {
                    return Err(EffectError::OutOfRange {
                        field: "border.color",
                        min: 0.0,
                        max: 1.0,
                    });
                }
                Ok(())
            }
            Self::Background { color } => validate_color("background.color", *color),
            Self::Filter(filter) => validate_filter(filter),
            Self::Shader(shader) => shader.validate(),
        }
    }

    /// How far outside the node's bounds the pass can draw.
    ///
    /// A shadow and an outside border draw outside their node; a blur samples within the
    /// node's own rectangle. The value is conservative on purpose: damage that
    /// is slightly too large costs a few pixels, damage that is too small leaves
    /// a stale shadow on screen.
    #[must_use]
    pub fn padding(&self) -> EffectExtent {
        match self {
            // The shadow lies inside the shape: nothing is drawn outside it.
            Self::Shadow(Shadow { inset: true, .. }) => EffectExtent::ZERO,
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
            // An inset shadow, a background and a backdrop blur stay within the node's own shape.
            Self::Blur(_) | Self::Background { .. } | Self::Shader(_) => EffectExtent::ZERO,
            // A filter's blur spills past the edge. The Gaussian's sigma is half the radius, so three sigma is one and a
            // half radii; two is the conservative round-up.
            Self::Filter(filter) => {
                let reach = f64::from(filter.blur.max(0.0)) * 2.0;
                EffectExtent {
                    left: reach,
                    top: reach,
                    right: reach,
                    bottom: reach,
                }
            }
            Self::Border(border) => {
                // The outer edge of the stroke is `offset + width` past the silhouette when it lies
                // outside, and `offset` before it, towards the inside, when it lies inside; a
                // negative `offset` on an inset stroke puts it outside by that much.
                let reach = if border.inset {
                    -border.offset
                } else {
                    border.offset + border.width
                };
                let reach = f64::from(reach.max(0.0));
                EffectExtent {
                    left: reach,
                    top: reach,
                    right: reach,
                    bottom: reach,
                }
            }
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

/// Checks a colour: every channel finite and inside 0 to 1.
fn validate_color(field: &'static str, color: Color) -> Result<(), EffectError> {
    if ![color.r, color.g, color.b, color.a]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(EffectError::NonFinite { field });
    }
    if !color.is_in_unit_range() {
        return Err(EffectError::OutOfRange {
            field,
            min: 0.0,
            max: 1.0,
        });
    }
    Ok(())
}

/// Checks every value of a filter pass before the pass is built.
fn validate_filter(filter: &FilterParams) -> Result<(), EffectError> {
    for (field, value, max) in [
        ("filter.blur", filter.blur, MAX_BLUR_RADIUS),
        ("filter.saturate", filter.saturate, MAX_BLUR_GRADE),
        ("filter.brightness", filter.brightness, MAX_BLUR_GRADE),
        ("filter.contrast", filter.contrast, MAX_BLUR_GRADE),
    ] {
        if !value.is_finite() {
            return Err(EffectError::NonFinite { field });
        }
        if !(0.0..=max).contains(&value) {
            return Err(EffectError::OutOfRange { field, min: 0.0, max });
        }
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
        let kind = pass.name();
        let cap = match pass {
            Pass::Border(_) | Pass::Shadow(_) => Some(MAX_STACKED),
            Pass::Background { .. } | Pass::Filter(_) => Some(MAX_SINGLE),
            Pass::Shader(_) => Some(MAX_SHADERS),
            Pass::Blur(_) => None,
        };
        if let Some(max) = cap {
            let held = self.passes.iter().filter(|held| held.name() == kind).count();
            if held >= max {
                return Err(EffectError::TooManyOfKind {
                    kind,
                    max,
                    got: held + 1,
                });
            }
        }
        self.passes.push(pass);
        Ok(())
    }

    /// The passes, in application order.
    #[must_use]
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// The chain without the shader passes whose key `dropped` accepts, the others in their order. The frame builder uses it to
    /// leave out a pass the frame-cost watchdog has switched off, so the layer is drawn without it.
    #[must_use]
    pub fn without_shaders(&self, mut dropped: impl FnMut(ShaderKey) -> bool) -> Self {
        Self {
            passes: self
                .passes
                .iter()
                .filter(|pass| !matches!(pass, Pass::Shader(shader) if dropped(shader.key())))
                .cloned()
                .collect(),
        }
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

    /// Whether any pass of `kind` (by [`Pass::name`]) is in the chain.
    #[must_use]
    pub fn has(&self, kind: &str) -> bool {
        self.passes.iter().any(|pass| pass.name() == kind)
    }

    /// Whether an outer shadow asks to follow the node's whole frame.
    #[must_use]
    pub fn has_frame_shadow(&self) -> bool {
        self.passes
            .iter()
            .any(|pass| matches!(pass, Pass::Shadow(shadow) if shadow.frame && !shadow.inset))
    }

    /// The chain without its borders, and the borders alone, each in list order.
    ///
    /// A window's borders are drawn over its whole frame, above its decoration children, so they are drawn as a layer
    /// of their own after the subtree; the rest of the chain is drawn with the window's content.
    #[must_use]
    pub fn split_borders(&self) -> (Self, Self) {
        let (borders, rest): (Vec<Pass>, Vec<Pass>) = self
            .passes
            .iter()
            .cloned()
            .partition(|pass| matches!(pass, Pass::Border(_)));
        (Self { passes: rest }, Self { passes: borders })
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

/// A refused effect.
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
    /// The chain already has the maximum number of borders, of shadows or of shader passes.
    TooManyOfKind {
        /// The kind that is over its cap, by [`Pass::name`].
        kind: &'static str,
        /// The cap.
        max: usize,
        /// The count including the refused pass.
        got: usize,
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
            Self::TooManyOfKind { kind, max, got } => {
                write!(formatter, "a node allows {max} {kind} effects, got {got}")
            }
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
                frame: false,
                inset: false,
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
                frame: false,
                inset: false,
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
            frame: false,
            inset: false,
        };
        let shrunk = Shadow { spread: -3.0, ..grown };
        assert_eq!(Pass::Shadow(grown).padding().left, 10.0);
        assert_eq!(Pass::Shadow(shrunk).padding().left, 4.0);
    }

    fn border(width: f32, offset: f32, inset: bool) -> Border {
        Border {
            width,
            offset,
            inset,
            color: Color::WHITE,
        }
    }

    #[test]
    fn an_outside_border_pads_by_its_offset_and_width_and_an_inside_one_does_not() {
        assert_eq!(Pass::Border(border(2.0, 1.0, false)).padding().top, 3.0);
        assert_eq!(Pass::Border(border(2.0, 0.0, true)).padding(), EffectExtent::ZERO);
        assert_eq!(
            Pass::Border(border(2.0, -1.0, true)).padding().left,
            1.0,
            "an inset border with a negative offset crosses the edge"
        );
        assert_eq!(
            Pass::Border(border(2.0, -5.0, false)).padding(),
            EffectExtent::ZERO,
            "an outside border moved back inside draws nothing outside"
        );
    }

    #[test]
    fn a_border_refuses_a_negative_or_non_finite_width_and_a_bad_colour_by_name() {
        for (stroke, field) in [
            (border(-1.0, 0.0, false), "border.width"),
            (border(f32::NAN, 0.0, false), "border.width"),
            (border(1.0, f32::INFINITY, false), "border.offset"),
            (border(1.0, 101.0, false), "border.offset"),
            (border(101.0, 0.0, false), "border.width"),
        ] {
            let error = Pass::Border(stroke).validate().expect_err("must refuse");
            assert!(
                matches!(error, EffectError::NonFinite { field: f } | EffectError::OutOfRange { field: f, .. } if f == field),
                "{field}: {error:?}"
            );
        }
        let mut tinted = border(1.0, 0.0, false);
        tinted.color = Color::new(2.0, 0.0, 0.0, 1.0);
        assert!(matches!(
            Pass::Border(tinted).validate(),
            Err(EffectError::OutOfRange {
                field: "border.color",
                ..
            })
        ));
    }

    #[test]
    fn a_node_holds_eight_borders_and_eight_shadows_and_refuses_a_ninth_of_either() {
        let mut effects = Effects::new();
        for _ in 0..MAX_STACKED {
            effects
                .push(Pass::Border(border(1.0, 0.0, true)))
                .expect("within the cap");
        }
        let refused = effects
            .push(Pass::Border(border(1.0, 0.0, true)))
            .expect_err("over the cap");
        assert_eq!(
            refused,
            EffectError::TooManyOfKind {
                kind: "border",
                max: MAX_STACKED,
                got: MAX_STACKED + 1
            }
        );
        assert_eq!(effects.len(), MAX_STACKED, "a refused pass leaves the chain unchanged");
        let shadow = Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 1.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: true,
            inset: false,
        });
        for _ in 0..MAX_STACKED {
            effects.push(shadow.clone()).expect("the kinds are counted apart");
        }
        assert!(matches!(
            effects.push(shadow),
            Err(EffectError::TooManyOfKind { kind: "shadow", .. })
        ));
    }

    #[test]
    fn every_pass_name_is_listed_in_names() {
        let names: Vec<&str> = [
            Pass::Blur(BlurParams::default()),
            Pass::Border(border(1.0, 0.0, false)),
            Pass::Shadow(Shadow {
                offset: Vec3::ZERO,
                blur: 1.0,
                spread: 0.0,
                opacity: 1.0,
                color: Color::BLACK,
                frame: false,
                inset: false,
            }),
            Pass::Shadow(Shadow {
                inset: true,
                ..shadow_of(1.0)
            }),
            Pass::Background { color: Color::WHITE },
            Pass::Filter(FilterParams::default()),
            Pass::Shader(tint_shader()),
        ]
        .iter()
        .map(Pass::name)
        .collect();
        for name in &names {
            assert!(Pass::NAMES.contains(name), "{name}");
        }
        for name in Pass::NAMES {
            assert!(names.contains(name), "{name} has no pass that carries it");
        }
    }

    fn shadow_of(blur: f32) -> Shadow {
        Shadow {
            offset: Vec3::ZERO,
            blur,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        }
    }

    #[test]
    fn an_inset_shadow_is_a_kind_of_its_own_counted_apart_and_draws_no_padding() {
        let inset = Shadow {
            inset: true,
            blur: 9.0,
            spread: 4.0,
            offset: Vec3::new(3.0, 3.0, 0.0),
            ..shadow_of(9.0)
        };
        assert_eq!(Pass::Shadow(inset).name(), "inset-shadow");
        assert_eq!(Pass::Shadow(shadow_of(9.0)).name(), "shadow");
        assert_eq!(Pass::Shadow(inset).padding(), EffectExtent::ZERO);
        let mut effects = Effects::new();
        for _ in 0..MAX_STACKED {
            effects.push(Pass::Shadow(shadow_of(1.0))).expect("outer shadows");
            effects
                .push(Pass::Shadow(inset))
                .expect("inset shadows are counted apart");
        }
        assert!(matches!(
            effects.push(Pass::Shadow(inset)),
            Err(EffectError::TooManyOfKind {
                kind: "inset-shadow",
                ..
            })
        ));
        assert!(effects.has("inset-shadow") && effects.has("shadow"));
        let framed = Shadow { frame: true, ..inset };
        let mut only_inset = Effects::new();
        only_inset.push(Pass::Shadow(framed)).expect("valid");
        assert!(
            !only_inset.has_frame_shadow(),
            "an inset shadow never follows the frame"
        );
        assert!(
            Pass::Shadow(Shadow {
                spread: f32::NAN,
                ..inset
            })
            .validate()
            .is_err()
        );
    }

    #[test]
    fn a_background_takes_one_colour_and_is_kept_to_one_per_node() {
        let mut effects = Effects::new();
        effects
            .push(Pass::Background {
                color: Color::new(0.1, 0.2, 0.3, 0.5),
            })
            .expect("valid");
        assert_eq!(effects.padding(), EffectExtent::ZERO);
        assert!(matches!(
            effects.push(Pass::Background { color: Color::WHITE }),
            Err(EffectError::TooManyOfKind {
                kind: "background",
                max: 1,
                ..
            })
        ));
        let nan = Pass::Background {
            color: Color::new(f32::NAN, 0.0, 0.0, 1.0),
        };
        assert_eq!(
            nan.validate(),
            Err(EffectError::NonFinite {
                field: "background.color"
            })
        );
        let out = Pass::Background {
            color: Color::new(1.5, 0.0, 0.0, 1.0),
        };
        assert!(matches!(
            out.validate(),
            Err(EffectError::OutOfRange {
                field: "background.color",
                ..
            })
        ));
    }

    #[test]
    fn a_filter_is_validated_by_field_and_its_blur_pads_the_node() {
        assert!(FilterParams::default().is_identity());
        assert!(Pass::Filter(FilterParams::IDENTITY).validate().is_ok());
        for (field, filter) in [
            (
                "filter.blur",
                FilterParams {
                    blur: MAX_BLUR_RADIUS + 1.0,
                    ..FilterParams::IDENTITY
                },
            ),
            (
                "filter.saturate",
                FilterParams {
                    saturate: -0.1,
                    ..FilterParams::IDENTITY
                },
            ),
            (
                "filter.brightness",
                FilterParams {
                    brightness: MAX_BLUR_GRADE + 0.5,
                    ..FilterParams::IDENTITY
                },
            ),
            (
                "filter.contrast",
                FilterParams {
                    contrast: 9.0,
                    ..FilterParams::IDENTITY
                },
            ),
        ] {
            assert!(
                matches!(Pass::Filter(filter).validate(), Err(EffectError::OutOfRange { field: got, .. }) if got == field),
                "{field}"
            );
        }
        assert_eq!(
            Pass::Filter(FilterParams {
                contrast: f32::INFINITY,
                ..FilterParams::IDENTITY
            })
            .validate(),
            Err(EffectError::NonFinite {
                field: "filter.contrast"
            })
        );
        let blurred = Pass::Filter(FilterParams {
            blur: 10.0,
            ..FilterParams::IDENTITY
        });
        assert_eq!(blurred.padding().left, 20.0, "a blur spills past the edge by two radii");
        assert_eq!(blurred.padding().bottom, 20.0);
        let graded = FilterParams {
            saturate: 0.0,
            ..FilterParams::IDENTITY
        };
        assert!(graded.grades() && !graded.is_identity());
        assert_eq!(
            Pass::Filter(graded).padding(),
            EffectExtent::ZERO,
            "a grade stays inside the node"
        );
        let wild = FilterParams {
            blur: f32::NAN,
            saturate: 7.0,
            brightness: -1.0,
            contrast: f32::INFINITY,
        }
        .clamped();
        assert_eq!(
            wild,
            FilterParams {
                blur: 0.0,
                saturate: MAX_BLUR_GRADE,
                brightness: 0.0,
                contrast: 1.0
            }
        );
        let mut effects = Effects::new();
        effects.push(blurred).expect("valid");
        assert!(matches!(
            effects.push(Pass::Filter(graded)),
            Err(EffectError::TooManyOfKind {
                kind: "filter",
                max: 1,
                ..
            })
        ));
    }

    fn tint_shader() -> ShaderPass {
        ShaderPass::new(
            "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c; }",
            &[0.5, 2.0],
            false,
        )
    }

    #[test]
    fn a_shader_pass_keeps_its_source_params_and_a_key_that_follows_the_source() {
        let pass = tint_shader();
        assert_eq!(pass.params(), &[0.5, 2.0]);
        assert_eq!(pass.params_padded(), [0.5, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(pass.key(), ShaderKey::of(pass.source()));
        let other = ShaderPass::new("fn effect() {}", &[0.5, 2.0], false);
        assert_ne!(pass.key(), other.key(), "another source is another key");
        let reparamed = ShaderPass::new(pass.source(), &[9.0], true);
        assert_eq!(
            pass.key(),
            reparamed.key(),
            "params and the backdrop flag are not part of the key"
        );
        assert_eq!(Pass::Shader(pass).padding(), EffectExtent::ZERO);
    }

    #[test]
    fn a_chain_without_some_shaders_keeps_every_other_pass_in_order() {
        let kept = tint_shader();
        let dropped = ShaderPass::new("fn effect() {}", &[], false);
        let dropped_key = dropped.key();
        let mut chain = Effects::new();
        chain.push(Pass::Border(border(1.0, 0.0, false))).unwrap();
        chain.push(Pass::Shader(dropped.clone())).unwrap();
        chain.push(Pass::Shader(kept.clone())).unwrap();
        chain.push(Pass::Shader(dropped)).unwrap();
        let without = chain.without_shaders(|key| key == dropped_key);
        assert_eq!(
            without.passes().iter().map(Pass::name).collect::<Vec<_>>(),
            ["border", "shader"]
        );
        assert!(matches!(&without.passes()[1], Pass::Shader(shader) if *shader == kept));
        assert_eq!(
            chain.without_shaders(|_| false),
            chain,
            "nothing dropped, nothing changed"
        );
    }

    #[test]
    fn a_shader_pass_refuses_a_source_or_params_outside_its_bounds() {
        assert!(Pass::Shader(tint_shader()).validate().is_ok());
        let empty = Pass::Shader(ShaderPass::new("", &[], false));
        assert!(matches!(
            empty.validate(),
            Err(EffectError::OutOfRange {
                field: "shader.source",
                ..
            })
        ));
        let big = Pass::Shader(ShaderPass::new(&"a".repeat(MAX_SHADER_SOURCE_BYTES + 1), &[], false));
        assert!(matches!(
            big.validate(),
            Err(EffectError::OutOfRange {
                field: "shader.source",
                ..
            })
        ));
        let long = Pass::Shader(ShaderPass::new("x", &[0.0; MAX_SHADER_PARAMS + 1], false));
        assert!(matches!(
            long.validate(),
            Err(EffectError::OutOfRange {
                field: "shader.params",
                ..
            })
        ));
        let nan = Pass::Shader(ShaderPass::new("x", &[f32::NAN], false));
        assert!(matches!(
            nan.validate(),
            Err(EffectError::NonFinite { field: "shader.params" })
        ));
    }

    #[test]
    fn a_node_holds_four_shader_passes_and_refuses_a_fifth() {
        let mut effects = Effects::new();
        for _ in 0..MAX_SHADERS {
            effects.push(Pass::Shader(tint_shader())).expect("within the cap");
        }
        assert!(matches!(
            effects.push(Pass::Shader(tint_shader())),
            Err(EffectError::TooManyOfKind {
                kind: "shader",
                max: MAX_SHADERS,
                got: 5
            })
        ));
        assert_eq!(effects.len(), MAX_SHADERS);
        effects
            .push(Pass::Blur(BlurParams::default()))
            .expect("the cap counts shaders only");
    }
}
