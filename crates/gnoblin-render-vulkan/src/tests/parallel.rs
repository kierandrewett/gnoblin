//! Renderers made, drawn with and dropped on several threads at once.
//!
//! A compositor makes one renderer per output, and this crate's own tests make one each on every test thread. The
//! validation layer keeps its per-device data in one global map that it changes, without a lock, when a device is made or
//! destroyed, and a thread that calls into it meanwhile has taken the test binary down with a segmentation fault inside the
//! layer (bead gnoblin-wphw). A context that validates therefore holds a process-wide seat for its life
//! (`context::ValidationSeat`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;
use crate::context::ValidationSeat;

/// Makes, draws with and drops renderers from several threads together: the load that took the binary down.
#[test]
fn renderers_are_made_drawn_with_and_dropped_on_several_threads_at_once() {
    if renderer(SIZE).is_none() {
        return;
    }
    let threads: Vec<_> = (0..6)
        .map(|thread| {
            std::thread::spawn(move || {
                for round in 0..4 {
                    let Ok(mut vulkan) = Vulkan::with_options(SIZE, &options()) else {
                        panic!("thread {thread}, round {round}: a renderer could not be made");
                    };
                    let mut frame = frame(Color::BLACK, SIZE);
                    solid(&mut frame, Rect::new(2, 2, 20, 20), colour(255, 0, 0, 255));
                    vulkan.draw(&frame, Scale::IDENTITY).expect("a frame");
                    let image = vulkan.screenshot().expect("a readback");
                    assert_eq!(image.colour_at(4, 4), Some([255, 0, 0, 255]));
                }
            })
        })
        .collect();
    for thread in threads {
        thread
            .join()
            .expect("a thread that made, drew with and dropped its renderers");
    }
}

/// One thread has the seat at a time, a thread can take it again, and it is free when the last of those lets go.
#[test]
fn the_validation_seat_is_held_by_one_thread_at_a_time() {
    let seat = ValidationSeat::take();
    let again = ValidationSeat::take();
    let taken = Arc::new(AtomicBool::new(false));
    let other = {
        let taken = Arc::clone(&taken);
        std::thread::spawn(move || {
            let _seat = ValidationSeat::take();
            taken.store(true, Ordering::Release);
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !taken.load(Ordering::Acquire),
        "another thread took a seat that is held"
    );
    drop(again);
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !taken.load(Ordering::Acquire),
        "another thread took a seat its holder still has once"
    );
    drop(seat);
    other.join().expect("the waiting thread");
    assert!(taken.load(Ordering::Acquire), "the seat was never let go");
}
