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
//!    idle iteration: `gnoblin_render_software::tests::an_empty_frame_draws_nothing` and
//!    the loop's `frame::tests::an_idle_iteration_draws_no_frame`.
//! 2. **A layer is drawn outside the damage.** Every layer is clipped to the
//!    frame's damage, so a rectangle that did not change is never repainted:
//!    `gnoblin_render_software::tests::damage_limits_what_is_repainted`.
//! 3. **An empty or inverted rectangle reaches the renderer.** A zero-width layer,
//!    a zero-area damage rectangle and a non-finite opacity are dropped by
//!    [`OutputFrame::push_layer`] and [`OutputFrame::push_damage`] instead of dividing by zero
//!    inside pixman: `gnoblin_render_software::tests::a_degenerate_layer_draws_nothing`.
//! 4. **A corner radius a shader cannot use reaches it.** A radius that is negative or
//!    not finite is dropped with the layer, the same way a non-finite opacity is, so a
//!    signed distance field never sees one: `tests::an_unusable_corner_radius_is_dropped`.
//!
//! # What a layer carries besides its pixels
//!
//! A layer names the visual properties the scene's [`VisualTransform`] and effect chain
//! hold: opacity, tint, corner radius and the passes of [`Effects`]. A renderer that
//! cannot draw a pass ignores it (the software path does: see
//! `gnoblin-render-software`), and the GPU path draws it. Keeping
//! them in one description is what lets the two paths be compared pixel for pixel on
//! the properties they both implement.
//!
//! [`VisualTransform`]: gnoblin_animation::VisualTransform

use std::sync::Arc;

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_render::{Effects, Mat4};
use gnoblin_types::{Color, Rect, RectF};

use super::image::Image;

/// One thing a frame paints.
#[derive(Clone, Debug)]
pub enum Layer {
    /// A client's buffer, cropped and placed.
    Surface(Surface),
    /// A rectangle of one colour, for the desktop background, an overlay flash or
    /// the built-in pointer.
    Solid(Solid),
    /// The screen overlay, above the layers before it and below the cursor. It is one
    /// entry in the list, so paint order is a single sequence. The caller pushes at most
    /// one, last, so it stays on top.
    Overlay(Overlay),
}

/// What a capture draws in place of a secure layer.
///
/// It changes only the pixels a capture reads. The display still shows the layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SecureTreatment {
    /// A neutral rectangle over the layer's place, so a viewer can see that something is hidden.
    #[default]
    Mask,
    /// Nothing for the layer, so the capture shows whatever lies beneath it.
    Omit,
}

impl Layer {
    /// Whether the layer is secure, so a capture replaces it.
    fn is_secure(&self) -> bool {
        match self {
            Self::Surface(surface) => surface.secure,
            Self::Solid(solid) => solid.secure,
            Self::Overlay(overlay) => overlay.secure,
        }
    }

    /// The neutral rectangle a capture draws in place of this secure layer.
    ///
    /// It keeps the layer's place, corners and perspective, so a masked window takes the shape
    /// the display showed. It drops the effects, which act on the content the mask hides.
    fn mask(&self) -> Solid {
        let grey = Color::new(0.5, 0.5, 0.5, 1.0);
        match self {
            Self::Surface(surface) => Solid {
                corner_radius: surface.corner_radius,
                model: surface.model,
                ..Solid::new(surface.dst, grey)
            },
            Self::Solid(solid) => Solid {
                corner_radius: solid.corner_radius,
                model: solid.model,
                ..Solid::new(solid.dst, grey)
            },
            Self::Overlay(overlay) => Solid::new(overlay.dst(), grey),
        }
    }
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
    /// It comes from the node's [`VisualTransform`](gnoblin_animation::VisualTransform) and
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
    /// Whether a capture replaces this layer; the display still shows it.
    pub secure: bool,
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
    /// Whether a capture replaces this layer; the display still shows it.
    pub secure: bool,
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
            secure: false,
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

/// A compositor-drawn image on top of every layer: the portal dialog.
///
/// Unlike a [`Surface`] it has no client buffer, so it is not tied to a
/// `wl_buffer` and survives the client that asked for it. The pixels are
/// premultiplied at construction, because both renderers blend premultiplied
/// colour and a straight-alpha edge would brighten. It is drawn above every
/// layer and below the cursor. A capture leaves it in unless the overlay is secure;
/// then the capture replaces it with the frame's [`SecureTreatment`].
#[derive(Clone, Debug)]
pub struct Overlay {
    /// Where the image lands, in the output's logical space.
    dst: Rect,
    /// The premultiplied RGBA8 pixels, rows packed.
    image: Arc<Image>,
    /// Whether a capture replaces the overlay; the display still shows it.
    secure: bool,
}

impl Overlay {
    /// An overlay of `image`, drawn at `dst`.
    ///
    /// Returns [`None`] when `dst` is empty. The image's own size is the source
    /// rectangle, so a `dst` of another size scales it.
    #[must_use]
    pub fn new(dst: Rect, image: &Image) -> Option<Self> {
        if dst.is_empty() {
            return None;
        }
        let size = image.size();
        let width = size.width as usize;
        let height = size.height as usize;
        let packed = width.checked_mul(4)?;
        let mut data = Vec::with_capacity(packed.checked_mul(height)?);
        for row in image.data().chunks(image.stride()).take(height) {
            for pixel in row.get(..packed)?.as_chunks::<4>().0 {
                let alpha = u32::from(pixel[3]);
                let premultiply =
                    |channel: u8| u8::try_from((u32::from(channel) * alpha + 127) / 255).unwrap_or(u8::MAX);
                data.extend_from_slice(&[
                    premultiply(pixel[0]),
                    premultiply(pixel[1]),
                    premultiply(pixel[2]),
                    pixel[3],
                ]);
            }
        }
        let image = Image::new(size, packed, data)?;
        Some(Self {
            dst,
            image: Arc::new(image),
            secure: false,
        })
    }

    /// Where the image lands, in the output's logical space.
    #[must_use]
    pub fn dst(&self) -> Rect {
        self.dst
    }

    /// The premultiplied pixels.
    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// Marks the overlay secure, so a capture replaces it with the frame's secure treatment.
    #[must_use]
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Whether a capture replaces this overlay.
    #[must_use]
    pub fn is_secure(&self) -> bool {
        self.secure
    }
}

/// The smallest magnification a [`View`] may ask for: no magnification at all.
pub const MIN_ZOOM: f32 = 1.0;

/// The largest magnification a [`View`] may ask for.
///
/// Past this a source rectangle is a few pixels across and the picture is only blocks.
pub const MAX_ZOOM: f32 = 16.0;

/// A magnified view of the output, drawn in place of the whole frame.
///
/// The renderer shows the source rectangle of size `output / zoom` centred on
/// (`centre_x`, `centre_y`) in the output's logical space, clamped so it never leaves
/// the output. The view samples only the frame of its own output, so it can never show
/// another output, or a lock screen drawn on one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The magnification, between [`MIN_ZOOM`] and [`MAX_ZOOM`].
    pub zoom: f32,
    /// The point of the output that stays at the middle of the view, x.
    pub centre_x: f32,
    /// The point of the output that stays at the middle of the view, y.
    pub centre_y: f32,
}

impl View {
    /// A view, or [`None`] when the zoom is outside the allowed range or a value is not finite.
    ///
    /// The range test is written so that `NaN` fails it.
    #[must_use]
    pub fn new(zoom: f32, centre_x: f32, centre_y: f32) -> Option<Self> {
        let valid = (MIN_ZOOM..=MAX_ZOOM).contains(&zoom) && centre_x.is_finite() && centre_y.is_finite();
        valid.then_some(Self {
            zoom,
            centre_x,
            centre_y,
        })
    }
}

/// A colour matrix applied to every pixel of the frame.
///
/// Row `i` gives output channel `i` (red, green, blue, alpha) as the sum of the four
/// input channels times `rows[i][0..4]`, plus the offset `rows[i][4]`. The matrix acts on
/// straight (unpremultiplied) colour, so a renderer that stores premultiplied pixels must
/// divide by alpha first. The presets (invert, greyscale, colour-blindness simulation)
/// are built from this type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourMatrix {
    /// The four rows, one per output channel, each with four weights and an offset.
    pub rows: [[f32; 5]; 4],
}

impl ColourMatrix {
    /// The matrix that leaves every colour unchanged.
    pub const IDENTITY: Self = Self {
        rows: [
            [1.0, 0.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0, 0.0],
        ],
    };

    /// A matrix from its rows, or [`None`] when any entry is not finite.
    #[must_use]
    pub fn new(rows: [[f32; 5]; 4]) -> Option<Self> {
        rows.iter()
            .flatten()
            .all(|value| value.is_finite())
            .then_some(Self { rows })
    }
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
    cursor: Option<Overlay>,
    /// The magnified view, `None` for the whole frame at its own scale.
    view: Option<View>,
    /// The colour matrix applied to the whole frame, `None` for the colours as drawn.
    colour_matrix: Option<ColourMatrix>,
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
            view: None,
            colour_matrix: None,
        }
    }

    /// Empties the frame for the next description, keeping the allocation.
    pub fn clear(&mut self) {
        self.background = Color::BLACK;
        self.damage.clear();
        self.layers.clear();
        self.cursor = None;
        self.view = None;
        self.colour_matrix = None;
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
            Layer::Overlay(overlay) => {
                if overlay.dst().is_empty() {
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

    /// The cursor image, drawn above every layer and above any overlay.
    ///
    /// The image's destination already accounts for the hotspot, so the caller places
    /// the rectangle at the pointer minus the hotspot. Unlike the overlay, a capture
    /// does not leave the cursor out.
    #[must_use]
    pub fn cursor(&self) -> Option<&Overlay> {
        self.cursor.as_ref()
    }

    /// Sets the cursor image, or clears it with [`None`].
    pub fn set_cursor_image(&mut self, cursor: Option<Overlay>) {
        self.cursor = cursor;
    }

    /// The magnified view, if one is set.
    #[must_use]
    pub fn view(&self) -> Option<View> {
        self.view
    }

    /// Sets the magnified view, or removes it with [`None`].
    ///
    /// A view whose zoom is out of range, or whose centre is not finite, is dropped, so
    /// the frame is drawn unmagnified rather than with a value a renderer cannot use.
    pub fn set_view(&mut self, view: Option<View>) {
        self.view = view.filter(|view| View::new(view.zoom, view.centre_x, view.centre_y).is_some());
    }

    /// The colour matrix applied to the whole frame, if one is set.
    #[must_use]
    pub fn colour_matrix(&self) -> Option<ColourMatrix> {
        self.colour_matrix
    }

    /// Sets the colour matrix applied to the whole frame, or removes it with [`None`].
    ///
    /// A matrix with a non-finite entry is dropped, for the same reason as a view.
    pub fn set_colour_matrix(&mut self, matrix: Option<ColourMatrix>) {
        self.colour_matrix = matrix.filter(|matrix| ColourMatrix::new(matrix.rows).is_some());
    }

    /// Whether any layer is secure, so a capture has to replace something.
    #[must_use]
    pub fn has_secure(&self) -> bool {
        self.layers.iter().any(Layer::is_secure)
    }

    /// The same frame with all of `bounds` damaged, so a draw repaints the whole output.
    #[must_use]
    pub fn fully_damaged(&self, bounds: Rect) -> Self {
        let mut frame = self.clone();
        frame.damage.clear();
        frame.damage.push(bounds);
        frame
    }

    /// The same frame with every secure layer replaced by `treatment`, damaging all of `bounds`.
    ///
    /// A capture draws this in place of the frame, then reads the target back. Full damage is
    /// what makes the replacement cover the whole output, not only the part that changed.
    #[must_use]
    pub fn redacted(&self, treatment: SecureTreatment, bounds: Rect) -> Self {
        let mut frame = self.fully_damaged(bounds);
        frame.layers.clear();
        for layer in &self.layers {
            match (layer.is_secure(), treatment) {
                (false, _) => frame.layers.push(layer.clone()),
                (true, SecureTreatment::Mask) => frame.layers.push(Layer::Solid(layer.mask())),
                (true, SecureTreatment::Omit) => {}
            }
        }
        frame
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

    #[test]
    fn a_secure_layer_is_masked_in_a_capture_and_the_frame_keeps_it() {
        let bounds = Rect::new(0, 0, 64, 64);
        let mut frame = OutputFrame::new();
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 64, 64), Color::WHITE)));
        let mut secret = Solid::new(Rect::new(8, 8, 16, 16), Color::BLACK);
        secret.secure = true;
        frame.push_layer(Layer::Solid(secret));
        assert!(frame.has_secure());
        let masked = frame.redacted(SecureTreatment::Mask, bounds);
        assert_eq!(
            masked.layers().len(),
            2,
            "the mask keeps the layer's place in the stack"
        );
        match &masked.layers()[1] {
            Layer::Solid(mask) => assert_eq!(mask.dst, Rect::new(8, 8, 16, 16)),
            other => panic!("expected a solid mask, got {other:?}"),
        }
        assert_eq!(frame.layers().len(), 2, "the display's frame is not touched");
        assert!(matches!(&frame.layers()[1], Layer::Solid(solid) if solid.secure));
    }

    #[test]
    fn an_omitted_secure_layer_leaves_the_layers_beneath_it_visible() {
        let mut frame = OutputFrame::new();
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 64, 64), Color::WHITE)));
        let mut secret = Solid::new(Rect::new(0, 0, 64, 64), Color::BLACK);
        secret.secure = true;
        frame.push_layer(Layer::Solid(secret));
        let omitted = frame.redacted(SecureTreatment::Omit, Rect::new(0, 0, 64, 64));
        assert_eq!(
            omitted.layers().len(),
            1,
            "the secure layer is gone, the one beneath stays"
        );
        assert!(matches!(&omitted.layers()[0], Layer::Solid(solid) if !solid.secure));
    }

    #[test]
    fn a_redacted_frame_repaints_the_whole_output() {
        let bounds = Rect::new(0, 0, 64, 64);
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(4, 4, 2, 2));
        let mut secret = Solid::new(Rect::new(0, 0, 8, 8), Color::BLACK);
        secret.secure = true;
        frame.push_layer(Layer::Solid(secret));
        assert_eq!(frame.redacted(SecureTreatment::Mask, bounds).damage(), &[bounds]);
    }

    #[test]
    fn a_secure_overlay_is_masked_and_a_plain_one_is_not() {
        let mut frame = OutputFrame::new();
        frame.push_layer(Layer::Overlay(overlay_at(0).with_secure(true)));
        assert!(frame.has_secure());
        frame.clear();
        frame.push_layer(Layer::Overlay(overlay_at(0)));
        assert!(!frame.has_secure(), "an overlay is captured unless it says otherwise");
    }

    #[test]
    fn a_view_is_limited_to_the_zoom_range_and_finite_centres() {
        assert!(View::new(MIN_ZOOM, 0.0, 0.0).is_some(), "no magnification is allowed");
        assert!(View::new(MAX_ZOOM, 10.0, 10.0).is_some(), "the largest zoom is allowed");
        assert!(View::new(0.99, 0.0, 0.0).is_none(), "below one is not a magnification");
        assert!(View::new(16.01, 0.0, 0.0).is_none(), "above sixteen is not allowed");
        assert!(View::new(f32::NAN, 0.0, 0.0).is_none(), "NaN zoom fails the range test");
        assert!(
            View::new(2.0, f32::INFINITY, 0.0).is_none(),
            "an infinite centre is refused"
        );
    }

    #[test]
    fn a_view_is_set_and_cleared_with_the_frame() {
        let mut frame = OutputFrame::new();
        let view = View::new(2.0, 100.0, 50.0);
        frame.set_view(view);
        assert_eq!(frame.view(), view);
        frame.clear();
        assert_eq!(frame.view(), None, "clearing a frame drops its view");
    }

    #[test]
    fn an_invalid_view_is_dropped_and_the_frame_is_not_magnified() {
        let mut frame = OutputFrame::new();
        frame.set_view(Some(View {
            zoom: 40.0,
            centre_x: 0.0,
            centre_y: 0.0,
        }));
        assert_eq!(frame.view(), None, "an out-of-range zoom is never stored");
    }

    #[test]
    fn a_colour_matrix_is_stored_only_when_every_entry_is_finite() {
        let mut frame = OutputFrame::new();
        frame.set_colour_matrix(ColourMatrix::new(ColourMatrix::IDENTITY.rows));
        assert_eq!(frame.colour_matrix(), Some(ColourMatrix::IDENTITY));
        let mut bad = ColourMatrix::IDENTITY;
        bad.rows[2][4] = f32::NAN;
        frame.set_colour_matrix(Some(bad));
        assert_eq!(frame.colour_matrix(), None, "a NaN offset drops the matrix");
    }

    /// A one-pixel overlay, enough to occupy an entry in the list.
    fn overlay_at(x: i32) -> Overlay {
        let image = Image::new(gnoblin_types::Size { width: 1, height: 1 }, 4, vec![0, 0, 0, 255])
            .expect("a one-pixel image is valid");
        Overlay::new(Rect::new(x, 0, 1, 1), &image).expect("a non-empty overlay is valid")
    }

    /// The left edge of every solid layer, bottom first, so the test reads paint order.
    fn solid_order(frame: &OutputFrame) -> Vec<i32> {
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Solid(solid) => Some(solid.dst.x),
                Layer::Surface(_) | Layer::Overlay(_) => None,
            })
            .collect()
    }

    #[test]
    fn layers_are_painted_in_the_order_they_are_pushed() {
        let mut frame = OutputFrame::new();
        for x in [3, 1, 2] {
            frame.push_layer(Layer::Solid(Solid::new(Rect::new(x, 0, 1, 1), Color::WHITE)));
        }
        assert_eq!(
            solid_order(&frame),
            vec![3, 1, 2],
            "the list is bottom first, never sorted"
        );
    }

    #[test]
    fn an_overlay_pushed_last_is_the_top_entry_of_the_list() {
        let mut frame = OutputFrame::new();
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(0, 0, 1, 1), Color::WHITE)));
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(1, 0, 1, 1), Color::WHITE)));
        frame.push_layer(Layer::Overlay(overlay_at(5)));
        assert!(
            matches!(frame.layers().last(), Some(Layer::Overlay(overlay)) if overlay.dst() == Rect::new(5, 0, 1, 1)),
            "the overlay is the top entry of the one list"
        );
        assert_eq!(solid_order(&frame), vec![0, 1]);
    }
}
