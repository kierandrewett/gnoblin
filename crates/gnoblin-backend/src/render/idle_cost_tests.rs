//! What one compositor iteration costs when nothing changed, for a scene of N windows.
//!
//! The loop wakes for a client, an input event or its timeout, and every wake runs the iteration: the motion clock
//! advances, the composer takes its changes and its damage, and each output asks for its frame. A scene that did not
//! change must make that cost nothing that grows with the windows, which `an_unchanged_scene_does_no_per_node_work`
//! asserts through the composer's own counters and `idle_iteration_cost` prints in microseconds
//! (`cargo test --release -p gnoblin-backend idle_iteration_cost -- --ignored --nocapture`).

use std::time::Instant;

use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_types::{OutputId, Rect, Size, WindowId};

use super::*;

const OUTPUT_RECT: Rect = Rect::new(0, 0, 1920, 1080);

/// A composer with one output and `windows` windows of 300 by 200, each showing a flat picture.
fn crowded(windows: u32) -> (Composer, OutputId) {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, OUTPUT_RECT);
    composer.set_reduced_motion(true);
    let size = Size::new(300, 200);
    let pixels = [40_u8, 80, 120, 255].repeat(size.width as usize * size.height as usize);
    let image = Image::new(size, size.width as usize * 4, pixels).expect("an image of that size");
    for index in 0..windows {
        let window = WindowId::new(u64::from(index) + 1);
        let at = Rect::new(
            i32::try_from(index % 20).unwrap_or(0) * 60,
            i32::try_from(index / 20).unwrap_or(0) * 40,
            size.width,
            size.height,
        );
        composer.insert_window(window, at);
        composer.set_window_content(
            window,
            Some(Content {
                src: RectF::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
                size,
                transform: Transform::Normal,
            }),
        );
        composer.set_window_source(window, Some(SurfaceSource::Snapshot(Arc::new(image.clone()))));
    }
    // The first iterations draw the new windows and settle their fades.
    for _ in 0..4 {
        iterate(&mut composer, output);
    }
    (composer, output)
}

/// One iteration as the compositor's loop runs it: the clock, the composer's changes and damage, the output's frame.
/// Returns whether the output had anything to draw.
fn iterate(composer: &mut Composer, output: OutputId) -> bool {
    composer.advance_motion(16);
    composer.begin_frame();
    drop(composer.take_tree_changes());
    drop(composer.take_tree_ready());
    let mut frame = OutputFrame::new();
    let drew = composer.compose(output, &mut frame);
    composer.end_frame();
    drew
}

#[test]
fn an_unchanged_scene_draws_nothing() {
    let (mut composer, output) = crowded(30);
    for _ in 0..5 {
        assert!(!iterate(&mut composer, output), "nothing changed, so nothing is drawn");
    }
}

#[test]
fn every_kind_of_change_is_drawn_on_the_next_iteration() {
    let (mut composer, output) = crowded(8);
    let window = WindowId::new(3);

    composer.place_window(window, Rect::new(500, 500, 300, 200));
    assert!(iterate(&mut composer, output), "a moved window is drawn");
    assert!(!iterate(&mut composer, output));

    composer.set_window_opacity(window, 0.5);
    assert!(iterate(&mut composer, output), "an opacity write is drawn");
    assert!(!iterate(&mut composer, output));

    composer.set_window_visible(window, false);
    assert!(iterate(&mut composer, output), "a hidden window is drawn away");
    assert!(!iterate(&mut composer, output));

    composer.set_cursor(Some(Rect::new(10, 10, 16, 16)), false);
    assert!(iterate(&mut composer, output), "a cursor that appeared is drawn");
    composer.set_cursor(Some(Rect::new(40, 40, 16, 16)), false);
    assert!(iterate(&mut composer, output), "a cursor that moved is drawn");
    composer.set_cursor(Some(Rect::new(40, 40, 16, 16)), true);
    assert!(iterate(&mut composer, output), "a cursor that changed shape is drawn");
    assert!(!iterate(&mut composer, output));

    composer.damage_output(output);
    assert!(
        iterate(&mut composer, output),
        "a damaged output (a lock cover) is drawn"
    );
    assert!(!iterate(&mut composer, output));

    composer.set_output(output, Rect::new(0, 0, 1280, 720));
    assert!(iterate(&mut composer, output), "an output that changed size is drawn");
    assert!(!iterate(&mut composer, output));

    composer.remove_window(WindowId::new(4));
    // A window that was shown goes at once or fades; either way a frame follows. (The hidden one above has nothing to remove.)
    let mut drew = false;
    for _ in 0..40 {
        drew |= iterate(&mut composer, output);
    }
    assert!(drew, "a closed window is drawn away");
}

#[test]
#[ignore = "a benchmark: run with --release --ignored --nocapture"]
fn idle_iteration_cost() {
    println!("windows | idle iteration us");
    for windows in [0_u32, 25, 100, 300] {
        let (mut composer, output) = crowded(windows);
        let frames = 400_u32;
        let started = Instant::now();
        for _ in 0..frames {
            iterate(&mut composer, output);
        }
        let per = started.elapsed().as_secs_f64() * 1e6 / f64::from(frames);
        println!("{windows:7} | {per:10.1}");
    }
}
