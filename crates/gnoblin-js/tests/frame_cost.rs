//! What one shell frame costs, for a shell with nothing to do and for one with a single change.
//!
//! The runtime role runs a frame every turn, so an idle frame is paid sixty times a second all day. Before change
//! gating it was proportional to the windows; with it, a frame that has nothing to do stops at a few comparisons.
//! `idle_frame_cost` prints the table (`cargo test --release -p gnoblin-js --test frame_cost -- --ignored
//! --nocapture`); it is ignored because a timing is not a pass or fail. What gating promises without a clock is
//! asserted by `frame_gating.rs` through the shell's frame counters.

use std::time::Instant;

mod common;
use common::policy::*;
use common::*;

const IDLE_FRAMES: u32 = 120;
const CHANGED_FRAMES: u32 = 60;

/// The mean cost of one frame in microseconds over `frames` runs of `step`.
fn mean_us(frames: u32, mut step: impl FnMut(u32)) -> f64 {
    let started = Instant::now();
    for index in 0..frames {
        step(index);
    }
    started.elapsed().as_secs_f64() * 1e6 / f64::from(frames)
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn idle_frame_cost() {
    println!("windows | idle frame us | frame with one title change us");
    for windows in [0_u64, 25, 100, 300] {
        let mut rig = world(windows, false, "new MasterStackLayout()", "");
        // Let the boot settle: the first frames announce the windows and lay them out.
        for _ in 0..12 {
            rig.frame();
        }
        let idle = mean_us(IDLE_FRAMES, |_| {
            rig.frame();
        });
        let changed = if windows == 0 {
            f64::NAN
        } else {
            mean_us(CHANGED_FRAMES, |index| {
                rig.comp.external_set(3, "title", text(&format!("title {index}")));
                rig.frame();
            })
        };
        println!("{windows:7} | {idle:13.1} | {changed:13.1}");
    }
}
