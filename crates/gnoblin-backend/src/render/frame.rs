//! What one output draws this frame, described without naming a renderer.
//!
//! A [`OutputFrame`] is the whole contract between the compositor and a renderer: the
//! colour behind everything, the layers in paint order, the cursor, and the
//! region that changed. It carries numbers, colours and client buffers only, so
//! the Pixman path of this slice and the GLES path of a later one read the same
//! description and cannot disagree about what a frame is.
//!
//! Coordinates are the ones every other crate states geometry in: output-local
//! logical pixels, origin at the output's top-left corner, y downwards. A layer's
//! `dst` places its content; the renderer multiplies by the output scale when it
//! needs physical pixels, so a description does not have to know which output it
//! is drawn on.
//!
//! Paint order is the order of [`OutputFrame::layers`], bottom first. The last layer is
//! on top, and the cursor is above all of them because a pointer a window could
//! cover would be unusable.
//!
//! # Failure modes
//!
//! 1. **A frame is drawn although nothing changed.** [`OutputFrame::is_empty`] answers
//!    whether there is damage, and a backend that respects it draws nothing for an
//!    idle iteration: `render::software::tests::an_empty_frame_draws_nothing` and
//!    the loop's `frame::tests::an_idle_iteration_draws_no_frame`.
//! 2. **A layer is drawn outside the damage.** Every layer is clipped to the
//!    frame's damage, so a rectangle that did not change is never repainted:
//!    `render::software::tests::damage_limits_what_is_repainted`.
//! 3. **An empty or inverted rectangle reaches the renderer.** A zero-width layer,
//!    a zero-area damage rectangle and a non-finite opacity are dropped by
//!    [`OutputFrame::push_layer`] and [`OutputFrame::push_damage`] instead of dividing by zero
//!    inside pixman: `render::software::tests::a_degenerate_layer_draws_nothing`.
//! 4. **A corner radius a shader cannot use reaches it.** A radius that is negative or
//!    not finite is dropped with the layer, the same way a non-finite opacity is, so a
//!    signed distance field never sees one: `tests::an_unusable_corner_radius_is_dropped`.
//!
//! # What a layer carries besides its pixels
//!
//! A layer names the visual properties the scene's [`VisualTransform`] and effect chain
//! hold: opacity, tint, corner radius and the passes of [`Effects`]. A renderer that
//! cannot draw a pass ignores it (the software path does: see
//! [`render::software`](crate::render::software)), and the GPU path draws it. Keeping
//! them in one description is what lets the two paths be compared pixel for pixel on
//! the properties they both implement.
//!
//! [`VisualTransform`]: gnoblin_render::VisualTransform

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_render::{Color, Effects, Mat4};
use gnoblin_types::{Rect, RectF};

/// One thing a frame paints.
#[derive(Clone, Debug)]
pub enum Layer {
    /// A client's buffer, cropped and placed.
    Surface(Surface),
    /// A rectangle of one colour, for the desktop background, an overlay flash or
    /// the built-in pointer.
    Solid(Solid),
}

/// A client buffer on the output.
///
/// The buffer is the client's `wl_shm` allocation, held by a handle so a client
/// that destroys it mid-frame keeps the object alive until the frame is done. The
/// renderer reads it through Smithay, which is the only safe way to touch memory
/// the client can also write.
#[derive(Clone, Debug)]
pub struct Surface {
    /// The client's buffer.
    pub buffer: WlBuffer,
    /// The crop to draw, in the buffer's own pixels.
    ///
    /// It is fractional because a viewported surface's source rectangle is stated
    /// in buffer coordinates divided by the buffer scale, which is not necessarily
    /// a whole pixel.
    pub src: RectF,
    /// Where the crop lands, in the output's logical space.
    pub dst: Rect,
    /// The buffer's `wl_surface.set_buffer_transform` orientation.
    pub transform: Transform,
    /// Group opacity, 0 to 1.
    pub opacity: f32,
    /// Multiplicative colour, white for none.
    pub tint: Color,
    /// Corner radius in logical pixels, 0 for square corners.
    ///
    /// It comes from the node's [`VisualTransform`](gnoblin_render::VisualTransform) and
    /// is a *clip*: the renderer draws the corner as background, the same way a rounded
    /// window reveals what is behind it.
    pub corner_radius: f32,
    /// The effect chain drawn with the layer, in application order.
    ///
    /// The passes are the scene's own [`Effects`], so the values a shader reads here are
    /// the values the script that set them was checked against.
    pub effects: Effects,
    /// How the layer sits in three dimensions, `None` when it is drawn flat.
    pub model: Option<Model>,
}

/// A solid rectangle on the output.
///
/// It is not `Copy` because it can carry an effect chain, which owns its passes.
#[derive(Clone, Debug)]
pub struct Solid {
    /// Where it lands, in the output's logical space.
    pub dst: Rect,
    /// Its colour, alpha included.
    pub colour: Color,
    /// Multiplicative colour, white for none.
    ///
    /// A solid carries a tint for the same reason a surface does: a script that dims a
    /// window dims whatever the window is made of, and the frame description should not
    /// have two rules for one property.
    pub tint: Color,
    /// Corner radius in logical pixels, 0 for square corners.
    pub corner_radius: f32,
    /// The effect chain drawn with the rectangle, in application order.
    pub effects: Effects,
    /// How the rectangle sits in three dimensions, `None` when it is drawn flat.
    pub model: Option<Model>,
}

impl Solid {
    /// A rectangle of one colour with square corners and no effects.
    ///
    /// This is the shape the desktop background, an overlay flash and the built-in
    /// pointer have, and the one every caller means unless it says otherwise.
    #[must_use]
    pub fn new(dst: Rect, colour: Color) -> Self {
        Self {
            dst,
            colour,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: Effects::new(),
            model: None,
        }
    }
}

/// How a layer sits in three dimensions.
///
/// A layer carries one when its node is tilted, moved in depth or drawn under a perspective
/// camera. The GLES renderer then draws the layer's plane through `matrix`, depth-tested
/// against the rest of the frame. A layer without one is drawn flat, in paint order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    /// Maps the layer's own plane, whose origin is its top-left corner and whose size is
    /// its `dst`, to the output's logical space before the perspective divide.
    pub matrix: Mat4,
    /// Whether the layer covers its plane with opaque pixels. An opaque layer is drawn
    /// front to back and writes depth; a transparent one is blended back to front after
    /// the opaque layers and does not write depth.
    pub opaque: bool,
}

/// The frame an output draws.
///
/// A frame is reused across iterations: [`OutputFrame::clear`] drops the layers but
/// keeps their allocation, so a steady session renders a frame without allocating.
#[derive(Clone, Debug, Default)]
pub struct OutputFrame {
    background: Color,
    damage: Vec<Rect>,
    layers: Vec<Layer>,
    cursor: Option<Solid>,
}

impl OutputFrame {
    /// An empty frame with an opaque black background.
    ///
    /// Black rather than transparent because a frame with no layers must still
    /// leave defined pixels: a screenshot of an empty session is the background,
    /// not whatever the target held before.
    #[must_use]
    pub fn new() -> Self {
        Self {
            background: Color::BLACK,
            damage: Vec::new(),
            layers: Vec::new(),
            cursor: None,
        }
    }

    /// Empties the frame for the next description, keeping the allocation.
    pub fn clear(&mut self) {
        self.background = Color::BLACK;
        self.damage.clear();
        self.layers.clear();
        self.cursor = None;
    }

    /// The colour behind every layer.
    #[must_use]
    pub fn background(&self) -> Color {
        self.background
    }

    /// Sets the colour behind every layer.
    ///
    /// A non-finite colour is ignored: a background of `NaN` would poison every
    /// pixel the output ever draws.
    pub fn set_background(&mut self, colour: Color) {
        if colour.is_finite() {
            self.background = colour;
        }
    }

    /// The rectangles to redraw, in the output's logical space.
    #[must_use]
    pub fn damage(&self) -> &[Rect] {
        &self.damage
    }

    /// Adds a rectangle to redraw, ignoring an empty one.
    ///
    /// Rectangles are not merged here: `gnoblin_render` already merges the scene's
    /// damage under its own budget, and a second merge would only hide which
    /// rectangle came from where.
    pub fn push_damage(&mut self, rect: Rect) {
        if !rect.is_empty() {
            self.damage.push(rect);
        }
    }

    /// Whether nothing has to be drawn.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.damage.is_empty()
    }

    /// The layers, bottom first.
    #[must_use]
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Adds a layer on top of the ones already described.
    ///
    /// A degenerate layer is dropped rather than drawn: a zero extent has nothing
    /// to show, a non-finite opacity would reach the blend as a division by zero, and a
    /// corner radius of `NaN` would reach a signed distance field as one. The effect
    /// chain itself needs no re-check here: [`Effects`] validates a pass as it is
    /// pushed, so an invalid pass cannot be inside a layer.
    pub fn push_layer(&mut self, layer: Layer) {
        match &layer {
            Layer::Surface(surface) => {
                if surface.dst.is_empty() || surface.src.width <= 0.0 || surface.src.height <= 0.0 {
                    return;
                }
                if !surface.opacity.is_finite() {
                    return;
                }
                if !(0.0..=1.0).contains(&surface.opacity) || !surface.tint.is_finite() {
                    return;
                }
                if !surface.corner_radius.is_finite() || surface.corner_radius < 0.0 {
                    return;
                }
            }
            Layer::Solid(solid) => {
                if solid.dst.is_empty() || !solid.colour.is_finite() || !solid.tint.is_finite() {
                    return;
                }
                // A transparent rectangle is nothing to draw — unless it carries effects,
                // which is how a layer that only *blurs what is behind it* is stated.
                if solid.colour.a <= 0.0 && solid.effects.is_empty() {
                    return;
                }
                if !solid.corner_radius.is_finite() || solid.corner_radius < 0.0 {
                    return;
                }
            }
        }
        self.layers.push(layer);
    }

    /// The cursor, drawn above every layer.
    ///
    /// The seat's cursor image needs a theme loader that this slice does not have,
    /// so a session composes no cursor yet; the slot exists so the frame
    /// description the GLES path and capture will read does not change shape when
    /// it arrives.
    #[must_use]
    pub fn cursor(&self) -> Option<&Solid> {
        self.cursor.as_ref()
    }

    /// Sets the cursor.
    pub fn set_cursor(&mut self, cursor: Option<Solid>) {
        self.cursor =
            cursor.filter(|solid| !solid.dst.is_empty() && solid.colour.is_finite() && solid.tint.is_finite());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_layer_draws_nothing() {
        let mut frame = OutputFrame::new();
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 0, 10), Color::WHITE)));
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 0, 10), Color::TRANSPARENT)));
        frame.push_layer(Layer::Solid(Solid::new(
            Rect::new(0, 0, 10, 10),
            Color::new(0.0, 0.0, 0.0, f32::NAN),
        )));
        assert!(frame.layers().is_empty(), "nothing to draw, nothing described");
    }

    #[test]
    fn an_empty_rectangle_is_not_damage() {
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 0, 4));
        assert!(frame.is_empty(), "a rectangle with no pixels changed nothing");
        frame.push_damage(Rect::new(4, 4, 2, 2));
        assert!(!frame.is_empty());
        assert_eq!(frame.damage(), &[Rect::new(4, 4, 2, 2)]);
    }

    #[test]
    fn clearing_a_frame_keeps_its_allocation_and_drops_the_description() {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::from_rgba8(1, 2, 3, 255));
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 4, 4), Color::WHITE)));
        frame.push_damage(Rect::new(0, 0, 4, 4));
        let capacity = frame.layers.capacity();
        frame.clear();
        assert!(frame.layers().is_empty() && frame.damage().is_empty());
        assert!(frame.cursor().is_none());
        assert_eq!(frame.background(), Color::BLACK, "the default background is back");
        assert_eq!(frame.layers.capacity(), capacity, "the allocation is reused");
    }

    #[test]
    fn an_unusable_corner_radius_is_dropped() {
        // A corner radius reaches a signed distance field, which turns a NaN into a
        // coverage of NaN and a whole layer of garbage; a negative one rounds the
        // wrong way. Both are refused with the layer.
        let mut frame = OutputFrame::new();
        let mut solid = Solid::new(Rect::new(0, 0, 8, 8), Color::WHITE);
        solid.corner_radius = f32::NAN;
        frame.push_layer(Layer::Solid(solid));
        let mut solid = Solid::new(Rect::new(0, 0, 8, 8), Color::WHITE);
        solid.corner_radius = -2.0;
        frame.push_layer(Layer::Solid(solid));
        assert!(frame.layers().is_empty(), "neither radius is drawable");

        let mut solid = Solid::new(Rect::new(0, 0, 8, 8), Color::WHITE);
        solid.corner_radius = 3.0;
        frame.push_layer(Layer::Solid(solid));
        assert_eq!(frame.layers().len(), 1, "a radius a corner can hold is drawn");
    }

    #[test]
    fn a_non_finite_background_is_ignored() {
        let mut frame = OutputFrame::new();
        frame.set_background(Color::new(f32::INFINITY, 0.0, 0.0, 1.0));
        assert_eq!(frame.background(), Color::BLACK);
    }
}
