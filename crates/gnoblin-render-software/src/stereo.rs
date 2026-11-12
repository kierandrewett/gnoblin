//! Off-axis frustums and the stereo rig: two eyes drawn side by side into one picture.
//!
//! A stereo [`ViewRig`] is how a View with a `stereo` camera is drawn. The camera node's pose is the point between the
//! eyes. The left eye sits half the eye separation to the camera's left and the right eye half to its right, along the
//! camera's own x axis, so turning the camera turns the pair. Each eye looks the way the camera looks.
//!
//! # The picture
//!
//! The left eye fills the left half of the picture and the right eye the right half. An odd width gives the spare
//! column to the right eye, the split the extension's projection layer reads back. Each eye has its own aspect ratio:
//! its half of the width over the whole height.
//!
//! # The frustum
//!
//! A [`Frustum`] is four angles in radians, as OpenXR states a field of view: `left` and `down` are negative for a
//! symmetric view, and a point on the frustum's left edge lands on the picture's left edge. An off-axis frustum is one
//! whose angles are not symmetric, which is what a headset's eye has: the nose side sees less than the temple side.
//! The rig's frustum is the left eye's. The right eye's is its mirror, so a rig describes both eyes with four angles.

use gnoblin_render::Scene;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{NodeId, Size, Vec3};

use crate::view::{ViewCamera, ViewImage, ViewProjection, render_view, rotation_matrix};

/// An asymmetric view frustum, in radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    /// The left angle, negative for a symmetric view.
    pub left: f64,
    /// The right angle.
    pub right: f64,
    /// The upper angle.
    pub up: f64,
    /// The lower angle, negative for a symmetric view.
    pub down: f64,
}

impl Frustum {
    /// A frustum from four angles in degrees, or `None` when it is not one a picture can show: an angle that is not
    /// finite or has a magnitude of 90 degrees or more, a `left` that is not below `right`, or a `down` that is not
    /// below `up`.
    #[must_use]
    pub fn from_degrees(left: f64, right: f64, up: f64, down: f64) -> Option<Self> {
        let frustum = Self {
            left: left.to_radians(),
            right: right.to_radians(),
            up: up.to_radians(),
            down: down.to_radians(),
        };
        frustum.is_usable().then_some(frustum)
    }

    /// The symmetric frustum with this vertical field of view in degrees, for a picture with this aspect ratio
    /// (width over height), or `None` when the field of view is not strictly between 0 and 180 degrees or the aspect is
    /// not finite and positive.
    #[must_use]
    pub fn symmetric(vertical_fov_degrees: f64, aspect: f64) -> Option<Self> {
        if !(vertical_fov_degrees > 0.0 && vertical_fov_degrees < 180.0 && aspect.is_finite() && aspect > 0.0) {
            return None;
        }
        let half_vertical = (vertical_fov_degrees.to_radians() / 2.0).tan();
        let half_horizontal = (aspect * half_vertical).atan();
        let half_vertical = half_vertical.atan();
        let frustum = Self {
            left: -half_horizontal,
            right: half_horizontal,
            up: half_vertical,
            down: -half_vertical,
        };
        frustum.is_usable().then_some(frustum)
    }

    /// The frustum seen in a mirror: the other eye's, when this is one eye's.
    #[must_use]
    pub fn mirrored(self) -> Self {
        Self {
            left: -self.right,
            right: -self.left,
            ..self
        }
    }

    /// The projection the renderer draws this frustum with.
    #[must_use]
    pub fn projection(self) -> ViewProjection {
        ViewProjection::Fov {
            left: self.left,
            right: self.right,
            up: self.up,
            down: self.down,
        }
    }

    fn is_usable(self) -> bool {
        let limit = std::f64::consts::FRAC_PI_2;
        [self.left, self.right, self.up, self.down]
            .iter()
            .all(|angle| angle.is_finite() && angle.abs() < limit)
            && self.left < self.right
            && self.down < self.up
    }
}

/// Two eyes as one camera: a pose, the distance between the eyes, and the left eye's frustum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StereoRig {
    /// The point between the eyes, in the view's world units.
    pub position: Vec3,
    /// The rig's rotation about each axis in degrees, applied as a node's rotation is.
    pub rotation_degrees: Vec3,
    /// The distance between the eyes in the view's world units, zero or more.
    pub eye_separation: f64,
    /// The left eye's frustum. The right eye's is its mirror.
    pub left: Frustum,
    /// The nearest eye depth drawn, greater than zero.
    pub near: f64,
    /// The farthest eye depth drawn, greater than `near`.
    pub far: f64,
}

impl StereoRig {
    /// The two eyes, left first. `None` when the eye separation is not finite and zero or more.
    #[must_use]
    pub fn eyes(&self) -> Option<[ViewCamera; 2]> {
        if !(self.eye_separation.is_finite() && self.eye_separation >= 0.0) {
            return None;
        }
        // The camera's own x axis in the world: the first column of its rotation.
        let rotation = rotation_matrix(self.rotation_degrees);
        let half = self.eye_separation / 2.0;
        let offset = Vec3 {
            x: rotation[0][0] * half,
            y: rotation[1][0] * half,
            z: rotation[2][0] * half,
        };
        let eye = |sign: f64, frustum: Frustum| ViewCamera {
            position: Vec3 {
                x: self.position.x + sign * offset.x,
                y: self.position.y + sign * offset.y,
                z: self.position.z + sign * offset.z,
            },
            rotation_degrees: self.rotation_degrees,
            projection: frustum.projection(),
            near: self.near,
            far: self.far,
        };
        Some([eye(-1.0, self.left), eye(1.0, self.left.mirrored())])
    }
}

/// How a View is drawn: through one camera, through a stereo pair, or not at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewRig {
    /// One camera fills the picture.
    Mono(ViewCamera),
    /// Two eyes, side by side.
    Stereo(StereoRig),
    /// A camera that cannot project: an unusable frustum or eye separation. The picture is blank.
    Blank,
}

impl ViewRig {
    /// The cameras the rig draws through and the rectangle of a picture of `size` each one fills, left first. A mono
    /// rig is one camera over the whole picture; a stereo rig is two, each over its half; a blank rig, a picture too
    /// narrow to give each eye a column and an eye separation that is not usable have none, which is the blank
    /// picture [`render_rig`] draws. A GPU renderer draws from this list, so it cannot place an eye anywhere the CPU
    /// rig does not.
    #[must_use]
    pub fn eyes(&self, size: Size) -> Vec<(gnoblin_types::Rect, ViewCamera)> {
        match self {
            Self::Mono(camera) => vec![(gnoblin_types::Rect::from_size(size), *camera)],
            Self::Blank => Vec::new(),
            Self::Stereo(stereo) => {
                let [left_size, right_size] = eye_sizes(size);
                match (stereo.eyes(), left_size.width > 0) {
                    (Some([left, right]), true) => vec![
                        (gnoblin_types::Rect::from_size(left_size), left),
                        (
                            gnoblin_types::Rect::new(left_size.width as i32, 0, right_size.width, right_size.height),
                            right,
                        ),
                    ],
                    _ => Vec::new(),
                }
            }
        }
    }
}

/// The sizes of the two halves of a stereo picture of `size`, left first. The spare column of an odd width belongs to
/// the right eye.
#[must_use]
pub fn eye_sizes(size: Size) -> [Size; 2] {
    let left = size.width / 2;
    [Size::new(left, size.height), Size::new(size.width - left, size.height)]
}

/// Draws the subtree rooted at `root` through `rig` into a picture of `size`.
///
/// A [`ViewRig::Mono`] is [`render_view`]. A [`ViewRig::Stereo`] draws each eye into its half of the picture. A
/// [`ViewRig::Blank`], a picture too narrow to give each eye a column, and an eye separation that is not usable all
/// give a blank picture of `size`, so the caller always gets the size it asked for.
pub fn render_rig<'a, F>(scene: &Scene, root: NodeId, rig: &ViewRig, size: Size, buffers: F) -> ViewImage
where
    F: Fn(NodeId) -> Option<&'a Image>,
{
    match rig {
        ViewRig::Mono(camera) => render_view(scene, root, camera, size, buffers),
        ViewRig::Blank => ViewImage::from_image(Image::blank(size)),
        ViewRig::Stereo(stereo) => {
            let [left_size, right_size] = eye_sizes(size);
            let eyes = stereo.eyes();
            let (Some([left_eye, right_eye]), true) = (eyes, left_size.width > 0) else {
                return ViewImage::from_image(Image::blank(size));
            };
            let left = render_view(scene, root, &left_eye, left_size, &buffers);
            let right = render_view(scene, root, &right_eye, right_size, &buffers);
            join(size, &left, &right)
        }
    }
}

/// The two halves side by side, row by row.
fn join(size: Size, left: &ViewImage, right: &ViewImage) -> ViewImage {
    let stride = size.width as usize * 4;
    let (left_row, right_row) = (left.size().width as usize * 4, right.size().width as usize * 4);
    let mut pixels = Vec::with_capacity(stride * size.height as usize);
    for row in 0..size.height as usize {
        pixels.extend_from_slice(&left.data()[row * left.stride()..][..left_row]);
        pixels.extend_from_slice(&right.data()[row * right.stride()..][..right_row]);
    }
    let image = Image::new(size, stride, pixels).expect("the joined pixels are sized for the image");
    ViewImage::from_image(image)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use gnoblin_render::NodeSpec;
    use gnoblin_types::Rect;

    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn id(n: u64) -> NodeId {
        NodeId::new(n)
    }

    fn insert(scene: &mut Scene, n: u64, parent: Option<u64>, bounds: Rect) {
        let mut spec = NodeSpec::new(bounds);
        spec.parent = parent.map(id);
        scene.insert(id(n), spec).expect("the node is inserted");
    }

    fn solid(width: u32, height: u32, colour: [u8; 4]) -> Image {
        let data = colour.repeat((width * height) as usize);
        Image::new(Size::new(width, height), (width * 4) as usize, data).expect("a packed image")
    }

    /// The centroid of the pixels of `colour` in columns `from..to`, as `(x, y)` in picture coordinates.
    fn centroid(image: &ViewImage, colour: [u8; 4], from: i32, to: i32) -> Option<(f64, f64)> {
        let (mut sum_x, mut sum_y, mut count) = (0.0, 0.0, 0.0);
        for y in 0..image.size().height as i32 {
            for x in from..to {
                if image.pixel(x, y) == Some(colour) {
                    sum_x += f64::from(x) + 0.5;
                    sum_y += f64::from(y) + 0.5;
                    count += 1.0;
                }
            }
        }
        (count > 0.0).then(|| (sum_x / count, sum_y / count))
    }

    fn rig(frustum: Frustum, separation: f64) -> StereoRig {
        StereoRig {
            position: Vec3 {
                x: 100.0,
                y: 100.0,
                z: 300.0,
            },
            rotation_degrees: Vec3::ZERO,
            eye_separation: separation,
            left: frustum,
            near: 1.0,
            far: 10_000.0,
        }
    }

    fn symmetric() -> Frustum {
        Frustum::from_degrees(-30.0, 30.0, 30.0, -30.0).expect("a usable frustum")
    }

    #[test]
    fn a_symmetric_field_of_view_is_four_angles_that_match_its_aspect() {
        let frustum = Frustum::symmetric(90.0, 2.0).expect("a usable frustum");
        // Half the vertical field is 45 degrees, so tan is 1; the horizontal half-tangent is 2.
        assert!((frustum.up.tan() - 1.0).abs() < 1e-12);
        assert!((frustum.down.tan() + 1.0).abs() < 1e-12);
        assert!((frustum.right.tan() - 2.0).abs() < 1e-12);
        assert!((frustum.left.tan() + 2.0).abs() < 1e-12);
    }

    #[test]
    fn an_unusable_frustum_is_refused() {
        for (l, r, u, d) in [
            (30.0, -30.0, 30.0, -30.0),
            (-30.0, 30.0, -30.0, 30.0),
            (-30.0, -30.0, 30.0, -30.0),
            (-90.0, 30.0, 30.0, -30.0),
            (-30.0, 30.0, 95.0, -30.0),
            (f64::NAN, 30.0, 30.0, -30.0),
            (-30.0, f64::INFINITY, 30.0, -30.0),
        ] {
            assert_eq!(Frustum::from_degrees(l, r, u, d), None, "{l} {r} {u} {d}");
        }
        for (fov, aspect) in [
            (0.0, 1.0),
            (180.0, 1.0),
            (-5.0, 1.0),
            (60.0, 0.0),
            (60.0, f64::NAN),
            (f64::NAN, 1.0),
        ] {
            assert_eq!(Frustum::symmetric(fov, aspect), None, "{fov} {aspect}");
        }
    }

    #[test]
    fn the_mirror_of_an_off_axis_frustum_swaps_and_negates_its_sides() {
        let left = Frustum::from_degrees(-52.0, 42.0, 43.0, -47.0).expect("a usable frustum");
        let right = left.mirrored();
        assert_eq!(
            (right.left, right.right, right.up, right.down),
            ((-42.0_f64).to_radians(), 52.0_f64.to_radians(), left.up, left.down)
        );
        assert_eq!(right.mirrored(), left);
    }

    #[test]
    fn the_eyes_sit_half_the_separation_either_side_along_the_cameras_x_axis() {
        let [left, right] = rig(symmetric(), 64.0).eyes().expect("two eyes");
        assert_eq!(
            (left.position.x, left.position.y, left.position.z),
            (68.0, 100.0, 300.0)
        );
        assert_eq!(
            (right.position.x, right.position.y, right.position.z),
            (132.0, 100.0, 300.0)
        );
        assert_eq!(left.rotation_degrees, Vec3::ZERO);
    }

    #[test]
    fn turning_the_rig_turns_the_eye_pair() {
        // A quarter turn about y takes the camera's x axis to the world's -z: the right eye is nearer the node.
        let mut turned = rig(symmetric(), 64.0);
        turned.rotation_degrees = Vec3 {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        };
        let [left, right] = turned.eyes().expect("two eyes");
        assert!((left.position.x - 100.0).abs() < 1e-9 && (right.position.x - 100.0).abs() < 1e-9);
        assert!((left.position.z - 332.0).abs() < 1e-9, "left z {}", left.position.z);
        assert!((right.position.z - 268.0).abs() < 1e-9, "right z {}", right.position.z);
        assert_eq!(left.rotation_degrees, turned.rotation_degrees);
    }

    #[test]
    fn a_zero_separation_puts_both_eyes_at_the_camera_and_a_bad_one_gives_no_eyes() {
        let [left, right] = rig(symmetric(), 0.0).eyes().expect("two eyes");
        assert_eq!(left.position, right.position);
        for separation in [-1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(rig(symmetric(), separation).eyes(), None, "{separation}");
        }
    }

    #[test]
    fn the_right_eye_uses_the_mirror_of_the_left_eyes_frustum() {
        let off_axis = Frustum::from_degrees(-40.0, 25.0, 30.0, -35.0).expect("a usable frustum");
        let [left, right] = rig(off_axis, 64.0).eyes().expect("two eyes");
        assert_eq!(left.projection, off_axis.projection());
        assert_eq!(right.projection, off_axis.mirrored().projection());
    }

    /// A 100-unit red square centred at (100, 100) on the root's plane, in a 200 by 200 world.
    fn scene_with_a_square() -> (Scene, HashMap<NodeId, Image>) {
        let mut scene = Scene::new();
        insert(&mut scene, 1, None, Rect::new(0, 0, 200, 200));
        insert(&mut scene, 2, Some(1), Rect::new(50, 50, 100, 100));
        (scene, HashMap::from([(id(2), solid(100, 100, RED))]))
    }

    #[test]
    fn a_stereo_picture_has_the_left_eye_on_the_left_and_the_right_eye_on_the_right() {
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(400, 200);
        let stereo = ViewRig::Stereo(rig(symmetric(), 64.0));

        let image = render_rig(&scene, id(1), &stereo, size, |n| buffers.get(&n));

        assert_eq!(image.size(), size);
        let (left_x, left_y) = centroid(&image, RED, 0, 200).expect("the left eye draws the square");
        let (right_x, right_y) = centroid(&image, RED, 200, 400).expect("the right eye draws the square");
        // The square's centre is at x = 100. The left eye sits at x = 68, so it sees the square right of its axis.
        assert!(left_x > 100.0, "left eye centroid {left_x}");
        assert!(right_x - 200.0 < 100.0, "right eye centroid {}", right_x - 200.0);
        assert!((left_y - 100.0).abs() <= 1.0 && (right_y - 100.0).abs() <= 1.0);
    }

    #[test]
    fn the_disparity_between_the_eyes_follows_the_separation_and_the_depth() {
        // With a symmetric frustum of tangent t over an eye of width w, the focal length is w / (2t) pixels. A node at
        // depth d seen from two eyes s apart lands f * s / d pixels apart.
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(400, 200);
        let frustum = symmetric();
        let focal = 200.0 / (2.0 * frustum.right.tan());
        for separation in [32.0, 64.0, 90.0] {
            let stereo = ViewRig::Stereo(rig(frustum, separation));
            let image = render_rig(&scene, id(1), &stereo, size, |n| buffers.get(&n));
            let (left_x, _) = centroid(&image, RED, 0, 200).expect("left eye");
            let (right_x, _) = centroid(&image, RED, 200, 400).expect("right eye");
            let disparity = left_x - (right_x - 200.0);
            let expected = focal * separation / 300.0;
            assert!(
                (disparity - expected).abs() <= 2.0,
                "separation {separation}: disparity {disparity}, expected {expected}"
            );
        }
    }

    #[test]
    fn an_off_axis_frustum_shifts_the_image_without_changing_its_scale() {
        // The node's centre is straight ahead of the camera. A frustum that reaches further left than right puts the
        // centre right of the picture's middle, by the lens shift (tan_left + tan_right) / 2 over the span.
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(200, 200);
        let frustum = Frustum::from_degrees(-40.0, 20.0, 25.0, -25.0).expect("a usable frustum");
        let camera = ViewCamera {
            position: Vec3 {
                x: 100.0,
                y: 100.0,
                z: 300.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: frustum.projection(),
            near: 1.0,
            far: 10_000.0,
        };
        let image = render_view(&scene, id(1), &camera, size, |n| buffers.get(&n));
        let (x, y) = centroid(&image, RED, 0, 200).expect("the node is drawn");

        let (tan_left, tan_right) = (frustum.left.tan(), frustum.right.tan());
        let expected_x = 200.0 * (0.0 - tan_left) / (tan_right - tan_left);
        assert!((x - expected_x).abs() <= 1.5, "centroid {x}, expected {expected_x}");
        assert!((y - 100.0).abs() <= 1.0);
        // The square is 100 units wide at 300 units: 100 / 300 of a tangent, over the horizontal span.
        let width = 100.0 / 300.0 / (tan_right - tan_left) * 200.0;
        let (mut min, mut max) = (i32::MAX, i32::MIN);
        for column in 0..200 {
            if (0..200).any(|row| image.pixel(column, row) == Some(RED)) {
                min = min.min(column);
                max = max.max(column);
            }
        }
        assert!(
            (f64::from(max - min + 1) - width).abs() <= 2.0,
            "width {} vs {width}",
            max - min + 1
        );
    }

    #[test]
    fn the_frustum_edges_land_on_the_picture_edges() {
        // A wide red plane at the root: its edge pixels are the picture's edge pixels only when the frustum edges
        // are exactly the angles stated.
        let mut scene = Scene::new();
        insert(&mut scene, 1, None, Rect::new(-10_000, -10_000, 20_000, 20_000));
        let buffers = HashMap::from([(id(1), solid(20_000 / 100, 20_000 / 100, RED))]);
        let size = Size::new(64, 48);
        let frustum = Frustum::from_degrees(-60.0, 10.0, 20.0, -35.0).expect("a usable frustum");
        let camera = ViewCamera {
            position: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: frustum.projection(),
            near: 1.0,
            far: 10_000.0,
        };
        let image = render_view(&scene, id(1), &camera, size, |n| buffers.get(&n));
        for (x, y) in [(0, 0), (63, 0), (0, 47), (63, 47)] {
            assert_eq!(image.pixel(x, y), Some(RED), "corner {x},{y}");
        }
    }

    #[test]
    fn an_odd_width_gives_the_spare_column_to_the_right_eye() {
        assert_eq!(
            eye_sizes(Size::new(401, 200)),
            [Size::new(200, 200), Size::new(201, 200)]
        );
        assert_eq!(
            eye_sizes(Size::new(400, 200)),
            [Size::new(200, 200), Size::new(200, 200)]
        );
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(401, 200);
        let stereo = ViewRig::Stereo(rig(symmetric(), 64.0));
        let image = render_rig(&scene, id(1), &stereo, size, |n| buffers.get(&n));
        assert_eq!(image.size(), size);
        assert!(centroid(&image, RED, 200, 401).is_some());
    }

    #[test]
    fn a_rig_that_cannot_project_draws_a_blank_picture_of_the_size_asked_for() {
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(300, 150);
        let bad_separation = ViewRig::Stereo(rig(symmetric(), -5.0));
        let too_narrow = ViewRig::Stereo(rig(symmetric(), 64.0));
        for (rig, size) in [
            (ViewRig::Blank, size),
            (bad_separation, size),
            (too_narrow, Size::new(1, 150)),
            (too_narrow, Size::new(0, 0)),
        ] {
            let image = render_rig(&scene, id(1), &rig, size, |n| buffers.get(&n));
            assert_eq!(image.size(), size);
            assert!(centroid(&image, RED, 0, i32::try_from(size.width).unwrap_or(0)).is_none());
            if size.width > 0 {
                assert_eq!(image.pixel(0, 0), Some(BLACK));
            }
        }
    }

    #[test]
    fn a_mono_rig_is_the_plain_view_render() {
        let (scene, buffers) = scene_with_a_square();
        let size = Size::new(200, 200);
        let camera = ViewCamera {
            position: Vec3 {
                x: 100.0,
                y: 100.0,
                z: 100.0,
            },
            rotation_degrees: Vec3::ZERO,
            projection: ViewProjection::Orthographic,
            near: 0.1,
            far: 10_000.0,
        };
        let plain = render_view(&scene, id(1), &camera, size, |n| buffers.get(&n));
        let through_rig = render_rig(&scene, id(1), &ViewRig::Mono(camera), size, |n| buffers.get(&n));
        assert_eq!(plain.data(), through_rig.data());
    }
}
