//! A headless CPU painter for egui output: it runs tessellated meshes into an
//! RGBA frame, so the portal dialogs can be checked without a GPU or a display.
//!
//! The painter keeps egui's textures (the font atlas arrives as texture deltas),
//! rasterises each triangle by pixel centre, samples the texture bilinearly,
//! modulates by the vertex colour and blends premultiplied-alpha over the frame.
//! It is not a general renderer. There is no anti-aliasing beyond egui's own
//! feathered mesh edges, no gamma handling (egui blends in sRGB space too), and
//! paint callbacks are skipped: no portal dialog emits one.

use std::collections::HashMap;

use egui::TexturesDelta;
use egui::epaint::{ClippedPrimitive, ImageData, ImageDelta, Primitive};
use egui::{Color32, Pos2, Rect, TextureId};

/// An opaque RGBA frame, row-major, eight bits per channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl Frame {
    /// A frame of `width` by `height` pixels, every pixel `color`.
    #[must_use]
    pub fn filled(width: usize, height: usize, color: [u8; 4]) -> Self {
        let rgba = color.iter().copied().cycle().take(width * height * 4).collect();
        Self { width, height, rgba }
    }

    /// The width in pixels.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// The height in pixels.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// The raw bytes, row-major, four per pixel.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.rgba
    }

    /// One pixel. Coordinates outside the frame are a caller bug and panic.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let index = (y * self.width + x) * 4;
        [
            self.rgba[index],
            self.rgba[index + 1],
            self.rgba[index + 2],
            self.rgba[index + 3],
        ]
    }

    /// The pixels inside `rect`, clamped to the frame, row by row.
    #[must_use]
    pub fn region(&self, rect: Rect) -> Vec<[u8; 4]> {
        let x0 = clamp_index(rect.min.x, self.width);
        let x1 = clamp_index(rect.max.x, self.width);
        let y0 = clamp_index(rect.min.y, self.height);
        let y1 = clamp_index(rect.max.y, self.height);
        let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            for x in x0..x1 {
                pixels.push(self.pixel(x, y));
            }
        }
        pixels
    }

    fn set(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let index = (y * self.width + x) * 4;
        self.rgba[index..index + 4].copy_from_slice(&color);
    }
}

/// Rounds a coordinate to a pixel index inside `[0, limit]`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped to [0, limit] before the cast"
)]
fn clamp_index(coordinate: f32, limit: usize) -> usize {
    let floored = coordinate.floor();
    if floored <= 0.0 {
        0
    } else if floored >= limit as f32 {
        limit
    } else {
        floored as usize
    }
}

/// One texture the painter holds: premultiplied texels, as egui stores them.
struct PaintImage {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
}

/// The textures egui has uploaded so far, keyed by id.
#[derive(Default)]
pub struct Textures {
    images: HashMap<TextureId, PaintImage>,
}

impl Textures {
    /// Applies the sets of a texture delta: whole images replace, partial ones
    /// copy a rectangle into the existing image.
    pub fn update(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for image_delta in deltas {
                self.set(*id, image_delta);
            }
        }
    }

    /// Drops the textures a delta frees. Call after painting the frame that used them.
    pub fn free(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.images.remove(id);
        }
    }

    fn set(&mut self, id: TextureId, delta: &ImageDelta) {
        // egui 0.37 has one image kind, RGBA. A partial update always follows a
        // full upload of the same id, so a missing entry is skipped, not invented.
        let ImageData::Color(color) = &delta.image;
        let [width, height] = color.size;
        match delta.pos {
            None => {
                self.images.insert(
                    id,
                    PaintImage {
                        width,
                        height,
                        pixels: color.pixels.clone(),
                    },
                );
            }
            Some([x0, y0]) => {
                let Some(image) = self.images.get_mut(&id) else {
                    return;
                };
                for row in 0..height {
                    let start = (y0 + row) * image.width + x0;
                    image.pixels[start..start + width].copy_from_slice(&color.pixels[row * width..(row + 1) * width]);
                }
            }
        }
    }

    fn get(&self, id: TextureId) -> Option<&PaintImage> {
        self.images.get(&id)
    }
}

/// Rasterises egui's clipped meshes onto `frame`. `pixels_per_point` converts
/// egui's point coordinates to frame pixels.
pub fn rasterise(frame: &mut Frame, textures: &Textures, primitives: &[ClippedPrimitive], pixels_per_point: f32) {
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        // A mesh whose texture was never uploaded has nothing to sample.
        let Some(texture) = textures.get(mesh.texture_id) else {
            continue;
        };
        let clip = Clip::new(primitive.clip_rect, pixels_per_point, frame);
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let vertices = [
                &mesh.vertices[triangle[0] as usize],
                &mesh.vertices[triangle[1] as usize],
                &mesh.vertices[triangle[2] as usize],
            ];
            let points = vertices.map(|vertex| to_pixels(vertex.pos, pixels_per_point));
            fill_triangle(
                frame,
                texture,
                &clip,
                points,
                vertices.map(|vertex| (vertex.uv, vertex.color)),
            );
        }
    }
}

/// The clip rectangle in frame pixels, intersected with the frame.
struct Clip {
    min: (f32, f32),
    max: (f32, f32),
}

impl Clip {
    fn new(rect: Rect, pixels_per_point: f32, frame: &Frame) -> Self {
        let min = to_pixels(rect.min, pixels_per_point);
        let max = to_pixels(rect.max, pixels_per_point);
        Self {
            min: (min.0.max(0.0), min.1.max(0.0)),
            max: (max.0.min(frame.width as f32), max.1.min(frame.height as f32)),
        }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.min.0 && x < self.max.0 && y >= self.min.1 && y < self.max.1
    }
}

fn to_pixels(point: Pos2, pixels_per_point: f32) -> (f32, f32) {
    (point.x * pixels_per_point, point.y * pixels_per_point)
}

/// Twice the signed area of the triangle `a b c`; positive on one winding.
fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn fill_triangle(
    frame: &mut Frame,
    texture: &PaintImage,
    clip: &Clip,
    points: [(f32, f32); 3],
    attributes: [(Pos2, Color32); 3],
) {
    let area = edge(points[0], points[1], points[2]);
    if area.abs() < f32::EPSILON {
        return;
    }
    let min_x = points.iter().map(|p| p.0).fold(f32::INFINITY, f32::min).max(clip.min.0);
    let min_y = points.iter().map(|p| p.1).fold(f32::INFINITY, f32::min).max(clip.min.1);
    let max_x = points
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(clip.max.0);
    let max_y = points
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(clip.max.1);
    let x0 = clamp_index(min_x, frame.width);
    let x1 = clamp_index(max_x.ceil(), frame.width);
    let y0 = clamp_index(min_y, frame.height);
    let y1 = clamp_index(max_y.ceil(), frame.height);

    for y in y0..y1 {
        for x in x0..x1 {
            let centre = (x as f32 + 0.5, y as f32 + 0.5);
            if !clip.contains(centre.0, centre.1) {
                continue;
            }
            let w0 = edge(points[1], points[2], centre) / area;
            let w1 = edge(points[2], points[0], centre) / area;
            let w2 = edge(points[0], points[1], centre) / area;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let uv = interpolate_uv(attributes, [w0, w1, w2]);
            let vertex = interpolate_color(attributes, [w0, w1, w2]);
            let texel = sample(texture, uv);
            let source = modulate(vertex, texel);
            let destination = frame_pixel(frame, x, y);
            frame.set(x, y, over(source, destination));
        }
    }
}

fn interpolate_uv(attributes: [(Pos2, Color32); 3], weights: [f32; 3]) -> (f32, f32) {
    let x = attributes[0].0.x * weights[0] + attributes[1].0.x * weights[1] + attributes[2].0.x * weights[2];
    let y = attributes[0].0.y * weights[0] + attributes[1].0.y * weights[1] + attributes[2].0.y * weights[2];
    (x, y)
}

/// The interpolated premultiplied colour, each channel in `[0, 1]`.
fn interpolate_color(attributes: [(Pos2, Color32); 3], weights: [f32; 3]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (channel, value) in out.iter_mut().enumerate() {
        *value = attributes
            .iter()
            .zip(weights)
            .map(|((_, color), weight)| f32::from(color.to_array()[channel]) / 255.0 * weight)
            .sum();
    }
    out
}

/// Bilinear sample of a premultiplied texture at `uv` (egui's texture space,
/// `[0, 1]` across the image), clamped to the edge. Channels in `[0, 1]`.
fn sample(texture: &PaintImage, uv: (f32, f32)) -> [f32; 4] {
    let x = uv.0 * texture.width as f32 - 0.5;
    let y = uv.1 * texture.height as f32 - 0.5;
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let texel = |dx: f32, dy: f32| -> [f32; 4] {
        let cx = clamp_index(x0 + dx, texture.width - 1);
        let cy = clamp_index(y0 + dy, texture.height - 1);
        let color = texture.pixels[cy * texture.width + cx].to_array();
        color.map(|channel| f32::from(channel) / 255.0)
    };
    let top_left = texel(0.0, 0.0);
    let top_right = texel(1.0, 0.0);
    let bottom_left = texel(0.0, 1.0);
    let bottom_right = texel(1.0, 1.0);
    let mut out = [0.0; 4];
    for channel in 0..4 {
        let top = top_left[channel] + (top_right[channel] - top_left[channel]) * fx;
        let bottom = bottom_left[channel] + (bottom_right[channel] - bottom_left[channel]) * fx;
        out[channel] = top + (bottom - top) * fy;
    }
    out
}

fn modulate(vertex: [f32; 4], texel: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for channel in 0..4 {
        out[channel] = vertex[channel] * texel[channel];
    }
    out
}

fn frame_pixel(frame: &Frame, x: usize, y: usize) -> [f32; 4] {
    frame.pixel(x, y).map(|channel| f32::from(channel) / 255.0)
}

/// Premultiplied source-over, then back to bytes.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "each channel is clamped to [0, 255] before the cast"
)]
fn over(source: [f32; 4], destination: [f32; 4]) -> [u8; 4] {
    let inverse = 1.0 - source[3];
    let mut out = [0_u8; 4];
    for channel in 0..4 {
        let value = source[channel] + destination[channel] * inverse;
        out[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    out
}
