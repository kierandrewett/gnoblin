//! The software renderer against the golden pictures every renderer is held to (`gnoblin_render_frame::golden`).
//!
//! Every case marked `software` is drawn here, shadow and blur included, and a case added to the shared list and marked
//! is checked on this renderer from the day it lands. A case the renderer draws differently, or not at all, is left
//! unmarked in `gnoblin_render_frame::golden` with the reason beside it. The pictures are the GPU renderers'
//! (`GNOBLIN_UPDATE_GOLDEN` is never set for `software`), so a difference is the software renderer's to explain.

use gnoblin_render_frame::golden;
use gnoblin_render_software::Software;
use gnoblin_types::Scale;

#[test]
fn every_marked_scene_matches_its_golden_on_the_software_renderer() {
    for case in golden::CASES.iter().filter(|case| case.software) {
        let mut renderer = Software::new(golden::SIZE).expect("a software renderer");
        renderer.draw(&golden::frame_of(case), Scale::IDENTITY).expect("drawn");
        golden::check(case, "software", &renderer.screenshot().expect("a readback"));
    }
}

/// How far each marked case is from its golden, for the record: the check passes within the case's tolerance, and this
/// says how much of it is used.
#[test]
fn how_close_each_scene_is_to_its_golden() {
    for case in golden::CASES.iter().filter(|case| case.software) {
        let mut renderer = Software::new(golden::SIZE).expect("a software renderer");
        renderer.draw(&golden::frame_of(case), Scale::IDENTITY).expect("drawn");
        let image = renderer.screenshot().expect("a readback");
        let mut worst = 0;
        for (index, want) in golden::golden(case).iter().enumerate() {
            let (x, y) = (
                (index as u32 % golden::SIZE.width) as i32,
                (index as u32 / golden::SIZE.width) as i32,
            );
            let got = image.colour_at(x, y).expect("a pixel");
            worst = worst.max(
                got.iter()
                    .zip(want)
                    .map(|(got, want)| got.abs_diff(*want))
                    .max()
                    .unwrap_or(0),
            );
        }
        println!("{}: worst channel difference {worst} of {}", case.name, case.tolerance);
        assert!(worst <= case.tolerance);
    }
}
