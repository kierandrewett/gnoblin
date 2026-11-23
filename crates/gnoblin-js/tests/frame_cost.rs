//! What one shell frame costs, for a shell with nothing to do and for one with a single change.
//!
//! The runtime role runs a frame every turn, so an idle frame is paid sixty times a second all day. Before change
//! gating it was proportional to the windows; with it, a frame that has nothing to do stops at a few comparisons.
//! `idle_frame_cost` prints the table (`cargo test --release -p gnoblin-js --test frame_cost -- --ignored
//! --nocapture`); it is ignored because a timing needs a release build, and it asserts that an idle frame at 300 windows stays under 300 us. What gating promises without a clock is
//! asserted by `frame_gating.rs` through the shell's frame counters. That the library's turn for a frame with one title change reads none
//! of the windows is asserted by `turn_scans.rs`, through the trips the library makes to the core.
//!
//! The link here is the fake of `tests/common`, which answers a poll at once. What a turn costs when it reads the real
//! tree, slot, session and idle links is `idle_turn_cost_over_the_real_links` in `crates/gnoblin/tests/js_shell.rs`.

use std::time::Instant;

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::policy::*;
use common::*;

const IDLE_FRAMES: u32 = 120;
const CHANGED_FRAMES: u32 = 60;

/// The most an idle frame at 300 windows may cost in a release build. Before change gating it was 3,400 us.
const IDLE_CEILING_US: f64 = 300.0;

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
        if windows == 300 {
            assert!(
                idle < IDLE_CEILING_US,
                "an idle frame at 300 windows costs {idle:.1} us, over the {IDLE_CEILING_US} us ceiling"
            );
        }
    }
}

/// A shell over the screen, the seat, the lock and `n` windows that are numbered from 100, so that none of them is one of the others.
/// `world` numbers its windows from 3 and the screen, the seat and the lock are 20, 21 and 23: from 18 windows on a window has taken
/// their place, so the shell of `idle_frame_cost` at 25, 100 and 300 windows has no screen to lay out on and no seat, and the library
/// reads every window for the focus. This world is laid out.
fn laid_out_world(n: u64) -> Rig {
    let init = "for (const id of [\"1\", \"2\", \"3\"]) gnoblin.workspaces.add(new Workspace({ id, layout: new MasterStackLayout() }));\n";
    let mut rig = rig_setup(&imports(), init, move |comp| {
        populate(comp, 0, false);
        for i in 0..n {
            comp.add_node(
                100 + i,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{i}"))),
                    ("app_id", text("foot")),
                    ("decoration", text("server")),
                    ("x", MirrorValue::Double(10.0 * i as f64)),
                    ("y", MirrorValue::Double(20.0 * i as f64)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            );
        }
    });
    settle(&mut rig);
    rig
}

#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn title_frame_cost_with_a_screen() {
    println!("windows | frame with one title change us (laid out: a screen, a seat, windows from 100)");
    for windows in [25_u64, 100, 300] {
        let mut rig = laid_out_world(windows);
        for _ in 0..12 {
            rig.frame();
        }
        let changed = mean_us(CHANGED_FRAMES, |index| {
            rig.comp.external_set(100, "title", text(&format!("title {index}")));
            rig.frame();
        });
        println!("{windows:7} | {changed:13.1}");
    }
}
