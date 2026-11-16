//! A session that changed its scene shows exactly the scene it has now: no window leaves a picture of where it was.
//!
//! The renderer repaints only what the frame's damage names, so a change the damage misses stays on the screen for
//! good. These scenarios run the whole stack (a real client, the window manager, the composer and the headless
//! backend's software renderer) over an empty desktop and read the output back. On an empty desktop every window is a
//! flat rectangle on black, so the picture the session should show is known without a second renderer: the
//! background everywhere and each window's colour inside its rectangle. A pixel that is neither is a stale pixel.
//!
//! `gnoblin-backend/tests/damage_correctness.rs` runs the same question over random scenes, and `cursor_trail.rs`
//! asks it of the pointer's cursor, which a screenshot leaves out; this file asks it where the composer meets the
//! session: window management and a scale change. The lock's reveal is in `session_lock.rs`.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Point, Rect};
use session::{Client, Handle, Session};

/// How long a scenario waits for a picture to settle before it fails.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The desktop's background.
const BACKGROUND: [u8; 3] = [0, 0, 0];

/// A window's colour, as the client's ARGB bytes `[b, g, r, a]` and as a screenshot's `[r, g, b]`.
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 3] = [255, 0, 0];

fn output_of(session: &Handle) -> OutputId {
    session
        .wait_for(|observation| !observation.outputs().is_empty())
        .outputs()
        .first()
        .copied()
        .expect("an output")
}

/// The pixels that are not what a scene of flat `windows` on [`BACKGROUND`] shows, as points, with `factor` physical
/// pixels to a logical one. A colour may be off by `tolerance` in a channel.
fn stale(image: &Image, windows: &[(Rect, [u8; 3])], factor: f64, tolerance: i32) -> Vec<Point> {
    let size = image.size();
    let mut wrong = Vec::new();
    for y in 0..size.height {
        for x in 0..size.width {
            let at = Point::new(x as i32, y as i32);
            let Some(pixel) = image.pixel(at) else {
                continue;
            };
            let shown = [pixel[0], pixel[1], pixel[2]];
            let (lx, ly) = (f64::from(x) / factor, f64::from(y) / factor);
            // A pixel on a window's edge may belong to either side once the rectangle is scaled; leave the one-pixel
            // border of every rectangle out of the comparison.
            let near_edge = |rect: &Rect| {
                let (left, top) = (f64::from(rect.x), f64::from(rect.y));
                let (right, bottom) = (left + f64::from(rect.width), top + f64::from(rect.height));
                lx >= left - 1.0 && lx < right + 1.0 && ly >= top - 1.0 && ly < bottom + 1.0
            };
            let inside = |rect: &Rect| {
                let (left, top) = (f64::from(rect.x), f64::from(rect.y));
                let (right, bottom) = (left + f64::from(rect.width), top + f64::from(rect.height));
                lx >= left + 1.0 && lx < right - 1.0 && ly >= top + 1.0 && ly < bottom - 1.0
            };
            let expected = windows.iter().find(|(rect, _)| inside(rect)).map(|(_, colour)| *colour);
            let on_edge = windows.iter().any(|(rect, _)| near_edge(rect));
            let want = match expected {
                Some(colour) => colour,
                None if on_edge => continue,
                None => BACKGROUND,
            };
            let close = shown
                .iter()
                .zip(&want)
                .all(|(seen, want)| (i32::from(*seen) - i32::from(*want)).abs() <= tolerance);
            if !close {
                wrong.push(at);
            }
        }
    }
    wrong
}

/// Waits until the output shows exactly `windows`, or fails with the stale pixels the last picture had.
#[track_caller]
fn shows(session: &Handle, output: OutputId, windows: &[(Rect, [u8; 3])], factor: f64, tolerance: i32, what: &str) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let image = session
            .control
            .screenshot(output)
            .expect("the loop answers a screenshot");
        let wrong = stale(&image, windows, factor, tolerance);
        if wrong.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: {} pixels are not what the scene shows, the first at {:?} (it shows {:?})",
            wrong.len(),
            wrong[0],
            image.pixel(wrong[0])
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// The window's rectangle once the session has placed it.
fn placed(session: &Handle, what: &str) -> (gnoblin_types::WindowId, Rect) {
    let window = session
        .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .unwrap_or_else(|| panic!("{what}: one window"))
        .clone();
    (window.id, window.geometry)
}

#[test]
fn a_moved_window_leaves_nothing_at_its_old_place() {
    let session = Session::start();
    session.run(|session| {
        let output = output_of(session);
        let mut client = Client::connect(session.socket());
        let _window = client.map_solid_window("org.gnoblin.Stale", "Stale", (120, 90), RED_BGRA);
        let (id, geometry) = placed(session, "mapped");
        shows(
            session,
            output,
            &[(geometry, RED)],
            1.0,
            0,
            "the window as it was placed",
        );

        for (x, y) in [(300, 200), (310, 210), (40, 400), (500, 20), (0, 0), (300, 200)] {
            let to = Rect::new(x, y, geometry.width, geometry.height);
            session.control.move_window(id, to);
            shows(
                session,
                output,
                &[(to, RED)],
                1.0,
                0,
                &format!("after moving to {to:?}"),
            );
        }
    });
}

#[test]
fn a_closed_window_leaves_nothing() {
    let session = Session::start();
    session.run(|session| {
        let output = output_of(session);
        let mut client = Client::connect(session.socket());
        let window = client.map_solid_window("org.gnoblin.Stale", "Stale", (120, 90), RED_BGRA);
        let (_, geometry) = placed(session, "mapped");
        shows(
            session,
            output,
            &[(geometry, RED)],
            1.0,
            0,
            "the window as it was placed",
        );
        client.destroy(window);
        shows(session, output, &[], 1.0, 0, "after the window closed");
    });
}

#[test]
fn a_window_that_fades_leaves_the_pixels_of_its_opacity() {
    let session = Session::start();
    session.run(|session| {
        let output = output_of(session);
        let mut client = Client::connect(session.socket());
        let _window = client.map_solid_window("org.gnoblin.Stale", "Stale", (120, 90), RED_BGRA);
        let (id, geometry) = placed(session, "mapped");
        shows(
            session,
            output,
            &[(geometry, RED)],
            1.0,
            0,
            "the window as it was placed",
        );

        // Half opaque over black is half the red; the window must show that everywhere and not the full red it had.
        session.control.set_window_opacity(id, 0.5);
        shows(session, output, &[(geometry, [128, 0, 0])], 1.0, 3, "at half opacity");
        session.control.set_window_opacity(id, 1.0);
        shows(session, output, &[(geometry, RED)], 1.0, 0, "back at full opacity");
    });
}

#[test]
fn a_scale_change_leaves_nothing_of_the_old_scale() {
    let session = Session::start();
    session.run(|session| {
        let output = output_of(session);
        let mut client = Client::connect(session.socket());
        let _window = client.map_solid_window("org.gnoblin.Stale", "Stale", (120, 90), RED_BGRA);
        let (_, geometry) = placed(session, "mapped");
        shows(session, output, &[(geometry, RED)], 1.0, 0, "at scale one");

        session.control.set_global_scale(Some(2));
        // The window is drawn twice as large in the output's pixels, wherever the window manager now puts it.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (_, geometry) = placed(session, "after the scale change");
            let image = session.control.screenshot(output).expect("a screenshot");
            let wrong = stale(&image, &[(geometry, RED)], 2.0, 0);
            if wrong.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "after the scale went to two: {} pixels are not what the scene shows, the first at {:?}",
                wrong.len(),
                wrong[0]
            );
            thread::sleep(Duration::from_millis(10));
        }

        session.control.set_global_scale(Some(1));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (_, geometry) = placed(session, "after the scale returned");
            let image = session.control.screenshot(output).expect("a screenshot");
            let wrong = stale(&image, &[(geometry, RED)], 1.0, 0);
            if wrong.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "back at scale one: {} pixels are not what the scene shows, the first at {:?}",
                wrong.len(),
                wrong[0]
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
}
