//! What a frame with a blurred, shadowed window costs on the CPU path.
//!
//! The software renderer draws a backdrop blur, a soft shadow and an anti-aliased border by computing the pixels in
//! memory (`src/layers.rs`), which is the most expensive thing it does. This file measures one 1920 by 1080 frame with
//! one blurred window, whole and with a small damage, so a change that makes the blur slower shows as a number. It is
//! a measurement, not a check, so it is ignored by default:
//!
//! ```text
//! cargo test --release -p gnoblin-render-software --test effect_cost -- --ignored --nocapture
//! ```
//!
//! Run it in release: a debug build is more than ten times slower and says nothing about the compositor. Each row is the
//! median of [`RUNS`] frames after [`WARM_UP`] that are not counted, in milliseconds of CPU time (so a busy machine does not
//! inflate it).

use std::sync::Arc;
use std::time::Duration;

use gnoblin_render::{BlurParams, Border, Effects, Pass, Shadow};
use gnoblin_render_frame::frame::{Layer, OutputFrame, Solid, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_software::Software;
use gnoblin_types::{Color, Rect, RectF, Scale, Size, Vec3};

const WARM_UP: usize = 3;
const RUNS: usize = 15;
const OUTPUT: Size = Size::new(1920, 1080);
const WINDOW: Rect = Rect::new(360, 190, 1200, 700);

/// A window's own picture: a diagonal gradient, so a blur has something to work on.
fn picture(size: Size) -> Arc<Image> {
    let mut data = Vec::with_capacity((size.width * size.height * 4) as usize);
    for y in 0..size.height {
        for x in 0..size.width {
            data.extend_from_slice(&[(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255]);
        }
    }
    Arc::new(Image::new(size, (size.width * 4) as usize, data).expect("a packed image"))
}

/// Two stripes behind the window, so the blur sees an edge.
fn backdrop(frame: &mut OutputFrame) {
    for (x, colour) in [
        (0, Color::from_rgba8(30, 60, 160, 255)),
        (960, Color::from_rgba8(240, 230, 210, 255)),
    ] {
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(x, 0, 960, OUTPUT.height), colour)));
    }
}

fn window(effects: Effects, opacity: f32) -> Layer {
    let size = Size::new(WINDOW.width, WINDOW.height);
    Layer::Surface(Surface {
        source: SurfaceSource::Snapshot(picture(size)),
        src: RectF {
            x: 0.0,
            y: 0.0,
            width: f64::from(size.width),
            height: f64::from(size.height),
        },
        dst: WINDOW,
        transform: smithay::utils::Transform::Normal,
        opacity,
        tint: Color::WHITE,
        corner_radius: 12.0,
        silhouette: None,
        effects,
        model: None,
        secure: false,
        content: None,
    })
}

/// A glass layer over the window's place: no colour of its own, so it shows its backdrop blurred.
fn glass(blur: BlurParams) -> Layer {
    let mut effects = Effects::new();
    effects.push(Pass::Blur(blur)).expect("a blur");
    let mut glass = Solid::new(WINDOW, Color::TRANSPARENT);
    glass.corner_radius = 12.0;
    glass.effects = effects;
    Layer::Solid(glass)
}

fn effects() -> Effects {
    let mut effects = Effects::new();
    effects
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 8.0, 0.0),
            blur: 24.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::from_rgba8(0, 0, 0, 120),
            frame: false,
            inset: false,
        }))
        .expect("a shadow");
    effects
        .push(Pass::Border(Border {
            width: 1.0,
            offset: 0.0,
            inset: true,
            color: Color::from_rgba8(255, 255, 255, 80),
        }))
        .expect("a border");
    effects
}

fn frame(blurred: bool, damage: Rect, opacity: f32, effects: Effects) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(damage);
    backdrop(&mut frame);
    if blurred {
        frame.push_layer(glass(BlurParams::default()));
    }
    frame.push_layer(window(effects, opacity));
    frame
}

/// The time this thread has spent running on a CPU. The measurement is of the renderer's work, not of how busy the
/// machine is, so it reads the scheduler's account of the thread and not the wall clock.
fn cpu_time() -> Duration {
    let schedstat = std::fs::read_to_string("/proc/thread-self/schedstat").expect("Linux's scheduler statistics");
    let nanoseconds: u64 = schedstat
        .split_whitespace()
        .next()
        .and_then(|first| first.parse().ok())
        .expect("a run time in nanoseconds");
    Duration::from_nanos(nanoseconds)
}

fn median(mut times: Vec<Duration>) -> f64 {
    times.sort();
    times[times.len() / 2].as_secs_f64() * 1000.0
}

fn measure(label: &str, blurred: bool, damage: Rect, opacity: f32, effects: Effects) {
    let mut software = Software::new(OUTPUT).expect("a software renderer");
    let frame = frame(blurred, damage, opacity, effects);
    let mut times = Vec::new();
    for run in 0..WARM_UP + RUNS {
        let started = cpu_time();
        software.draw(&frame, Scale::IDENTITY).expect("drawn");
        if run >= WARM_UP {
            times.push(cpu_time() - started);
        }
    }
    println!("{label:<58} {:>8.2} ms", median(times));
}

#[test]
#[ignore = "a measurement: run with --ignored in release"]
fn the_cost_of_one_frame_with_a_blurred_window() {
    let whole = Rect::new(0, 0, OUTPUT.width, OUTPUT.height);
    let small = Rect::new(800, 400, 240, 48);
    println!("1920x1080, a 1200x700 window with radius 12, a shadow and a border");
    measure("window only, whole screen damaged", false, whole, 1.0, Effects::new());
    let mut shadow_only = Effects::new();
    let mut border_only = Effects::new();
    for pass in effects().passes() {
        match pass {
            Pass::Shadow(_) => shadow_only.push(pass.clone()).expect("a pass"),
            _ => border_only.push(pass.clone()).expect("a pass"),
        }
    }
    measure("window and a shadow, whole screen", false, whole, 1.0, shadow_only);
    measure("window and a border, whole screen", false, whole, 1.0, border_only);
    measure(
        "window and shadow and border, whole screen",
        false,
        whole,
        1.0,
        effects(),
    );
    measure(
        "... and a backdrop blur (radius 12, 3 passes)",
        true,
        whole,
        1.0,
        effects(),
    );
    measure(
        "... with the window at opacity 0.8 (a group)",
        true,
        whole,
        0.8,
        effects(),
    );
    measure("... with a 240x48 damage", true, small, 1.0, effects());
}
