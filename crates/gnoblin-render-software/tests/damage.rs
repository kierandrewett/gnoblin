//! A blurred layer redrawn over a small damage looks like the whole frame redrawn.
//!
//! A blur reads the pixels around its damage. The renderer redraws only the damage, so those pixels are what the last frame
//! left, which under a blurred layer is the layer's own output. `OutputFrame::expand_damage_for_blur`, which the compositor
//! calls on every frame it composes, damages the whole layer instead, and this is the picture that proves it: a frame, then
//! a change under the glass drawn over its damage, against a fresh renderer that draws the changed frame whole.

use gnoblin_render::{BlurParams, Effects, Pass};
use gnoblin_render_frame::frame::{Layer, OutputFrame, Solid};
use gnoblin_render_software::Software;
use gnoblin_types::{Color, Rect, Scale, Size};

const SIZE: Size = Size::new(160, 96);
const GLASS: Rect = Rect::new(40, 16, 80, 64);

/// A stripy backdrop, a small block that moves, and glass over both.
fn frame(block: Rect, damage: Rect) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(damage);
    for stripe in 0..16 {
        let colour = if stripe % 2 == 0 {
            Color::from_rgba8(20, 30, 90, 255)
        } else {
            Color::from_rgba8(230, 220, 200, 255)
        };
        frame.push_layer(Layer::Solid(Solid::new(Rect::new(stripe * 10, 0, 10, 96), colour)));
    }
    frame.push_layer(Layer::Solid(Solid::new(block, Color::from_rgba8(220, 40, 40, 255))));
    let mut glass = Solid::new(GLASS, Color::TRANSPARENT);
    let mut effects = Effects::new();
    effects
        .push(Pass::Blur(BlurParams {
            radius: 6.0,
            passes: 2,
            ..BlurParams::default()
        }))
        .expect("a blur");
    glass.effects = effects;
    frame.push_layer(Layer::Solid(glass));
    frame
}

fn draw(software: &mut Software, frame: &OutputFrame) {
    software.draw(frame, Scale::IDENTITY).expect("drawn");
}

fn worst_difference(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0)
}

#[test]
fn a_change_under_glass_drawn_over_its_damage_matches_a_full_redraw() {
    let (before, after) = (Rect::new(56, 32, 8, 8), Rect::new(60, 36, 8, 8));
    // The damage of the move: the block's old and new rectangles.
    let changed = before.union(after);

    let mut software = Software::new(SIZE).expect("a software renderer");
    draw(&mut software, &frame(before, Rect::new(0, 0, SIZE.width, SIZE.height)));
    let mut partial = frame(after, changed);
    partial.expand_damage_for_blur();
    draw(&mut software, &partial);
    let drawn = software.screenshot().expect("a readback");

    let mut fresh = Software::new(SIZE).expect("a software renderer");
    draw(&mut fresh, &frame(after, Rect::new(0, 0, SIZE.width, SIZE.height)));
    let whole = fresh.screenshot().expect("a readback");
    assert_eq!(
        worst_difference(drawn.data(), whole.data()),
        0,
        "the partial redraw is the full redraw"
    );

    // Without the growth the picture is seamed: the test would not catch its absence otherwise.
    let mut software = Software::new(SIZE).expect("a software renderer");
    draw(&mut software, &frame(before, Rect::new(0, 0, SIZE.width, SIZE.height)));
    draw(&mut software, &frame(after, changed));
    let seamed = software.screenshot().expect("a readback");
    assert!(
        worst_difference(seamed.data(), whole.data()) > 8,
        "an unexpanded damage redraws blurred pixels again, and the picture differs"
    );
}
