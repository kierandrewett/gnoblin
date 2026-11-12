//! What one redraw of a View costs on the CPU path.
//!
//! A View is drawn by `render_view` whenever its source changes, at most `fps` times a second. The owner kept the CPU
//! path (decision O8, option (b)) until a profile says otherwise, so this is the profile: the time of one redraw for
//! the View sizes and sources a shell is likely to make. It is a measurement, not a check, so it is ignored by default;
//! its numbers are in `design/cutover-survey.md` (section 16.2) and `docs/shell-api/views.md`.
//!
//! ```text
//! cargo test --release -p gnoblin-render-software --test view_cost -- --ignored --nocapture
//! ```
//!
//! Run it in release: a debug build is more than ten times slower and says nothing about the compositor. The table
//! prints the median of [`RUNS`] redraws after [`WARM_UP`] more that are not counted, in milliseconds.

use std::time::{Duration, Instant};

use gnoblin_render::{NodeSpec, Scene};
use gnoblin_render_frame::image::Image;
use gnoblin_render_software::{ViewCamera, ViewProjection, render_view};
use gnoblin_types::{NodeId, Rect, Size, Vec3};

const WARM_UP: usize = 3;
const RUNS: usize = 15;

/// The View sizes measured, in pixels.
const SIZES: [(u32, u32); 5] = [(320, 200), (640, 360), (1280, 720), (1920, 1080), (3840, 2160)];

fn id(raw: u64) -> NodeId {
    NodeId::new(raw)
}

fn picture(width: u32, height: u32, colour: [u8; 4]) -> Image {
    let data = colour.repeat((width * height) as usize);
    Image::new(Size::new(width, height), (width * 4) as usize, data).expect("a packed image")
}

/// A source subtree of `windows` nodes under one root of `size`. Each node is `size / divisor` wide and high, at its
/// own offset, with a picture of its own, at opacity `opacity`. Returns the scene and the pictures by node.
fn scene_of(size: Size, windows: u64, opacity: f32, divisor: u32) -> (Scene, Vec<(NodeId, Image)>) {
    let mut scene = Scene::new();
    scene
        .insert(id(1), NodeSpec::new(Rect::new(0, 0, size.width, size.height)))
        .expect("the root");
    let (width, height) = (size.width / divisor, size.height / divisor);
    let mut pictures = Vec::new();
    for index in 0..windows {
        let step = u32::try_from(index).expect("a small count");
        let x = i32::try_from((step * 37) % (size.width - width + 1)).expect("fits");
        let y = i32::try_from((step * 53) % (size.height - height + 1)).expect("fits");
        let mut spec = NodeSpec::new(Rect::new(x, y, width, height));
        spec.parent = Some(id(1));
        spec.transform.opacity = opacity;
        let node = id(2 + index);
        scene.insert(node, spec).expect("a window node");
        let colour = [40 + (step * 20 % 200) as u8, 90, 160, 255];
        pictures.push((node, picture(width, height, colour)));
    }
    (scene, pictures)
}

fn orthographic(size: Size) -> ViewCamera {
    ViewCamera {
        position: Vec3::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0, 100.0),
        rotation_degrees: Vec3::ZERO,
        projection: ViewProjection::Orthographic,
        near: 0.1,
        far: 10_000.0,
    }
}

fn perspective(size: Size) -> ViewCamera {
    let (centre_x, centre_y) = (f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    ViewCamera {
        position: Vec3::new(centre_x, centre_y, f64::from(size.height)),
        rotation_degrees: Vec3::ZERO,
        projection: ViewProjection::Perspective {
            vertical_fov_degrees: 53.0,
        },
        near: 1.0,
        far: 100_000.0,
    }
}

/// The median time of one redraw, in milliseconds.
fn median_ms(scene: &Scene, pictures: &[(NodeId, Image)], camera: &ViewCamera, size: Size) -> f64 {
    let lookup = |node: NodeId| {
        pictures
            .iter()
            .find(|(owner, _)| *owner == node)
            .map(|(_, image)| image)
    };
    let mut times: Vec<Duration> = Vec::with_capacity(RUNS);
    for run in 0..WARM_UP + RUNS {
        let started = Instant::now();
        let image = render_view(scene, id(1), camera, size, lookup);
        let elapsed = started.elapsed();
        // Keep the picture alive past the clock, so a redraw nobody reads is not optimised away.
        assert_eq!(image.size(), size);
        if run >= WARM_UP {
            times.push(elapsed);
        }
    }
    times.sort_unstable();
    times[times.len() / 2].as_secs_f64() * 1000.0
}

#[test]
#[ignore = "a measurement, not a check: run it in release with --ignored --nocapture"]
fn the_cost_of_one_view_redraw_on_the_cpu_path() {
    println!("| View size | 1 window, full size | 1 window, quarter area | 10 windows, 90% | 1 window, perspective |");
    println!("| --------- | ------------------- | ---------------------- | --------------- | --------------------- |");
    for (width, height) in SIZES {
        let size = Size::new(width, height);
        let (full, full_pictures) = scene_of(size, 1, 1.0, 1);
        let (quarter, quarter_pictures) = scene_of(size, 1, 1.0, 2);
        let (ten, ten_pictures) = scene_of(size, 10, 0.9, 2);
        let flat_full = median_ms(&full, &full_pictures, &orthographic(size), size);
        let flat_quarter = median_ms(&quarter, &quarter_pictures, &orthographic(size), size);
        let flat_ten = median_ms(&ten, &ten_pictures, &orthographic(size), size);
        let deep_quarter = median_ms(&quarter, &quarter_pictures, &perspective(size), size);
        println!(
            "| {width}x{height} | {flat_full:.2} ms | {flat_quarter:.2} ms | {flat_ten:.2} ms | {deep_quarter:.2} ms |"
        );
    }
}
