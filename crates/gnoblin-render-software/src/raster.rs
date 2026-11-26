//! The CPU twins of the shaders that draw a shadow, a border and a rounded corner.
//!
//! Pixman composites rectangles and has no anti-aliased mask, so the soft edges the GPU renderers draw
//! (`wgsl/shadow.wgsl`, `wgsl/inset_shadow.wgsl`, `wgsl/border.wgsl`, `gnoblin_rounded_box`) are computed here, one pixel
//! at a time, on a [`Patch`]: a rectangle of the output read back from the buffer, changed in memory and written over
//! the damaged pixels it came from. Every function evaluates the shader's own expression at the pixel centre, so a
//! shadow, a border and a corner fall off exactly as they do on the GPU, within the rounding of an 8-bit buffer.
//!
//! Everything here is pure arithmetic on bytes: nothing in this module touches Pixman, which keeps it testable without
//! a buffer and keeps the one place that reads and writes the output (`layer.rs`) small.

use smithay::utils::{Physical, Rectangle};

use gnoblin_render::{Border, Shadow};
use gnoblin_types::{Color, Scale};

/// A rectangle of the output, in physical pixels.
pub(crate) type Area = Rectangle<i32, Physical>;

/// The largest corner radius or blur radius a pass draws, in physical pixels. The GLES renderer's bound.
pub(crate) const MAX_RADIUS: f32 = 4096.0;

/// A rectangle of the output as premultiplied RGBA8 rows, packed (`width * 4` bytes a row), top row first.
pub(crate) struct Patch {
    /// Where the rectangle sits on the output.
    pub(crate) rect: Area,
    /// The pixels, in the byte order of `READBACK_FORMAT`.
    pub(crate) bytes: Vec<u8>,
}

impl Patch {
    /// The index of the first byte of the pixel at (`x`, `y`) of the output, which must lie inside the patch.
    fn at(&self, x: i32, y: i32) -> usize {
        ((y - self.rect.loc.y) as usize * self.rect.size.w as usize + (x - self.rect.loc.x) as usize) * 4
    }

    /// The pixel at (`x`, `y`) of the output as premultiplied floats in 0..1.
    fn load(&self, x: i32, y: i32) -> [f32; 4] {
        let at = self.at(x, y);
        let pixel = &self.bytes[at..at + 4];
        [
            f32::from(pixel[0]) / 255.0,
            f32::from(pixel[1]) / 255.0,
            f32::from(pixel[2]) / 255.0,
            f32::from(pixel[3]) / 255.0,
        ]
    }

    /// Stores premultiplied floats in 0..1 as the pixel at (`x`, `y`) of the output.
    fn store(&mut self, x: i32, y: i32, value: [f32; 4]) {
        let at = self.at(x, y);
        for (byte, channel) in self.bytes[at..at + 4].iter_mut().zip(value) {
            *byte = to_byte(channel);
        }
    }

    /// `colour` composited over the pixel at (`x`, `y`): the blend every pass draws with, `ONE, ONE_MINUS_SRC_ALPHA`.
    fn over(&mut self, x: i32, y: i32, colour: [f32; 4]) {
        if colour[3] <= 0.0 {
            return;
        }
        let at = self.at(x, y);
        Self::blend(&mut self.bytes[at..at + 4], Blend::new(colour));
    }

    /// Composites `blend` over one pixel of four bytes.
    fn blend(pixel: &mut [u8], blend: Blend) {
        for (byte, source) in pixel.iter_mut().zip(blend.source) {
            // `x / 255` rounded, for x up to 255 * 255, as two shifts: exact for every value in range.
            let kept = u32::from(*byte) * blend.keep + 128;
            let kept = (kept + (kept >> 8)) >> 8;
            *byte = (kept + source).min(255) as u8;
        }
    }
}

/// A premultiplied colour ready to composite over bytes: its own bytes and how much of what is underneath it stays,
/// out of 255.
#[derive(Clone, Copy)]
struct Blend {
    source: [u32; 4],
    keep: u32,
}

impl Blend {
    fn new(colour: [f32; 4]) -> Self {
        Self {
            source: colour.map(|channel| u32::from(to_byte(channel))),
            keep: 255 - u32::from(to_byte(colour[3])),
        }
    }
}

/// A float channel in 0..1 as the nearest byte.
pub(crate) fn to_byte(channel: f32) -> u8 {
    (channel * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// `smoothstep(edge0, edge1, x)` as WGSL defines it.
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The signed distance from a point to the boundary of a rounded box of `width` by `height` whose corner is the origin,
/// negative inside: `gnoblin_sdf`. The radius is clamped so one larger than half the shorter side cannot invert the box.
fn sdf(x: f32, y: f32, width: f32, height: f32, radius: f32) -> f32 {
    let (half_x, half_y) = (width * 0.5, height * 0.5);
    let corner = radius.clamp(0.0, half_x.min(half_y).max(0.0));
    let offset_x = (x - half_x).abs() - (half_x - corner);
    let offset_y = (y - half_y).abs() - (half_y - corner);
    offset_x.max(0.0).hypot(offset_y.max(0.0)) + offset_x.max(offset_y).min(0.0) - corner
}

/// The coverage of a pixel at signed distance `distance` from an edge: 1 inside, 0 outside, a smooth step over the one
/// pixel that straddles it. This is what anti-aliases a corner.
fn coverage(distance: f32) -> f32 {
    1.0 - smoothstep(-0.5, 0.5, distance)
}

/// A rounded rectangle on the output, the shape a layer's corners, borders and shadows follow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
}

impl Shape {
    /// The rounded rectangle `rect` with corner `radius`, both physical.
    pub(crate) fn new(rect: Area, radius: f32) -> Self {
        Self {
            x: rect.loc.x as f32,
            y: rect.loc.y as f32,
            width: rect.size.w.max(0) as f32,
            height: rect.size.h.max(0) as f32,
            radius,
        }
    }

    /// The signed distance of the centre of pixel (`x`, `y`) to the shape's edge.
    fn distance(&self, x: i32, y: i32) -> f32 {
        sdf(
            x as f32 + 0.5 - self.x,
            y as f32 + 0.5 - self.y,
            self.width,
            self.height,
            self.radius,
        )
    }

    /// The share of pixel (`x`, `y`) inside the shape.
    pub(crate) fn coverage(&self, x: i32, y: i32) -> f32 {
        coverage(self.distance(x, y))
    }

    /// The corner radius the distance uses: clamped so it cannot invert the box.
    fn corner(&self) -> f32 {
        self.radius
            .clamp(0.0, (self.width * 0.5).min(self.height * 0.5).max(0.0))
    }

    /// For each pixel column of `area` and each pixel row, how far the pixel's centre is beyond the box's shrunk
    /// rectangle along that axis (`gnoblin_sdf`'s `offset`). The distance of a pixel is a function of its two numbers,
    /// so a loop over an area works the two arrays out once instead of once a pixel.
    fn axes(&self, area: Area) -> (Vec<f32>, Vec<f32>) {
        let corner = self.corner();
        let axis = |from: i32, count: i32, origin: f32, size: f32| -> Vec<f32> {
            (from..from + count)
                .map(|at| (at as f32 + 0.5 - origin - size * 0.5).abs() - (size * 0.5 - corner))
                .collect()
        };
        (
            axis(area.loc.x, area.size.w, self.x, self.width),
            axis(area.loc.y, area.size.h, self.y, self.height),
        )
    }

    /// The signed distance of a pixel whose offsets along the axes are `across` and `down` (see [`Shape::axes`]).
    fn distance_at(&self, across: f32, down: f32) -> f32 {
        let beyond = if across > 0.0 && down > 0.0 {
            across.hypot(down)
        } else {
            across.max(down)
        };
        beyond - self.corner()
    }
}

/// `rect` grown by `by` pixels on every side; a negative `by` shrinks it. A rectangle that shrinks away has no area.
pub(crate) fn grown(rect: Area, by: i32) -> Area {
    Rectangle::new(
        (rect.loc.x - by, rect.loc.y - by).into(),
        ((rect.size.w + 2 * by).max(0), (rect.size.h + 2 * by).max(0)).into(),
    )
}

/// A straight colour as premultiplied floats, with `alpha` already folded in.
fn premultiplied(colour: Color, alpha: f32) -> [f32; 4] {
    [colour.r * alpha, colour.g * alpha, colour.b * alpha, alpha]
}

/// A shadow cast by a shape: the drop shadow outside it, or the inset one inside it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cast {
    /// Where the shadow is drawn: the pixels outside it are left alone.
    pub(crate) quad: Area,
    /// The straight colour and the alpha the shadow starts from at full strength.
    colour: Color,
    alpha: f32,
    blur: f32,
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    /// The shape the shadow is cast from: the layer's, moved and grown, with the layer's own corner radius.
    Outer(Shape),
    /// The hole the shadow is cast from, relative to the layer's shape, which is the quad.
    Inset {
        offset: [f32; 2],
        spread: f32,
        shape: Shape,
    },
}

impl Cast {
    /// The drop shadow `held` casts from `base`, the rectangle it follows, with corner `radius`.
    ///
    /// The geometry is the GLES renderer's: the quad is `base` moved by the offset and grown by the spread and by the
    /// blur rounded to a pixel, and `shadow.wgsl` finds the shape by insetting that quad by the unrounded blur, so the
    /// shape is the quad's inner rectangle and not exactly `base` grown by the spread.
    ///
    /// `fade` is the layer's opacity, which the shadow takes as CSS's `box-shadow` does: a half-transparent window casts a
    /// half-strength shadow.
    pub(crate) fn outer(held: &Shadow, base: Area, radius: f32, fade: f32, scale: Scale) -> Self {
        let factor = scale.as_f64() as f32;
        let blur = (held.blur * factor).clamp(1.0, MAX_RADIUS);
        let spread = (held.spread * factor).round() as i32;
        let grow = blur.round() as i32 + spread;
        let quad = grown(
            Rectangle::new(
                (
                    base.loc.x + (held.offset.x as f32 * factor).round() as i32,
                    base.loc.y + (held.offset.y as f32 * factor).round() as i32,
                )
                    .into(),
                base.size,
            ),
            grow,
        );
        let shape = Shape {
            x: quad.loc.x as f32 + blur,
            y: quad.loc.y as f32 + blur,
            width: (quad.size.w as f32 - blur * 2.0).max(1.0),
            height: (quad.size.h as f32 - blur * 2.0).max(1.0),
            radius,
        };
        Self {
            quad,
            colour: held.color,
            alpha: held.color.a * held.opacity * fade,
            blur,
            kind: Kind::Outer(shape),
        }
    }

    /// The inset shadow `held` casts inwards from the edge of `base`, with corner `radius` (`inset_shadow.wgsl`).
    ///
    /// The hole is `base` moved by the offset and shrunk by the spread on every side. A pixel on or outside the hole's
    /// edge has the full shadow, which falls to nothing `blur` pixels inside the edge, and the shadow is multiplied by
    /// the shape's own coverage so nothing leaks past a rounded corner.
    ///
    /// `fade` is the share of the layer's own opacity the shadow is drawn with: an inset shadow is part of the layer, so
    /// it fades with it, where a drop shadow behind the layer does not.
    pub(crate) fn inset(held: &Shadow, base: Area, radius: f32, fade: f32, scale: Scale) -> Self {
        let factor = scale.as_f64() as f32;
        Self {
            quad: base,
            colour: held.color,
            alpha: held.color.a * held.opacity * fade,
            blur: (held.blur * factor).clamp(1.0, MAX_RADIUS),
            kind: Kind::Inset {
                offset: [held.offset.x as f32 * factor, held.offset.y as f32 * factor],
                spread: held.spread * factor,
                shape: Shape::new(base, radius),
            },
        }
    }

    /// The share of the shadow's colour at the centre of pixel (`x`, `y`).
    fn strength(&self, x: i32, y: i32) -> f32 {
        match self.kind {
            Kind::Outer(shape) => 1.0 - smoothstep(0.0, self.blur, shape.distance(x, y)),
            Kind::Inset { offset, spread, shape } => {
                let point = (x as f32 + 0.5 - shape.x, y as f32 + 0.5 - shape.y);
                let hole = (
                    (shape.width - spread * 2.0).max(1.0),
                    (shape.height - spread * 2.0).max(1.0),
                );
                let distance = sdf(
                    point.0 - offset[0] - spread,
                    point.1 - offset[1] - spread,
                    hole.0,
                    hole.1,
                    (shape.radius - spread).max(0.0),
                );
                smoothstep(-self.blur, 0.0, distance) * shape.coverage(x, y)
            }
        }
    }

    /// Draws the shadow over the pixels of `patch` inside the quad.
    pub(crate) fn draw(&self, patch: &mut Patch) {
        let Some(area) = patch.rect.intersection(self.quad) else {
            return;
        };
        match self.kind {
            Kind::Outer(shape) => {
                // Most pixels of a shadow are whole shadow or none, so they take the two cheap branches.
                let (across, down) = shape.axes(area);
                let whole = premultiplied(self.colour, self.alpha);
                let whole_blend = Blend::new(whole);
                for (row, y) in (area.loc.y..area.loc.y + area.size.h).enumerate() {
                    for (column, x) in (area.loc.x..area.loc.x + area.size.w).enumerate() {
                        let distance = shape.distance_at(across[column], down[row]);
                        if distance >= self.blur {
                            continue;
                        }
                        if distance <= 0.0 {
                            let at = patch.at(x, y);
                            Patch::blend(&mut patch.bytes[at..at + 4], whole_blend);
                        } else {
                            let strength = 1.0 - smoothstep(0.0, self.blur, distance);
                            patch.over(x, y, premultiplied(self.colour, self.alpha * strength));
                        }
                    }
                }
            }
            Kind::Inset { .. } => {
                for y in area.loc.y..area.loc.y + area.size.h {
                    for x in area.loc.x..area.loc.x + area.size.w {
                        let alpha = self.alpha * self.strength(x, y);
                        patch.over(x, y, premultiplied(self.colour, alpha));
                    }
                }
            }
        }
    }
}

/// Fills `rect` of `patch` with `colour` at `alpha`, over what is there: a layer's `background`.
///
/// The fill is square. A rounded layer is drawn square and faded back outside its curve afterwards ([`fade`]), which
/// gives the fill the same anti-aliased corner as every other part of the layer.
pub(crate) fn fill(patch: &mut Patch, rect: Area, colour: Color, alpha: f32) {
    let Some(area) = patch.rect.intersection(rect) else {
        return;
    };
    let blend = Blend::new(premultiplied(colour, alpha));
    if blend.source[3] == 0 {
        return;
    }
    for y in area.loc.y..area.loc.y + area.size.h {
        let at = patch.at(area.loc.x, y);
        for pixel in patch.bytes[at..at + area.size.w as usize * 4].as_chunks_mut::<4>().0 {
            Patch::blend(pixel, blend);
        }
    }
}

/// A border: a ring along the edge of a shape, drawn over the layer's own pixels (`border.wgsl`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stroke {
    shape: Shape,
    /// The ring is the band of points whose signed distance to the shape lies from `near` to `far`: positive outside
    /// the shape, negative inside it.
    near: f32,
    far: f32,
    colour: Color,
    /// The alpha the ring starts from at full coverage: the colour's own, the layer's tint and the layer's opacity.
    alpha: f32,
    /// Where the ring is drawn: the shape grown by how far it reaches outside it, and one pixel more for the fringe.
    pub(crate) quad: Area,
}

impl Stroke {
    /// The ring `held` draws along `outline` with corner `radius`, under the layer's `tint` and `opacity`.
    pub(crate) fn new(held: &Border, outline: Area, radius: f32, tint: Color, opacity: f32, scale: Scale) -> Self {
        let factor = scale.as_f64() as f32;
        let width = held.width * factor;
        let offset = held.offset * factor;
        let (near, far) = if held.inset {
            (-(offset + width), -offset)
        } else {
            (offset, offset + width)
        };
        let reach = if held.inset { -offset } else { offset + width };
        let grow = reach.max(0.0).ceil() as i32 + 1;
        Self {
            shape: Shape::new(outline, radius),
            near,
            far,
            colour: Color::new(
                held.color.r * tint.r,
                held.color.g * tint.g,
                held.color.b * tint.b,
                held.color.a * tint.a,
            ),
            alpha: held.color.a * tint.a * opacity,
            quad: grown(outline, grow),
        }
    }

    /// How far from the outline's sides a pixel can be and still be changed by the ring, in pixels: the band's reach, a
    /// pixel for each of its anti-aliased edges, and the corner's radius, inside which the distance is not the plain
    /// distance to a side.
    pub(crate) fn reach(&self) -> i32 {
        self.near.abs().max(self.far.abs()).ceil() as i32 + 2 + self.shape.corner().ceil() as i32
    }

    /// Draws the ring over the pixels of `patch` inside the quad. Each edge of the band is anti-aliased over one pixel, so
    /// a stroke thinner than a pixel is fainter rather than aliased.
    ///
    /// Only the pixels near the outline are looked at: a pixel further from every side than the band's reach, and than
    /// the corner's radius, has a distance that is the plain distance to the nearest side, which is outside the band.
    pub(crate) fn draw(&self, patch: &mut Patch) {
        let Some(area) = patch.rect.intersection(self.quad) else {
            return;
        };
        let reach = self.reach();
        let (left, right) = (self.shape.x as i32, (self.shape.x + self.shape.width) as i32);
        let (top, bottom) = (self.shape.y as i32, (self.shape.y + self.shape.height) as i32);
        let (across, down) = self.shape.axes(area);
        for (row, y) in (area.loc.y..area.loc.y + area.size.h).enumerate() {
            let on_edge = (y - top).abs() <= reach || (y - bottom).abs() <= reach;
            for (column, x) in (area.loc.x..area.loc.x + area.size.w).enumerate() {
                if !on_edge && (x - left).abs() > reach && (x - right).abs() > reach {
                    continue;
                }
                let distance = self.shape.distance_at(across[column], down[row]);
                let within = (0.5 - (distance - self.far)).clamp(0.0, 1.0);
                let beyond = (0.5 + (distance - self.near)).clamp(0.0, 1.0);
                patch.over(x, y, premultiplied(self.colour, self.alpha * within * beyond));
            }
        }
    }
}

/// The rectangles of `quad` within `reach` pixels of a side of `outline`, which together hold everything a ring along
/// the outline changes. They do not overlap, so a ring is read, drawn and written once a pixel; a quad too small to
/// have a middle is one rectangle.
pub(crate) fn ring(quad: Area, outline: Area, reach: i32) -> Vec<Area> {
    let (left, top) = (outline.loc.x, outline.loc.y);
    let (right, bottom) = (left + outline.size.w, top + outline.size.h);
    let rows = |from: i32, to: i32, across: (i32, i32)| {
        let rect = Rectangle::new((across.0, from).into(), (across.1 - across.0, to - from).into());
        rect.intersection(quad).filter(|rect| !rect.is_empty())
    };
    let whole = (quad.loc.x, quad.loc.x + quad.size.w);
    let (quad_top, quad_bottom) = (quad.loc.y, quad.loc.y + quad.size.h);
    let (top_end, bottom_start) = ((top + reach).max(quad_top), (bottom - reach).min(quad_bottom));
    if top_end >= bottom_start {
        return rows(quad_top, quad_bottom, whole).into_iter().collect();
    }
    let top_strip = rows(quad_top, top_end, whole);
    let bottom_strip = rows(bottom_start, quad_bottom, whole);
    let sides = if left + reach >= right - reach {
        vec![rows(top_end, bottom_start, whole)]
    } else {
        vec![
            rows(top_end, bottom_start, (quad.loc.x, left + reach)),
            rows(top_end, bottom_start, (right - reach, quad.loc.x + quad.size.w)),
        ]
    };
    [top_strip, bottom_strip].into_iter().chain(sides).flatten().collect()
}

/// Fades `drawn` back towards `before`, the same rectangle as it was before the layer drew into it, by the layer's
/// `fade` (its group opacity) and by the shape's coverage: `before + (drawn - before) * fade * coverage`.
///
/// A rounded layer is drawn square and this puts the pixels outside its curve back, with the edge anti-aliased; a
/// layer with a backdrop blur is drawn as one group and this fades the group over the frame by the layer's opacity.
pub(crate) fn fade(drawn: &mut Patch, before: &Patch, shape: Shape, fade: f32) {
    debug_assert_eq!(drawn.rect, before.rect, "the pixels before and after are one rectangle");
    let rect = drawn.rect;
    for y in rect.loc.y..rect.loc.y + rect.size.h {
        for x in rect.loc.x..rect.loc.x + rect.size.w {
            let share = fade * shape.coverage(x, y);
            if share >= 1.0 {
                continue;
            }
            let (old, new) = (before.load(x, y), drawn.load(x, y));
            drawn.store(x, y, [0, 1, 2, 3].map(|i| old[i] + (new[i] - old[i]) * share));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_types::Vec3;

    fn area(x: i32, y: i32, w: i32, h: i32) -> Area {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn patch(rect: Area, pixel: [u8; 4]) -> Patch {
        let bytes = pixel.repeat((rect.size.w * rect.size.h) as usize);
        Patch { rect, bytes }
    }

    fn pixel(patch: &Patch, x: i32, y: i32) -> [u8; 4] {
        let at = patch.at(x, y);
        [
            patch.bytes[at],
            patch.bytes[at + 1],
            patch.bytes[at + 2],
            patch.bytes[at + 3],
        ]
    }

    /// The one place a test builds a shadow, so a field the pass gains is added once.
    fn shadow(blur: f32, spread: f32, offset: (f32, f32)) -> Shadow {
        Shadow {
            offset: Vec3::new(f64::from(offset.0), f64::from(offset.1), 0.0),
            blur,
            spread,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        }
    }

    #[test]
    fn a_rounded_box_has_the_distance_the_shader_computes() {
        // The edge of a square box, the middle of a side and a corner pixel of a rounded one.
        assert_eq!(sdf(0.0, 5.0, 10.0, 10.0, 0.0), 0.0);
        assert_eq!(sdf(5.0, 5.0, 10.0, 10.0, 0.0), -5.0);
        assert_eq!(sdf(-3.0, 5.0, 10.0, 10.0, 0.0), 3.0);
        let corner = sdf(0.5, 0.5, 10.0, 10.0, 4.0);
        assert!(
            corner > 0.5,
            "the corner pixel of a rounded box is outside the arc: {corner}"
        );
        assert_eq!(
            sdf(5.0, 0.5, 10.0, 10.0, 100.0),
            -0.5,
            "an oversized radius is a circle of radius 5"
        );
    }

    #[test]
    fn a_shadow_falls_off_with_the_shaders_smoothstep() {
        let mut picture = patch(area(0, 0, 48, 48), [255, 255, 255, 255]);
        // A 16 pixel box at (16, 16), a blur of 8 and no offset.
        let cast = Cast::outer(
            &shadow(8.0, 0.0, (0.0, 0.0)),
            area(16, 16, 16, 16),
            0.0,
            1.0,
            Scale::IDENTITY,
        );
        cast.draw(&mut picture);
        // Inside the shape the shadow is whole; `x` pixels beyond the left edge it is `1 - smoothstep(0, 8, x)`.
        assert_eq!(pixel(&picture, 24, 24), [0, 0, 0, 255], "full strength inside");
        assert_eq!(pixel(&picture, 16, 24), [0, 0, 0, 255], "and at the edge");
        for beyond in 1..8 {
            let want = (smoothstep(0.0, 8.0, beyond as f32 - 0.5) * 255.0).round() as u8;
            let got = pixel(&picture, 16 - beyond, 24)[0];
            assert!(
                got.abs_diff(want) <= 1,
                "{beyond} pixels beyond the edge: drew {got}, the falloff is {want}"
            );
        }
        assert_eq!(pixel(&picture, 7, 24), [255, 255, 255, 255], "nothing past the blur");
    }

    #[test]
    fn a_shadow_is_smooth_not_stepped() {
        let mut picture = patch(area(0, 0, 64, 8), [255, 255, 255, 255]);
        let cast = Cast::outer(
            &shadow(24.0, 0.0, (0.0, 0.0)),
            area(32, 0, 32, 8),
            0.0,
            1.0,
            Scale::IDENTITY,
        );
        cast.draw(&mut picture);
        let row: Vec<u8> = (8..32).map(|x| pixel(&picture, x, 4)[0]).collect();
        assert!(
            row.windows(2).all(|pair| pair[1] <= pair[0]),
            "the falloff only brightens away from the box: {row:?}"
        );
        let distinct = row.iter().collect::<std::collections::BTreeSet<_>>().len();
        assert!(
            distinct >= 20,
            "24 pixels of falloff make {distinct} distinct levels: {row:?}"
        );
    }

    #[test]
    fn a_shadow_takes_the_layers_opacity() {
        let drawn = |fade: f32| {
            let mut picture = patch(area(0, 0, 48, 48), [255, 255, 255, 255]);
            Cast::outer(
                &shadow(8.0, 0.0, (0.0, 0.0)),
                area(16, 16, 16, 16),
                0.0,
                fade,
                Scale::IDENTITY,
            )
            .draw(&mut picture);
            pixel(&picture, 12, 24)[0]
        };
        let (whole, half) = (255 - drawn(1.0), 255 - drawn(0.5));
        assert!(whole > 40, "the shadow is there at full opacity: {whole}");
        assert!(
            (i32::from(whole) - 2 * i32::from(half)).abs() <= 2,
            "half the opacity, half the shadow: {whole} against {half}"
        );
    }

    #[test]
    fn a_shadow_offset_and_spread_move_and_grow_its_shape() {
        let mut picture = patch(area(0, 0, 48, 48), [255, 255, 255, 255]);
        let cast = Cast::outer(
            &shadow(1.0, 2.0, (4.0, 0.0)),
            area(16, 16, 16, 16),
            0.0,
            1.0,
            Scale::IDENTITY,
        );
        cast.draw(&mut picture);
        // The shape runs from x = 16 + 4 - 2 to 32 + 4 + 2.
        assert_eq!(pixel(&picture, 18, 24), [0, 0, 0, 255]);
        assert_eq!(pixel(&picture, 37, 24), [0, 0, 0, 255]);
        assert_eq!(pixel(&picture, 16, 24), [255, 255, 255, 255]);
        assert_eq!(pixel(&picture, 39, 24), [255, 255, 255, 255]);
    }

    #[test]
    fn an_inset_shadow_is_full_at_the_edge_and_gone_blur_pixels_inside() {
        let mut picture = patch(area(0, 0, 32, 32), [255, 255, 255, 255]);
        let cast = Cast::inset(
            &shadow(6.0, 0.0, (0.0, 0.0)),
            area(8, 8, 16, 16),
            0.0,
            1.0,
            Scale::IDENTITY,
        );
        cast.draw(&mut picture);
        assert!(
            pixel(&picture, 8, 16)[0] < 10,
            "the edge's pixel is all but whole shadow"
        );
        assert!(
            pixel(&picture, 11, 16)[0] > 60 && pixel(&picture, 11, 16)[0] < 200,
            "partway in it fades: {:?}",
            pixel(&picture, 11, 16)
        );
        assert_eq!(
            pixel(&picture, 16, 16),
            [255, 255, 255, 255],
            "past `blur` pixels the shape is untouched"
        );
        assert_eq!(
            pixel(&picture, 7, 16),
            [255, 255, 255, 255],
            "nothing leaks past the edge"
        );
    }

    #[test]
    fn an_inset_shadow_does_not_leak_past_a_rounded_corner() {
        let mut picture = patch(area(0, 0, 32, 32), [255, 255, 255, 255]);
        let cast = Cast::inset(
            &shadow(4.0, 0.0, (0.0, 0.0)),
            area(4, 4, 24, 24),
            10.0,
            1.0,
            Scale::IDENTITY,
        );
        cast.draw(&mut picture);
        assert_eq!(
            pixel(&picture, 4, 4),
            [255, 255, 255, 255],
            "the square corner is outside the shape"
        );
        assert!(pixel(&picture, 16, 4)[0] < 40, "the edge's middle is dark");
    }

    #[test]
    fn an_inset_shadows_offset_moves_the_hole_and_its_spread_shrinks_it() {
        let shift = |offset: (f32, f32), spread: f32| {
            let mut picture = patch(area(0, 0, 32, 32), [255, 255, 255, 255]);
            Cast::inset(
                &shadow(2.0, spread, offset),
                area(8, 8, 16, 16),
                0.0,
                1.0,
                Scale::IDENTITY,
            )
            .draw(&mut picture);
            picture
        };
        // Moved right by 4 the hole leaves the left edge uncovered... and covers 4 more pixels on the right.
        let moved = shift((4.0, 0.0), 0.0);
        assert_eq!(pixel(&moved, 9, 16), [0, 0, 0, 255], "the left band is wider");
        assert_eq!(pixel(&moved, 22, 16), [255, 255, 255, 255], "the right band is gone");
        let spread = shift((0.0, 0.0), 3.0);
        assert_eq!(
            pixel(&spread, 10, 16),
            [0, 0, 0, 255],
            "a spread of 3 darkens 3 more pixels in"
        );
    }

    #[test]
    fn a_fill_blends_over_what_is_there() {
        let mut picture = patch(area(0, 0, 16, 16), [0, 0, 200, 255]);
        fill(&mut picture, area(4, 4, 8, 8), Color::new(1.0, 0.0, 0.0, 1.0), 0.5);
        assert_eq!(pixel(&picture, 8, 8), [128, 0, 100, 255], "half red over blue");
        assert_eq!(pixel(&picture, 3, 8), [0, 0, 200, 255], "nothing outside the rectangle");
        fill(&mut picture, area(0, 0, 16, 16), Color::WHITE, 0.0);
        assert_eq!(
            pixel(&picture, 8, 8),
            [128, 0, 100, 255],
            "a fill of no alpha draws nothing"
        );
    }

    #[test]
    fn an_inset_shadow_fades_with_its_layer() {
        let mut picture = patch(area(0, 0, 32, 32), [255, 255, 255, 255]);
        Cast::inset(
            &shadow(6.0, 0.0, (0.0, 0.0)),
            area(8, 8, 16, 16),
            0.0,
            0.5,
            Scale::IDENTITY,
        )
        .draw(&mut picture);
        let edge = pixel(&picture, 8, 16)[0];
        assert!((120..140).contains(&edge), "half the shadow at the edge: {edge}");
    }

    #[test]
    fn a_stroke_covers_the_band_and_nothing_else() {
        let mut picture = patch(area(0, 0, 32, 32), [0, 0, 0, 255]);
        let held = Border {
            width: 2.0,
            offset: 0.0,
            inset: true,
            color: Color::WHITE,
        };
        Stroke::new(&held, area(8, 8, 16, 16), 0.0, Color::WHITE, 1.0, Scale::IDENTITY).draw(&mut picture);
        assert_eq!(pixel(&picture, 8, 16), [255, 255, 255, 255]);
        assert_eq!(pixel(&picture, 9, 16), [255, 255, 255, 255]);
        assert_eq!(pixel(&picture, 10, 16), [0, 0, 0, 255]);
        assert_eq!(pixel(&picture, 7, 16), [0, 0, 0, 255]);
    }

    #[test]
    fn a_stroke_is_anti_aliased_where_it_turns_a_corner() {
        let mut picture = patch(area(0, 0, 48, 48), [0, 0, 0, 255]);
        let held = Border {
            width: 2.0,
            offset: 0.0,
            inset: true,
            color: Color::WHITE,
        };
        Stroke::new(&held, area(8, 8, 32, 32), 12.0, Color::WHITE, 1.0, Scale::IDENTITY).draw(&mut picture);
        // Along the diagonal of the corner arc the ring is crossed obliquely: some pixels are part covered.
        let partial = (8..20)
            .map(|i| pixel(&picture, i, i)[0])
            .filter(|level| *level > 10 && *level < 245)
            .count();
        assert!(
            partial >= 1,
            "a rounded stroke has grey edge pixels, not only black and white"
        );
    }

    #[test]
    fn a_thin_stroke_is_fainter_not_aliased() {
        let mut picture = patch(area(0, 0, 32, 32), [0, 0, 0, 255]);
        let held = Border {
            width: 0.5,
            offset: 0.0,
            inset: true,
            color: Color::WHITE,
        };
        Stroke::new(&held, area(8, 8, 16, 16), 0.0, Color::WHITE, 1.0, Scale::IDENTITY).draw(&mut picture);
        let level = pixel(&picture, 8, 16)[0];
        assert!(
            (100..160).contains(&level),
            "half a pixel of stroke is half a level: {level}"
        );
    }

    #[test]
    fn a_stroke_takes_the_layers_tint_and_opacity() {
        let mut picture = patch(area(0, 0, 32, 32), [0, 0, 0, 255]);
        let held = Border {
            width: 2.0,
            offset: 0.0,
            inset: true,
            color: Color::WHITE,
        };
        Stroke::new(
            &held,
            area(8, 8, 16, 16),
            0.0,
            Color::new(1.0, 0.0, 0.0, 1.0),
            0.5,
            Scale::IDENTITY,
        )
        .draw(&mut picture);
        assert_eq!(
            pixel(&picture, 8, 16),
            [128, 0, 0, 255],
            "red at half opacity over black"
        );
    }

    #[test]
    fn a_ring_covers_every_pixel_a_stroke_changes_and_none_twice() {
        let outline = area(20, 20, 60, 40);
        let held = Border {
            width: 3.0,
            offset: 1.0,
            inset: false,
            color: Color::WHITE,
        };
        let stroke = Stroke::new(&held, outline, 10.0, Color::WHITE, 1.0, Scale::IDENTITY);
        let strips = ring(stroke.quad, outline, stroke.reach());
        let mut whole = patch(area(0, 0, 100, 80), [0, 0, 0, 255]);
        stroke.draw(&mut whole);
        let mut counts = vec![0_u8; 100 * 80];
        for strip in &strips {
            for y in strip.loc.y..strip.loc.y + strip.size.h {
                for x in strip.loc.x..strip.loc.x + strip.size.w {
                    counts[(y * 100 + x) as usize] += 1;
                }
            }
        }
        assert!(counts.iter().all(|count| *count <= 1), "no pixel is in two strips");
        for y in 0..80 {
            for x in 0..100 {
                if whole.bytes[((y * 100 + x) * 4) as usize] != 0 {
                    assert_eq!(
                        counts[(y * 100 + x) as usize],
                        1,
                        "({x}, {y}) is inked, so it is in a strip"
                    );
                }
            }
        }
        assert!(strips.len() > 1, "a big outline has a hollow middle");
        assert_eq!(
            ring(area(0, 0, 10, 10), area(0, 0, 10, 10), 20).len(),
            1,
            "a small one is whole"
        );
    }

    #[test]
    fn fading_puts_back_what_a_rounded_corner_hides() {
        let before = patch(area(0, 0, 16, 16), [0, 0, 255, 255]);
        let mut drawn = patch(area(0, 0, 16, 16), [255, 0, 0, 255]);
        fade(&mut drawn, &before, Shape::new(area(0, 0, 16, 16), 8.0), 1.0);
        assert_eq!(pixel(&drawn, 0, 0), [0, 0, 255, 255], "the corner is what was there");
        assert_eq!(pixel(&drawn, 8, 8), [255, 0, 0, 255], "the middle is the layer");
        let edge = pixel(&drawn, 2, 2);
        assert!(
            edge[0] > 0 && edge[0] < 255,
            "a pixel the arc crosses blends the two: {edge:?}"
        );
    }

    #[test]
    fn fading_by_an_opacity_mixes_the_whole_rectangle() {
        let before = patch(area(0, 0, 8, 8), [0, 0, 0, 255]);
        let mut drawn = patch(area(0, 0, 8, 8), [200, 100, 0, 255]);
        fade(&mut drawn, &before, Shape::new(area(0, 0, 8, 8), 0.0), 0.5);
        assert_eq!(pixel(&drawn, 4, 4), [100, 50, 0, 255]);
    }
}
