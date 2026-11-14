//! Rounded shapes, borders and shadows as spans of pixels.
//!
//! Pixman has no antialiased mask to draw a rounded rectangle with, so every shape here is the one-pixel-high spans
//! it covers, the same way a corner radius is drawn: a pixel is in when its centre is. A border is the band between
//! two such shapes, one grown by the band's far edge and one by its near edge. A shadow is a stack of nested shapes,
//! drawn outermost first, each with the alpha that brings the pixels it covers to the falloff the GLES shader draws
//! there. The edge of every shape is hard, and the shadow's falloff is stepped rather than smooth: the software
//! renderer is the reference for placement, not for antialiasing.
//!
//! The geometry is the GLES renderer's: the same band for a border (`wgsl/border.wgsl`), the same smoothstep falloff
//! over the blur for a shadow (`wgsl/shadow.wgsl`), both measured from the same rounded rectangle.

use std::collections::BTreeMap;

use smithay::backend::renderer::{Color32F, Frame};
use smithay::utils::{Physical, Rectangle};

use gnoblin_render::{Border, Shadow};
use gnoblin_types::{Color, Scale};

use gnoblin_render_frame::error::RenderError;

use crate::{colour, corner_spans, failed};

/// The most nested shapes one shadow is drawn with. The falloff is stepped at one pixel up to this many steps, so a
/// wider blur is drawn with steps wider than a pixel rather than with more of them.
const SHADOW_STEPS: usize = 16;

/// A rectangle of the output, in physical pixels.
type Area = Rectangle<i32, Physical>;

/// `rect` grown by `by` pixels on every side; a negative `by` shrinks it. A rectangle that shrinks away has no size,
/// which [`rows`] draws as nothing.
pub(crate) fn grown(rect: Area, by: i32) -> Area {
    Rectangle::new(
        (rect.loc.x - by, rect.loc.y - by).into(),
        (rect.size.w + 2 * by, rect.size.h + 2 * by).into(),
    )
}

/// The one-pixel-high spans of `rect` with corners of `radius`, in the output's own coordinates.
pub(crate) fn rows(rect: Area, radius: f64) -> Vec<Area> {
    corner_spans(rect.size, radius.max(0.0))
        .into_iter()
        .map(|span| Rectangle::new(span.loc + rect.loc, span.size))
        .collect()
}

/// The spans of the band between `near` and `far` pixels of distance from the edge of `shape` (negative inside it),
/// which is what a border covers. A band of no width, or a far edge inside the near one, has no spans.
pub(crate) fn band(shape: Area, radius: f64, near: i32, far: i32) -> Vec<Area> {
    if far <= near {
        return Vec::new();
    }
    let outer = rows(grown(shape, far), radius + f64::from(far));
    let inner: BTreeMap<i32, (i32, i32)> = rows(grown(shape, near), radius + f64::from(near))
        .into_iter()
        .map(|span| (span.loc.y, (span.loc.x, span.loc.x + span.size.w)))
        .collect();
    let mut spans = Vec::with_capacity(outer.len());
    for span in outer {
        let (left, right) = (span.loc.x, span.loc.x + span.size.w);
        let row = |from: i32, to: i32| Rectangle::new((from, span.loc.y).into(), (to - from, 1).into());
        match inner.get(&span.loc.y) {
            Some(&(inner_left, inner_right)) if inner_right > inner_left => {
                if inner_left > left {
                    spans.push(row(left, inner_left.min(right)));
                }
                if inner_right < right {
                    spans.push(row(inner_right.max(left), right));
                }
            }
            _ => spans.push(row(left, right)),
        }
    }
    spans
}

/// Fills `spans` with `colour` where they meet `damage`.
///
/// The fill is drawn against the spans' bounding rectangle, with the damaged parts of it as the rectangles to
/// cover, because pixman takes a destination and the parts of it to paint.
pub(crate) fn fill<F>(
    frame: &mut F,
    spans: &[Area],
    damage: &[Area],
    colour: Color32F,
    parts: &mut Vec<Area>,
) -> Result<(), RenderError>
where
    F: Frame,
    F::Error: std::fmt::Debug,
{
    let Some(bounds) = spans.iter().copied().reduce(union) else {
        return Ok(());
    };
    parts.clear();
    for damaged in damage {
        let Some(within) = damaged.intersection(bounds) else {
            continue;
        };
        for span in spans {
            if let Some(part) = within.intersection(*span) {
                parts.push(Rectangle::new(part.loc - bounds.loc, part.size));
            }
        }
    }
    if parts.is_empty() {
        return Ok(());
    }
    frame.draw_solid(bounds, parts, colour).map_err(failed)
}

fn union(a: Area, b: Area) -> Area {
    let left = a.loc.x.min(b.loc.x);
    let top = a.loc.y.min(b.loc.y);
    let right = (a.loc.x + a.size.w).max(b.loc.x + b.size.w);
    let bottom = (a.loc.y + a.size.h).max(b.loc.y + b.size.h);
    Rectangle::new((left, top).into(), (right - left, bottom - top).into())
}

/// A pixel count from a length in logical pixels at `scale`, rounded to the nearest pixel.
fn pixels(logical: f32, scale: Scale) -> i32 {
    (f64::from(logical) * scale.as_f64()).round() as i32
}

/// The band a border covers, in physical pixels from the edge of its shape: `(near, far)`, negative inside. This is
/// the same band the GLES border shader is given.
pub(crate) fn border_band(border: &Border, scale: Scale) -> (i32, i32) {
    let offset = f64::from(border.offset) * scale.as_f64();
    let width = f64::from(border.width) * scale.as_f64();
    let (near, far) = if border.inset {
        (-(offset + width), -offset)
    } else {
        (offset, offset + width)
    };
    (near.round() as i32, far.round() as i32)
}

/// Draws a shadow behind `shape`: the nested shapes from the outermost in, each with the alpha that makes the
/// composite at its distance equal the shader's `colour.a * (1 - smoothstep(0, blur, distance))`.
///
/// `shape` is the rectangle the shadow follows, `radius` its corner radius, both physical. The shadow's own shape is
/// that rectangle moved by the offset and grown by the spread, so a distance is measured from there, and inside it
/// the shadow is as dark as its nearest step.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shadow<F>(
    frame: &mut F,
    shadow: &Shadow,
    shape: Area,
    radius: f64,
    scale: Scale,
    damage: &[Area],
    parts: &mut Vec<Area>,
) -> Result<(), RenderError>
where
    F: Frame,
    F::Error: std::fmt::Debug,
{
    let blur = f64::from(shadow.blur) * scale.as_f64();
    let offset = Rectangle::new(
        (
            shape.loc.x + pixels(shadow.offset.x as f32, scale),
            shape.loc.y + pixels(shadow.offset.y as f32, scale),
        )
            .into(),
        shape.size,
    );
    let spread = pixels(shadow.spread, scale);
    let base = grown(offset, spread);
    let base_radius = radius + f64::from(spread);
    let straight = shadow.color;
    let strength = straight.a * shadow.opacity;
    // At zero blur the shadow is a hard copy of the shape.
    let steps = (blur.ceil() as usize).clamp(1, SHADOW_STEPS);
    let reach = (blur.round() as i32).max(1);
    // The falloff at the middle of step `j`, which runs over distances `j * blur / steps` to `(j + 1) * blur / steps`.
    let falloff = |j: usize| {
        let distance = (j as f64 + 0.5) / steps as f64;
        1.0 - smoothstep(distance)
    };
    // Drawn from the outermost step in, so the pixels of step `j` take the draws of every step from the last down to
    // `j`. The alpha of each draw is what turns the composite of the steps outside it into that step's target.
    let mut outer = 0.0_f64;
    for j in (0..steps).rev() {
        let target = falloff(j);
        let alpha = if outer >= 1.0 {
            0.0
        } else {
            1.0 - (1.0 - target) / (1.0 - outer)
        };
        outer = target;
        if alpha <= 0.0 {
            continue;
        }
        // The step covers everything within `(j + 1) * blur / steps` of the shape's edge.
        let grow = ((j + 1) as f64 * f64::from(reach) / steps as f64).round() as i32;
        let spans = rows(grown(base, grow), base_radius + f64::from(grow));
        let coloured = colour(Color::new(straight.r, straight.g, straight.b, strength * alpha as f32));
        fill(frame, &spans, damage, coloured, parts)?;
    }
    Ok(())
}

/// `smoothstep(0, 1, x)`, as the shader's falloff is written.
fn smoothstep(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(x: i32, y: i32, w: i32, h: i32) -> Area {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn covers(spans: &[Area], x: i32, y: i32) -> bool {
        spans
            .iter()
            .any(|span| y == span.loc.y && x >= span.loc.x && x < span.loc.x + span.size.w)
    }

    #[test]
    fn an_inside_band_is_the_stroke_along_the_edge() {
        let spans = band(area(8, 8, 48, 48), 0.0, -2, 0);
        assert!(
            covers(&spans, 8, 30) && covers(&spans, 9, 30),
            "the stroke's two columns"
        );
        assert!(!covers(&spans, 10, 30) && !covers(&spans, 7, 30), "and not beside them");
        assert!(
            covers(&spans, 30, 8) && covers(&spans, 30, 55),
            "the top and bottom rows"
        );
        assert!(covers(&spans, 20, 9) && covers(&spans, 20, 8));
        assert!(!covers(&spans, 20, 10), "the middle is clear");
    }

    #[test]
    fn an_outside_band_sits_beyond_the_edge() {
        let spans = band(area(8, 8, 48, 48), 0.0, 1, 3);
        assert!(covers(&spans, 5, 30) && covers(&spans, 6, 30));
        assert!(!covers(&spans, 7, 30) && !covers(&spans, 4, 30) && !covers(&spans, 8, 30));
    }

    #[test]
    fn a_rounded_band_bends_round_the_corner() {
        let spans = band(area(8, 8, 48, 48), 12.0, -2, 0);
        assert!(
            !covers(&spans, 8, 8) && !covers(&spans, 9, 9),
            "the corner pixel is outside the arc"
        );
        assert!(covers(&spans, 12, 12), "the stroke runs through the corner's arc");
        assert!(!covers(&spans, 14, 14), "and the fill is inside it");
    }

    #[test]
    fn a_band_with_its_far_edge_inside_its_near_edge_is_empty() {
        assert!(band(area(0, 0, 10, 10), 0.0, 2, 2).is_empty());
        assert!(band(area(0, 0, 10, 10), 0.0, 3, 1).is_empty());
    }

    #[test]
    fn a_border_band_matches_the_edges_the_gles_shader_is_given() {
        let border = |offset: f32, inset: bool| Border {
            width: 2.0,
            offset,
            inset,
            color: gnoblin_types::Color::WHITE,
        };
        assert_eq!(border_band(&border(1.0, false), Scale::IDENTITY), (1, 3));
        assert_eq!(border_band(&border(0.0, true), Scale::IDENTITY), (-2, 0));
        assert_eq!(border_band(&border(1.0, true), Scale::new(3, 2)), (-5, -2));
    }

    // ---- what the renderer draws, by probe -----------------------------------------------------------------------

    use crate::Software;
    use gnoblin_render::{Effects, Pass};
    use gnoblin_render_frame::frame::{Layer, OutputFrame, Solid};
    use gnoblin_types::{Rect, Size};

    const BLUE: Color = Color::new(0.0, 0.0, 1.0, 1.0);
    const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);
    const GREEN: Color = Color::new(0.0, 1.0, 0.0, 1.0);

    fn picture(background: Color, layers: Vec<Solid>) -> gnoblin_render_frame::image::Image {
        let size = Size::new(96, 96);
        let mut software = Software::new(size).expect("a software renderer");
        let mut frame = OutputFrame::new();
        frame.set_background(background);
        frame.push_damage(Rect::new(0, 0, size.width, size.height));
        for layer in layers {
            frame.push_layer(Layer::Solid(layer));
        }
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        software.screenshot().expect("a readback")
    }

    fn chain(passes: Vec<Pass>) -> Effects {
        let mut effects = Effects::new();
        for pass in passes {
            effects.push(pass).expect("a pass in range");
        }
        effects
    }

    fn stroke(width: f32, offset: f32, inset: bool, color: Color) -> Pass {
        Pass::Border(Border {
            width,
            offset,
            inset,
            color,
        })
    }

    fn soft_shadow(frame: bool) -> Pass {
        Pass::Shadow(Shadow {
            offset: gnoblin_types::Vec3::ZERO,
            blur: 10.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame,
        })
    }

    fn luma(pixel: [u8; 4]) -> u32 {
        u32::from(pixel[0]) * 3 + u32::from(pixel[1]) * 6 + u32::from(pixel[2])
    }

    #[test]
    fn a_silhouette_cuts_each_layer_from_the_frames_corners() {
        let frame = Rect::new(8, 8, 48, 48);
        let layer = |rect: Rect, colour: Color| {
            let mut solid = Solid::new(rect, colour);
            solid.corner_radius = 10.0;
            solid.silhouette = Some(frame);
            solid
        };
        let image = picture(
            BLUE,
            vec![
                layer(Rect::new(8, 8, 48, 12), RED),
                layer(Rect::new(8, 20, 48, 36), GREEN),
            ],
        );
        assert_eq!(
            image.colour_at(9, 9),
            Some([0, 0, 255, 255]),
            "the frame's corner is cut"
        );
        assert_eq!(
            image.colour_at(9, 54),
            Some([0, 0, 255, 255]),
            "and the content's bottom corner"
        );
        assert_eq!(
            image.colour_at(9, 21),
            Some([0, 255, 0, 255]),
            "the join under the bar is not a corner of the frame"
        );
        assert_eq!(image.colour_at(32, 14), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(32, 40), Some([0, 255, 0, 255]));
    }

    #[test]
    fn borders_are_drawn_as_rings_and_stack_in_list_order() {
        let mut layer = Solid::new(Rect::new(8, 8, 48, 48), Color::TRANSPARENT);
        layer.effects = chain(vec![stroke(2.0, 0.0, true, RED), stroke(2.0, 2.0, true, GREEN)]);
        let image = picture(BLUE, vec![layer]);
        assert_eq!(image.colour_at(8, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(9, 30), Some([255, 0, 0, 255]));
        assert_eq!(
            image.colour_at(10, 30),
            Some([0, 255, 0, 255]),
            "the second ring sits inside the first"
        );
        assert_eq!(image.colour_at(11, 30), Some([0, 255, 0, 255]));
        assert_eq!(image.colour_at(12, 30), Some([0, 0, 255, 255]));
        assert_eq!(
            image.colour_at(7, 30),
            Some([0, 0, 255, 255]),
            "an inside border stays inside"
        );

        let mut outside = Solid::new(Rect::new(8, 8, 48, 48), Color::TRANSPARENT);
        outside.effects = chain(vec![stroke(2.0, 1.0, false, RED)]);
        let image = picture(Color::WHITE, vec![outside]);
        assert_eq!(image.colour_at(5, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(6, 30), Some([255, 0, 0, 255]));
        assert_eq!(image.colour_at(7, 30), Some([255, 255, 255, 255]));
        assert_eq!(image.colour_at(4, 30), Some([255, 255, 255, 255]));
    }

    #[test]
    fn a_border_follows_the_rounded_corner() {
        let mut fill = Solid::new(Rect::new(8, 8, 48, 48), GREEN);
        fill.corner_radius = 12.0;
        fill.effects = chain(vec![stroke(2.0, 0.0, true, RED)]);
        let image = picture(BLUE, vec![fill]);
        assert_eq!(image.colour_at(9, 9), Some([0, 0, 255, 255]), "the corner is cut");
        assert_eq!(image.colour_at(8, 32), Some([255, 0, 0, 255]), "the edge is the border");
        assert_eq!(
            image.colour_at(32, 32),
            Some([0, 255, 0, 255]),
            "the middle is the fill"
        );
        assert_eq!(
            image.colour_at(12, 12),
            Some([255, 0, 0, 255]),
            "the border bends round the corner"
        );
        assert_eq!(image.colour_at(14, 14), Some([0, 255, 0, 255]));
    }

    #[test]
    fn a_shadow_is_darkest_at_the_edge_and_never_darker_further_out() {
        let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
        solid.corner_radius = 8.0;
        solid.effects = chain(vec![soft_shadow(false)]);
        let image = picture(Color::WHITE, vec![solid]);
        let mut previous = 0;
        for x in (12..32).rev() {
            let brightness = luma(image.colour_at(x, 48).expect("a pixel"));
            assert!(
                brightness >= previous,
                "x = {x} is {brightness}, the pixel nearer was {previous}"
            );
            previous = brightness;
        }
        assert!(
            luma(image.colour_at(31, 48).expect("a pixel")) < luma(image.colour_at(22, 48).expect("a pixel")),
            "darker at the edge than ten pixels out"
        );
        assert_eq!(
            image.colour_at(8, 48),
            Some([255, 255, 255, 255]),
            "past the blur nothing is drawn"
        );
    }

    #[test]
    fn a_frame_shadow_covers_the_room_above_the_content() {
        let frame = Rect::new(16, 16, 64, 64);
        let body = |follows_frame: bool| {
            let mut solid = Solid::new(Rect::new(16, 40, 64, 40), RED);
            solid.silhouette = Some(frame);
            solid.effects = chain(vec![soft_shadow(follows_frame)]);
            solid
        };
        let with_frame = picture(Color::WHITE, vec![body(true)]);
        let content_only = picture(Color::WHITE, vec![body(false)]);
        assert!(
            luma(with_frame.colour_at(48, 28).expect("a pixel")) < 200,
            "the room above the content is shadowed"
        );
        assert_eq!(content_only.colour_at(48, 28), Some([255, 255, 255, 255]));
    }

    #[test]
    fn a_layer_placed_in_depth_is_drawn_without_its_border_and_shadow() {
        let mut solid = Solid::new(Rect::new(32, 32, 32, 32), RED);
        solid.model = Some(gnoblin_render_frame::frame::Model {
            matrix: gnoblin_render::Mat4::IDENTITY,
            opaque: true,
        });
        solid.effects = chain(vec![soft_shadow(false), stroke(2.0, 1.0, false, GREEN)]);
        let image = picture(Color::WHITE, vec![solid]);
        assert_eq!(
            image.colour_at(30, 48),
            Some([255, 255, 255, 255]),
            "no shadow beside it"
        );
        assert_eq!(
            image.colour_at(29, 48),
            Some([255, 255, 255, 255]),
            "no border beside it"
        );
    }
}
