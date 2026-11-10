//! Draws a subtree of a scene through a camera into an offscreen image.
//!
//! [`render_view`] is the primitive behind a View node, a capture and a headset
//! eye. It is CPU-only and deterministic: the same scene, camera and buffers give
//! byte-identical pixels, which is what makes it usable as a test oracle.
//!
//! # Coordinates
//!
//! The subtree's world is the root's plane: a point at the root's bounds
//! coordinates is at that position in the view's world, and every node below the
//! root is placed by the scene's own transforms relative to it. Ancestors above
//! the root are not drawn and do not move the view.
//!
//! The camera looks down negative z in its own space, as a scene camera does.
//! Scene y grows downwards, so a point below the centre has a larger eye `y`, and
//! an OpenXR `up` angle is a negative eye `y` in this space. A node is drawn at
//! pixel `(x, y)` when the pixel centre maps back into the node's bounds on its
//! own plane, at an eye depth between `near` and `far`.
//!
//! # Drawing rules
//!
//! - Nodes are drawn depth-first in tree order: a node first, then its children in
//!   order, so a later sibling is on top of an earlier one.
//! - A node draws the pixels its buffer supplies. A node with no buffer draws
//!   nothing, but its children still draw.
//! - An invisible node, a node whose composed opacity is zero, and a node whose
//!   projection is not finite or not invertible take their whole subtree with them.
//! - A node's opacity is multiplied by the opacity of every node from the root down
//!   to it, and blends its pixels over what is below, straight alpha.
//! - A node under a clipping ancestor is drawn only where the pixel centre lies in
//!   that ancestor's bounds, on the ancestor's own plane under the same camera.
//! - The image starts opaque black, like [`Image::blank`].
//!
//! # Status
//!
//! Not yet used by the compositor. This is the primitive for View nodes, captures
//! and headset eyes; the compositor and the XR extension call it when they land.
//! Only software rasterisation: no GPU and no DMA-BUF.

use std::sync::Arc;

use gnoblin_render::Scene;
use gnoblin_types::{NodeId, Rect, Size, Vec3};

use gnoblin_render_frame::image::Image;

/// A camera's pose and projection in the subtree's world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewCamera {
    /// Where the camera sits, in the view's world units (pixels for a flat scene).
    pub position: Vec3,
    /// The camera's rotation about each axis, in degrees, applied as `Rz · Ry · Rx`
    /// like a node's rotation.
    pub rotation_degrees: Vec3,
    /// How points are projected onto the image.
    pub projection: ViewProjection,
    /// The nearest eye depth drawn, greater than zero.
    pub near: f64,
    /// The farthest eye depth drawn, greater than `near`.
    pub far: f64,
}

/// How a [`ViewCamera`] projects its subtree onto the image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewProjection {
    /// One world unit is one pixel, and depth does not change a node's size.
    /// The camera's position is the image's centre.
    Orthographic,
    /// A symmetric perspective with this vertical field of view in degrees, strictly
    /// between 0 and 180. The horizontal field follows the image's aspect ratio.
    Perspective {
        /// The vertical field of view in degrees.
        vertical_fov_degrees: f64,
    },
    /// An off-axis perspective from four half-angles in radians, as OpenXR reports
    /// one eye's field of view. `left` is negative and `down` is negative for a
    /// symmetric view; any values with `left < right` and `down < up` are accepted.
    Fov {
        /// The left half-angle in radians.
        left: f64,
        /// The right half-angle in radians.
        right: f64,
        /// The upper half-angle in radians.
        up: f64,
        /// The lower half-angle in radians.
        down: f64,
    },
}

/// A rendered view: RGBA8, straight alpha, top left first.
#[derive(Clone, Debug)]
pub struct ViewImage {
    image: Image,
}

impl ViewImage {
    /// The image's size in pixels.
    #[must_use]
    pub fn size(&self) -> Size {
        self.image.size()
    }

    /// The bytes of one row, in RGBA8.
    #[must_use]
    pub fn stride(&self) -> usize {
        self.image.stride()
    }

    /// The pixels, row by row.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.image.data()
    }

    /// The colour of one pixel in RGBA8, or `None` outside the image.
    #[must_use]
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        self.image.colour_at(x, y)
    }

    /// The rendered image, for a compositor that draws it.
    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }

    /// The rendered image, shared, so the compositor can hold it as a snapshot.
    #[must_use]
    pub fn into_image(self) -> Arc<Image> {
        Arc::new(self.image)
    }
}

/// Draws the subtree rooted at `root` through `camera` into an image of `size`.
///
/// `buffers` supplies a node's pixels by its id, in RGBA8 straight alpha. A node the
/// lookup does not supply draws nothing. The image is scaled to the node's bounds,
/// so a buffer of any size covers the node exactly.
///
/// A missing root, a zero-sized image or a camera that cannot project draws a blank
/// image rather than failing: the caller always gets pixels of the size it asked for.
pub fn render_view<'a, F>(scene: &Scene, root: NodeId, camera: &ViewCamera, size: Size, buffers: F) -> ViewImage
where
    F: Fn(NodeId) -> Option<&'a Image>,
{
    let rows = projection_rows(camera.projection, size);
    let depths_valid =
        camera.near.is_finite() && camera.far.is_finite() && camera.near > 0.0 && camera.far > camera.near;
    let (Some(rows), true) = (rows, depths_valid) else {
        return ViewImage {
            image: Image::blank(size),
        };
    };
    if size.width == 0 || size.height == 0 {
        return ViewImage {
            image: Image::blank(size),
        };
    }
    let stride = size.width as usize * 4;
    let mut pixels = [0_u8, 0, 0, 255].repeat(size.width as usize * size.height as usize);
    let context = ViewContext {
        scene,
        root,
        eye: eye_matrix(camera),
        rows,
        near: camera.near,
        far: camera.far,
        size,
    };
    draw_node(&context, &buffers, root, 1.0, &mut Vec::new(), &mut pixels);
    let image = Image::new(size, stride, pixels).expect("the pixel buffer is sized for the image");
    ViewImage { image }
}

/// Shared state for one render: the camera, the projection and the output size.
struct ViewContext<'s> {
    scene: &'s Scene,
    root: NodeId,
    /// The world-to-eye matrix of the camera, row-major, homogeneous.
    eye: [[f64; 4]; 4],
    /// The eye-to-homogeneous-pixel rows `[X, Y, W]` over `(a, b, c, w)`.
    rows: [[f64; 4]; 3],
    near: f64,
    far: f64,
    size: Size,
}

/// A node's plane mapped to pixels, both ways.
#[derive(Clone, Copy)]
struct PlaneMap {
    /// The eye matrix of the node's plane, so `eye = eye_rows · (u, v, 0, 1)`.
    eye: [[f64; 4]; 4],
    /// Inverse of the plane-to-pixel homography: `(u, v, 1) ∝ inverse · (px, py, 1)`.
    inverse: [[f64; 3]; 3],
}

impl PlaneMap {
    /// The map for node `id`, or `None` when its projection is not finite or not
    /// invertible, which removes the node and its subtree.
    fn new(scene: &Scene, root: NodeId, id: NodeId, eye_matrix: &[[f64; 4]; 4], rows: &[[f64; 4]; 3]) -> Option<Self> {
        let world = scene.subtree_matrix(root, id)?.m;
        let eye = mul4(eye_matrix, &world);
        if !eye.iter().flatten().all(|value| value.is_finite()) {
            return None;
        }
        // The plane's z is zero, so the columns of the homography are u, v and 1.
        let mut forward = [[0.0; 3]; 3];
        for (k, row) in rows.iter().enumerate() {
            for (column, source) in [0, 1, 3].into_iter().enumerate() {
                forward[k][column] = (0..4).map(|j| row[j] * eye[j][source]).sum();
            }
        }
        let inverse = invert3(&forward)?;
        Some(Self { eye, inverse })
    }

    /// The plane point under the pixel centre `(x, y)`, or `None` when the pixel
    /// does not map onto the plane.
    fn locate(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let point = [x, y, 1.0];
        let dot = |row: &[f64; 3]| row[0] * point[0] + row[1] * point[1] + row[2] * point[2];
        let scale = dot(&self.inverse[2]);
        if !scale.is_finite() || scale == 0.0 {
            return None;
        }
        Some((dot(&self.inverse[0]) / scale, dot(&self.inverse[1]) / scale))
    }

    /// The eye depth of the plane point `(u, v)`: positive in front of the camera.
    fn depth(&self, u: f64, v: f64) -> Option<f64> {
        let at = |row: usize| self.eye[row][0] * u + self.eye[row][1] * v + self.eye[row][3];
        let w = at(3);
        let depth = -at(2) / w;
        (w != 0.0 && depth.is_finite()).then_some(depth)
    }
}

/// A clipping ancestor: its children draw only where the pixel centre lies in its
/// bounds on its own plane, in front of the camera.
#[derive(Clone, Copy)]
struct Clipper {
    plane: PlaneMap,
    bounds: Rect,
}

impl Clipper {
    fn admits(&self, x: f64, y: f64) -> bool {
        self.plane
            .locate(x, y)
            .is_some_and(|(u, v)| inside(self.bounds, u, v) && self.plane.depth(u, v).is_some_and(|depth| depth > 0.0))
    }
}

/// Draws `id` and then its children in order, so a later sibling paints over an
/// earlier one. `parent_opacity` is the product of the opacity above `id`.
fn draw_node<'a>(
    ctx: &ViewContext<'_>,
    buffers: &dyn Fn(NodeId) -> Option<&'a Image>,
    id: NodeId,
    parent_opacity: f64,
    clips: &mut Vec<Clipper>,
    pixels: &mut [u8],
) {
    if ctx.scene.visible(id) != Some(true) {
        return;
    }
    let Some(transform) = ctx.scene.transform(id) else {
        return;
    };
    let opacity = parent_opacity * f64::from(transform.opacity);
    if !opacity.is_finite() || opacity <= 0.0 {
        return;
    }
    let Some(plane) = PlaneMap::new(ctx.scene, ctx.root, id, &ctx.eye, &ctx.rows) else {
        return;
    };
    let Some(bounds) = ctx.scene.bounds(id) else {
        return;
    };
    if let Some(source) = buffers(id) {
        paint(ctx, &plane, bounds, source, opacity, clips, pixels);
    }
    let clipping = ctx.scene.clip(id) == Some(true);
    if clipping {
        clips.push(Clipper { plane, bounds });
    }
    for child in ctx.scene.children(id).unwrap_or(&[]) {
        draw_node(ctx, buffers, *child, opacity, clips, pixels);
    }
    if clipping {
        clips.pop();
    }
}

/// Blends a node's buffer over the output for every pixel centre that maps into its
/// bounds, in front of the camera and inside every clipping ancestor.
fn paint(
    ctx: &ViewContext<'_>,
    plane: &PlaneMap,
    bounds: Rect,
    source: &Image,
    opacity: f64,
    clips: &[Clipper],
    pixels: &mut [u8],
) {
    let source_size = source.size();
    if source_size.width == 0 || source_size.height == 0 || bounds.width == 0 || bounds.height == 0 {
        return;
    }
    let row = ctx.size.width as usize * 4;
    for y in 0..ctx.size.height {
        for x in 0..ctx.size.width {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let Some((u, v)) = plane.locate(px, py) else {
                continue;
            };
            if !inside(bounds, u, v) {
                continue;
            }
            if !plane
                .depth(u, v)
                .is_some_and(|depth| depth >= ctx.near && depth <= ctx.far)
            {
                continue;
            }
            if !clips.iter().all(|clip| clip.admits(px, py)) {
                continue;
            }
            let sx = texel(u, bounds.x, bounds.width, source_size.width);
            let sy = texel(v, bounds.y, bounds.height, source_size.height);
            let Some(colour) = source.colour_at(sx, sy) else {
                continue;
            };
            let alpha = f64::from(colour[3]) / 255.0 * opacity;
            let index = y as usize * row + x as usize * 4;
            for channel in 0..3 {
                let below = f64::from(pixels[index + channel]);
                let above = f64::from(colour[channel]);
                pixels[index + channel] = (above * alpha + below * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Whether a plane point lies in the half-open bounds of a rectangle.
fn inside(bounds: Rect, u: f64, v: f64) -> bool {
    let (left, top) = (f64::from(bounds.x), f64::from(bounds.y));
    u >= left && u < left + f64::from(bounds.width) && v >= top && v < top + f64::from(bounds.height)
}

/// The texel index of a plane coordinate across a rectangle, clamped to the buffer.
fn texel(coordinate: f64, origin: i32, extent: u32, texels: u32) -> i32 {
    let scaled = ((coordinate - f64::from(origin)) / f64::from(extent) * f64::from(texels)).floor();
    scaled.clamp(0.0, f64::from(texels - 1)) as i32
}

/// The world-to-eye matrix of a camera: the inverse of its pose, so a point at the
/// camera's position maps to the origin and the camera looks down negative z.
fn eye_matrix(camera: &ViewCamera) -> [[f64; 4]; 4] {
    let rotation = rotation_matrix(camera.rotation_degrees);
    let position = [camera.position.x, camera.position.y, camera.position.z];
    let mut eye = [[0.0; 4]; 4];
    for (i, row) in eye.iter_mut().take(3).enumerate() {
        // The inverse of a rotation is its transpose.
        for (j, cell) in row.iter_mut().take(3).enumerate() {
            *cell = rotation[j][i];
        }
        row[3] = -(0..3).map(|j| rotation[j][i] * position[j]).sum::<f64>();
    }
    eye[3][3] = 1.0;
    eye
}

/// The eye-to-pixel rows `[X, Y, W]` over the homogeneous eye point `(a, b, c, w)`,
/// where the eye point is `(a, b, c) / w`, the depth is `-c / w` and the pixel is
/// `(X, Y) / W`. `None` for a projection that cannot draw.
fn projection_rows(projection: ViewProjection, size: Size) -> Option<[[f64; 4]; 3]> {
    let width = f64::from(size.width);
    let height = f64::from(size.height);
    let rows = match projection {
        ViewProjection::Orthographic => [
            [1.0, 0.0, 0.0, width / 2.0],
            [0.0, 1.0, 0.0, height / 2.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        ViewProjection::Perspective { vertical_fov_degrees } => {
            if !(vertical_fov_degrees > 0.0 && vertical_fov_degrees < 180.0) {
                return None;
            }
            // Square pixels: one focal length for both axes.
            let focal = (height / 2.0) / (vertical_fov_degrees.to_radians() / 2.0).tan();
            [
                [focal, 0.0, -width / 2.0, 0.0],
                [0.0, focal, -height / 2.0, 0.0],
                [0.0, 0.0, -1.0, 0.0],
            ]
        }
        ViewProjection::Fov { left, right, up, down } => {
            let (tan_left, tan_right, tan_up, tan_down) = (left.tan(), right.tan(), up.tan(), down.tan());
            let (across, down_span) = (tan_right - tan_left, tan_up - tan_down);
            if !(across > 0.0 && down_span > 0.0 && across.is_finite() && down_span.is_finite()) {
                return None;
            }
            // The top edge is eye y = -tan(up) · depth, so `up` is a negative eye y.
            [
                [width / across, 0.0, width * tan_left / across, 0.0],
                [0.0, height / down_span, -height * tan_up / down_span, 0.0],
                [0.0, 0.0, -1.0, 0.0],
            ]
        }
    };
    Some(rows)
}

/// The camera rotation `Rz · Ry · Rx`, in degrees about each axis.
fn rotation_matrix(degrees: Vec3) -> [[f64; 3]; 3] {
    let (sin_x, cos_x) = degrees.x.to_radians().sin_cos();
    let (sin_y, cos_y) = degrees.y.to_radians().sin_cos();
    let (sin_z, cos_z) = degrees.z.to_radians().sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cos_x, -sin_x], [0.0, sin_x, cos_x]];
    let ry = [[cos_y, 0.0, sin_y], [0.0, 1.0, 0.0], [-sin_y, 0.0, cos_y]];
    let rz = [[cos_z, -sin_z, 0.0], [sin_z, cos_z, 0.0], [0.0, 0.0, 1.0]];
    mul3(&mul3(&rz, &ry), &rx)
}

fn mul3(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn mul4(a: &[[f64; 4]; 4], b: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// The inverse of a 3 by 3 matrix, or `None` when it is singular.
fn invert3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    let inverse = [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / det,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / det,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / det,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / det,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / det,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / det,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / det,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / det,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / det,
        ],
    ];
    inverse
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_render::NodeSpec;
    use gnoblin_types::{Rect, Vec3};
    use std::collections::HashMap;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn id(n: u64) -> NodeId {
        NodeId::new(n)
    }

    /// Insert a node with a rectangle, under `parent`.
    fn node(scene: &mut Scene, n: u64, parent: Option<u64>, bounds: Rect) -> NodeId {
        let mut spec = NodeSpec::new(bounds);
        spec.parent = parent.map(id);
        scene.insert(id(n), spec).expect("the node is inserted");
        id(n)
    }

    /// A solid-colour buffer of `width` by `height` pixels.
    fn solid(width: u32, height: u32, colour: [u8; 4]) -> Image {
        let data = colour.repeat((width * height) as usize);
        Image::new(Size::new(width, height), (width * 4) as usize, data).expect("a packed image")
    }

    /// An orthographic camera looking at the image centre of a `width` by `height`
    /// subtree whose world origin is at the origin: the camera sits at the centre,
    /// 100 units in front of the plane.
    fn ortho_centred(width: i32, height: i32) -> ViewCamera {
        ViewCamera {
            position: Vec3 {
                x: f64::from(width) / 2.0,
                y: f64::from(height) / 2.0,
                z: 100.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Orthographic,
            near: 0.1,
            far: 10_000.0,
        }
    }

    fn render(
        scene: &Scene,
        root: NodeId,
        camera: &ViewCamera,
        size: Size,
        buffers: &HashMap<NodeId, Image>,
    ) -> ViewImage {
        render_view(scene, root, camera, size, |n| buffers.get(&n))
    }

    fn pixel(image: &ViewImage, x: i32, y: i32) -> [u8; 4] {
        image.pixel(x, y).expect("the pixel is inside the image")
    }

    /// Whether each channel is within `tolerance` of `expected`.
    fn near(actual: [u8; 4], expected: [u8; 4], tolerance: i32) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (i32::from(*a) - i32::from(e)).abs() <= tolerance)
    }

    /// The bounding box of the pixels whose colour is `colour`, or `None`.
    fn bounding_box_of(image: &ViewImage, colour: [u8; 4]) -> Option<(i32, i32, i32, i32)> {
        let size = image.size();
        let mut found: Option<(i32, i32, i32, i32)> = None;
        for y in 0..size.height as i32 {
            for x in 0..size.width as i32 {
                if pixel(image, x, y) == colour {
                    found = Some(match found {
                        None => (x, y, x, y),
                        Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                    });
                }
            }
        }
        found
    }

    /// The centroid of the pixels whose colour is `colour`, as `(x, y)`.
    fn centroid_of(image: &ViewImage, colour: [u8; 4]) -> Option<(f64, f64)> {
        let size = image.size();
        let (mut sum_x, mut sum_y, mut count) = (0.0, 0.0, 0.0);
        for y in 0..size.height as i32 {
            for x in 0..size.width as i32 {
                if pixel(image, x, y) == colour {
                    sum_x += f64::from(x) + 0.5;
                    sum_y += f64::from(y) + 0.5;
                    count += 1.0;
                }
            }
        }
        (count > 0.0).then(|| (sum_x / count, sum_y / count))
    }

    /// FNV-1a over the pixel bytes: a stable fingerprint for replay comparisons.
    fn fingerprint(image: &ViewImage) -> u64 {
        image.data().iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn an_orthographic_node_lands_on_its_pixel_rectangle() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(10, 20, 100, 100));
        let buffers = HashMap::from([(id(2), solid(100, 100, RED))]);
        let camera = ortho_centred(200, 200);

        let image = render(&scene, root, &camera, Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), Some((10, 20, 109, 119)));
        assert_eq!(pixel(&image, 60, 70), RED, "the centre of the node is red");
        assert_eq!(pixel(&image, 9, 70), BLACK, "one pixel left of the node is not");
        assert_eq!(pixel(&image, 110, 70), BLACK, "one pixel right of the node is not");
        assert_eq!(pixel(&image, 60, 19), BLACK, "one pixel above the node is not");
        assert_eq!(pixel(&image, 60, 120), BLACK, "one pixel below the node is not");
    }

    #[test]
    fn a_rotated_node_under_perspective_lands_on_its_projected_corners() {
        // The node is 100 by 100 on the plane z = 0 and is turned 40 degrees about Y,
        // about its centre (50, 50). The camera sits at (50, 50, 300) and looks down
        // -z with a 60 degree vertical field, on a 200 by 200 image.
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let child = node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        let mut spec = *scene.transform(child).expect("the child has a transform");
        spec.base.rotation_degrees.y = 40.0;
        scene.set_transform(child, spec).expect("the transform is set");
        let buffers = HashMap::from([(child, solid(100, 100, RED))]);
        let camera = ViewCamera {
            position: Vec3 {
                x: 50.0,
                y: 50.0,
                z: 300.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Perspective {
                vertical_fov_degrees: 60.0,
            },
            near: 1.0,
            far: 10_000.0,
        };

        let image = render(&scene, root, &camera, Size::new(200, 200), &buffers);

        // The expected corners, from the same pinhole model the renderer must match.
        let world = scene.world_matrix(child).expect("the child has a world matrix");
        let focal = 100.0 / 30.0_f64.to_radians().tan();
        let projected: Vec<(f64, f64)> = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
            .into_iter()
            .map(|(x, y)| {
                let p = world.apply([x, y, 0.0, 1.0]);
                let (wx, wy, depth) = (p[0] / p[3], p[1] / p[3], 300.0 - p[2] / p[3]);
                (100.0 + focal * (wx - 50.0) / depth, 100.0 + focal * (wy - 50.0) / depth)
            })
            .collect();
        let min_x = projected.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let max_x = projected.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let min_y = projected.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let max_y = projected.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);

        let (x0, y0, x1, y1) = bounding_box_of(&image, RED).expect("the node is drawn");
        assert!((f64::from(x0) - min_x).abs() <= 1.0, "left edge {x0} vs {min_x}");
        assert!((f64::from(y0) - min_y).abs() <= 1.0, "top edge {y0} vs {min_y}");
        assert!(
            (f64::from(x1 + 1) - max_x).abs() <= 1.0,
            "right edge {} vs {max_x}",
            x1 + 1
        );
        assert!(
            (f64::from(y1 + 1) - max_y).abs() <= 1.0,
            "bottom edge {} vs {max_y}",
            y1 + 1
        );
        assert_eq!(pixel(&image, 100, 100), RED, "the centre of the node is red");
    }

    #[test]
    fn a_stereo_pair_of_off_axis_cameras_displaces_the_node_in_opposite_directions() {
        // A square 100 units wide sits at the centre of the plane, 300 units in front
        // of two eyes 64 units apart. Each eye has the same asymmetric (off-axis)
        // frustum, so the node lands right of centre for the left eye and left of centre
        // for the right eye.
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(50, 50, 100, 100));
        let buffers = HashMap::from([(id(2), solid(100, 100, RED))]);
        let eye = |x: f64| ViewCamera {
            position: Vec3 { x, y: 100.0, z: 300.0 },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Fov {
                left: -0.45,
                right: 0.35,
                up: 0.4,
                down: -0.4,
            },
            near: 1.0,
            far: 10_000.0,
        };
        let size = Size::new(200, 200);

        let left = render(&scene, root, &eye(100.0 - 32.0), size, &buffers);
        let right = render(&scene, root, &eye(100.0 + 32.0), size, &buffers);

        assert_ne!(left.data(), right.data(), "the two eyes see different images");
        let (left_x, left_y) = centroid_of(&left, RED).expect("the left eye draws the node");
        let (right_x, right_y) = centroid_of(&right, RED).expect("the right eye draws the node");

        // The node's centre (100, 100, 0) seen from an eye at x is at tan = (100 - x) / 300.
        let expected = |eye_x: f64| {
            let t = (100.0 - eye_x) / 300.0;
            let (tan_left, tan_right) = ((-0.45_f64).tan(), 0.35_f64.tan());
            200.0 * (t - tan_left) / (tan_right - tan_left)
        };
        assert!((left_x - expected(68.0)).abs() <= 2.0, "left eye centroid {left_x}");
        assert!((right_x - expected(132.0)).abs() <= 2.0, "right eye centroid {right_x}");
        assert!(left_x > 100.0, "the left eye sees the node right of centre");
        assert!(right_x < 100.0, "the right eye sees the node left of centre");
        assert!(
            (left_y - 100.0).abs() <= 1.0 && (right_y - 100.0).abs() <= 1.0,
            "the node stays level"
        );
    }

    #[test]
    fn a_later_sibling_paints_over_an_earlier_one() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        node(&mut scene, 3, Some(1), Rect::new(50, 50, 100, 100));
        let buffers = HashMap::from([(id(2), solid(100, 100, RED)), (id(3), solid(100, 100, BLUE))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(pixel(&image, 75, 75), BLUE, "the overlap shows the later sibling");
        assert_eq!(
            pixel(&image, 10, 10),
            RED,
            "the earlier sibling shows where it is alone"
        );
        assert_eq!(
            pixel(&image, 120, 120),
            BLUE,
            "the later sibling shows where it is alone"
        );
    }

    #[test]
    fn an_invisible_subtree_draws_nothing() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        node(&mut scene, 3, Some(2), Rect::new(0, 0, 100, 100));
        scene.set_visible(id(2), false).expect("the visibility is set");
        let buffers = HashMap::from([(id(2), solid(100, 100, RED)), (id(3), solid(100, 100, BLUE))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), None, "the hidden node is not drawn");
        assert_eq!(
            bounding_box_of(&image, BLUE),
            None,
            "nothing under the hidden node is drawn"
        );
    }

    #[test]
    fn a_node_with_zero_opacity_draws_nothing() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let child = node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        let mut spec = *scene.transform(child).expect("the child has a transform");
        spec.opacity = 0.0;
        scene.set_transform(child, spec).expect("the transform is set");
        let buffers = HashMap::from([(child, solid(100, 100, RED))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), None);
    }

    #[test]
    fn a_half_transparent_node_blends_fifty_percent_over_white() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        let red = node(&mut scene, 3, Some(1), Rect::new(0, 0, 100, 100));
        let mut spec = *scene.transform(red).expect("the child has a transform");
        spec.opacity = 0.5;
        scene.set_transform(red, spec).expect("the transform is set");
        let buffers = HashMap::from([(id(2), solid(100, 100, WHITE)), (red, solid(100, 100, RED))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        let blended = pixel(&image, 10, 10);
        assert!(
            near(blended, [255, 127, 127, 255], 1),
            "50% red over white, got {blended:?}"
        );
    }

    #[test]
    fn a_clipping_node_clips_its_children() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let clip = node(&mut scene, 2, Some(1), Rect::new(0, 0, 50, 50));
        scene.set_clip(clip, true).expect("the clip is set");
        node(&mut scene, 3, Some(2), Rect::new(0, 0, 100, 100));
        let buffers = HashMap::from([(id(3), solid(100, 100, RED))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(pixel(&image, 25, 25), RED, "inside the clip the child is drawn");
        assert_eq!(pixel(&image, 75, 25), BLACK, "outside the clip on x the child is not");
        assert_eq!(pixel(&image, 25, 75), BLACK, "outside the clip on y the child is not");
    }

    #[test]
    fn a_node_behind_the_camera_draws_nothing_and_does_not_panic() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let behind = node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        let mut spec = *scene.transform(behind).expect("the child has a transform");
        spec.base.offset.z = 400.0;
        scene.set_transform(behind, spec).expect("the transform is set");
        let buffers = HashMap::from([(behind, solid(100, 100, RED))]);
        let camera = ViewCamera {
            position: Vec3 {
                x: 50.0,
                y: 50.0,
                z: 300.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Perspective {
                vertical_fov_degrees: 60.0,
            },
            near: 1.0,
            far: 10_000.0,
        };

        let image = render(&scene, root, &camera, Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), None, "nothing behind the camera is drawn");
    }

    #[test]
    fn a_non_finite_transform_never_reaches_the_renderer() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let mut spec = NodeSpec::new(Rect::new(0, 0, 100, 100));
        spec.parent = Some(id(1));
        spec.transform.base.offset.x = f64::NAN;
        assert!(
            scene.insert(id(2), spec).is_err(),
            "the scene refuses a non-finite transform"
        );
        let buffers = HashMap::from([(id(2), solid(100, 100, RED))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), None);
    }

    #[test]
    fn only_the_subtree_under_the_root_is_drawn() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        node(&mut scene, 2, Some(1), Rect::new(0, 0, 50, 50));
        node(&mut scene, 3, None, Rect::new(100, 100, 100, 100));
        let buffers = HashMap::from([(id(2), solid(50, 50, RED)), (id(3), solid(100, 100, BLUE))]);

        let image = render(&scene, root, &ortho_centred(200, 200), Size::new(200, 200), &buffers);

        assert_eq!(bounding_box_of(&image, RED), Some((0, 0, 49, 49)));
        assert_eq!(
            bounding_box_of(&image, BLUE),
            None,
            "a node outside the subtree is not drawn"
        );
    }

    #[test]
    fn replaying_the_same_scene_gives_byte_identical_images() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        let tilted = node(&mut scene, 2, Some(1), Rect::new(0, 0, 100, 100));
        let mut spec = *scene.transform(tilted).expect("the child has a transform");
        spec.base.rotation_degrees.y = 25.0;
        scene.set_transform(tilted, spec).expect("the transform is set");
        node(&mut scene, 3, Some(1), Rect::new(60, 60, 100, 100));
        let buffers = HashMap::from([(tilted, solid(100, 100, RED)), (id(3), solid(100, 100, BLUE))]);
        let camera = ViewCamera {
            position: Vec3 {
                x: 50.0,
                y: 50.0,
                z: 300.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Perspective {
                vertical_fov_degrees: 60.0,
            },
            near: 1.0,
            far: 10_000.0,
        };

        let first = render(&scene, root, &camera, Size::new(200, 200), &buffers);
        let second = render(&scene, root, &camera, Size::new(200, 200), &buffers);

        assert_eq!(first.data(), second.data());
        assert_eq!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn the_image_and_its_shared_form_hold_the_same_pixels() {
        let mut scene = Scene::new();
        let root = node(&mut scene, 1, None, Rect::new(0, 0, 40, 40));
        let buffers = HashMap::from([(id(1), solid(40, 40, RED))]);
        let image = render(&scene, root, &ortho_centred(40, 40), Size::new(40, 40), &buffers);
        assert_eq!(image.image().size(), Size::new(40, 40));
        let shared = image.clone().into_image();
        assert_eq!(shared.data(), image.data());
        assert_eq!(Arc::strong_count(&shared), 1);
    }
}
