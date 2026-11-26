//! Golden pixels for what a node's `effects` list and its colour properties draw, shared by every renderer.
//!
//! Each case below is one small fixed scene. A renderer draws it and [`check`] compares every pixel with a picture
//! checked in under `tests/golden/`, so a change to a blur kernel, a shadow falloff, a tint or the group-opacity rule
//! shows as a diff in a file, not as a probe that happens to still hold. The two GPU renderers draw the same cases
//! against the same files: a case one of them draws differently is a disagreement settled by the design
//! (`gnoblin-render-vulkan/DESIGN.md`, section 6), not by whichever picture is newer.
//!
//! A golden is text, one row of the picture a line and one pixel a word of eight hex digits (`rrggbbaa`), so a diff can
//! be read. A pixel matches within the case's own tolerance on every channel: one rounding step of one driver differs
//! from another's, and the check is for the effect, not for one driver's arithmetic.
//!
//! `GNOBLIN_UPDATE_GOLDEN=gles` or `=vulkan` writes the files from that renderer (the name [`check_all`] is given,
//! compared without regard to case) and does not compare. The first renderer to draw a new case writes it; the other
//! must match it. Read the diff before committing it.
//!
//! Each case also has probes: statements about the picture that hold whatever the golden says (the window's colour
//! under a tint, a shadow darker than the backdrop). They run on every picture, including the one a renderer is about to
//! write, so a wrong golden written by the first renderer to draw a case is caught.
//!
//! Tint and opacity are properties the software renderer draws too, so a case marked `software` is checked against its
//! picture as well. Blur and shadow are the GPU renderers' alone.

use std::path::PathBuf;
use std::sync::Arc;

use gnoblin_render::{BlurParams, Effects, Pass, Shadow, Vec3};
use gnoblin_types::{Color, Rect, RectF, Size};

use crate::frame::{Layer, OutputFrame, Solid, Surface, SurfaceSource};
use crate::image::Image;

/// The tolerance of a case whose pixels are shapes and flat colour.
pub const SHAPE_TOLERANCE: u8 = 3;
/// The tolerance of a case that blurs: two drivers round bilinear taps differently.
pub const BLUR_TOLERANCE: u8 = 4;
/// The size of every golden scene, in pixels.
pub const SIZE: Size = Size::new(32, 24);
/// Where the window sits in every scene.
pub const WINDOW: Rect = Rect::new(8, 7, 16, 9);

/// The window's own picture: orange with a lighter top row, so a crop or a flip shows.
fn window_picture() -> Arc<Image> {
    let mut data = Vec::new();
    for row in 0..9u8 {
        for _ in 0..16 {
            data.extend_from_slice(if row == 0 {
                &[250, 190, 90, 255]
            } else {
                &[230, 120, 30, 255]
            });
        }
    }
    Arc::new(Image::new(Size::new(16, 9), 64, data).expect("an image that holds its rows"))
}

/// What the scenes draw over: a blue half beside a pale half, so a blur has an edge to soften and a shadow a
/// background to darken.
fn backdrop() -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, SIZE.width, SIZE.height));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 16, SIZE.height),
        Color::from_rgba8(30, 60, 160, 255),
    )));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(16, 0, 16, SIZE.height),
        Color::from_rgba8(240, 230, 210, 255),
    )));
    frame
}

/// The window layer, with `effects`, `tint` and group `opacity`.
fn window(effects: Effects, tint: Color, opacity: f32) -> Layer {
    Layer::Surface(Surface {
        source: SurfaceSource::Snapshot(window_picture()),
        src: RectF {
            x: 0.0,
            y: 0.0,
            width: 16.0,
            height: 9.0,
        },
        dst: WINDOW,
        transform: smithay::utils::Transform::Normal,
        opacity,
        tint,
        corner_radius: 0.0,
        silhouette: None,
        effects,
        model: None,
        secure: false,
    })
}

fn one(pass: Pass) -> Effects {
    let mut effects = Effects::new();
    effects.push(pass).expect("a pass in range");
    effects
}

fn shadow() -> Pass {
    Pass::Shadow(Shadow {
        offset: Vec3::new(0.0, 3.0, 0.0),
        blur: 6.0,
        spread: 1.0,
        opacity: 1.0,
        color: Color::from_rgba8(0, 0, 0, 160),
        frame: false,
        inset: false,
    })
}

fn blur() -> Pass {
    Pass::Blur(BlurParams {
        radius: 4.0,
        passes: 2,
        tint: Color::from_rgba8(255, 255, 255, 40),
        ..BlurParams::default()
    })
}

/// A glass layer: no colour of its own, so what it shows is the blur of what is behind it.
fn glass() -> Layer {
    let mut glass = Solid::new(Rect::new(6, 5, 20, 13), Color::TRANSPARENT);
    glass.effects = one(blur());
    Layer::Solid(glass)
}

const TINT: Color = Color::new(0.5, 0.8, 1.0, 1.0);

/// The backdrop's two colours and the window's, as the probes read them.
const BLUE: [u8; 3] = [30, 60, 160];
const PALE: [u8; 3] = [240, 230, 210];
const ORANGE: [u8; 3] = [230, 120, 30];

/// One named scene.
pub struct Case {
    /// The case's name; its golden is `tests/golden/effects_<name>.txt`.
    pub name: &'static str,
    /// The layers that go over the backdrop.
    pub layers: fn() -> Vec<Layer>,
    /// Whether the software renderer draws this scene as well, and so is checked against the same picture.
    pub software: bool,
    /// The most a channel of a pixel may differ from the golden.
    pub tolerance: u8,
    /// Statements about the picture that hold whatever the golden says. They panic when one does not.
    pub probes: fn(&Image),
}

/// The five scenes every renderer is held to.
pub const CASES: [Case; 5] = [
    Case {
        name: "shadow",
        layers: || vec![window(one(shadow()), Color::WHITE, 1.0)],
        software: false,
        tolerance: SHAPE_TOLERANCE,
        probes: |image| {
            assert_eq!(rgb(image, 12, 10), ORANGE, "the window is over its shadow");
            // Below the window, on the pale half: the shadow darkens it and the backdrop far away is untouched.
            let under = rgb(image, 20, 17);
            assert!(
                under.iter().zip(PALE).all(|(got, plain)| *got < plain),
                "a shadow darkens the backdrop under the window: {under:?}"
            );
            assert_eq!(rgb(image, 30, 1), PALE, "and leaves the corner alone");
        },
    },
    Case {
        name: "blur",
        layers: || vec![glass(), window(Effects::new(), Color::WHITE, 1.0)],
        software: false,
        tolerance: BLUR_TOLERANCE,
        probes: |image| {
            assert_eq!(rgb(image, 12, 10), ORANGE, "the window is over its glass");
            // Below the window, across the blue/pale edge: the blur softens it.
            let blue_side = rgb(image, 15, 17);
            let pale_side = rgb(image, 16, 17);
            assert!(
                blue_side[0] > BLUE[0] + 20,
                "blue is blurred towards pale: {blue_side:?}"
            );
            assert!(
                pale_side[0] < PALE[0] - 20,
                "pale is blurred towards blue: {pale_side:?}"
            );
        },
    },
    Case {
        name: "tint",
        layers: || vec![window(Effects::new(), TINT, 1.0)],
        software: true,
        tolerance: SHAPE_TOLERANCE,
        probes: |image| {
            let want = [(230.0_f32 * 0.5), (120.0 * 0.8), 30.0].map(|channel| channel.round() as u8);
            assert_close(
                rgb(image, 12, 10),
                want,
                SHAPE_TOLERANCE,
                "the window's colour times the tint",
            );
        },
    },
    Case {
        name: "opacity",
        layers: || vec![window(Effects::new(), Color::WHITE, 0.5)],
        software: true,
        tolerance: SHAPE_TOLERANCE,
        probes: |image| {
            // Over the pale half: half the window and half the backdrop.
            let want = [0, 1, 2].map(|i| ((ORANGE[i] as f32 + PALE[i] as f32) / 2.0).round() as u8);
            assert_close(
                rgb(image, 20, 10),
                want,
                SHAPE_TOLERANCE,
                "half the window over the backdrop",
            );
        },
    },
    Case {
        name: "stack",
        layers: || {
            let mut chain = Effects::new();
            chain.push(shadow()).expect("a shadow");
            chain.push(blur()).expect("a blur");
            vec![window(chain, TINT, 0.6)]
        },
        software: false,
        tolerance: BLUR_TOLERANCE,
        probes: |image| {
            let window_pixel = rgb(image, 12, 10);
            assert_ne!(
                window_pixel, ORANGE,
                "a tinted translucent window is not its own colour"
            );
            assert_ne!(window_pixel, BLUE, "and the backdrop does not show through whole");
        },
    },
];

fn rgb(image: &Image, x: i32, y: i32) -> [u8; 3] {
    let [r, g, b, _] = image.colour_at(x, y).expect("a pixel of the picture");
    [r, g, b]
}

fn assert_close(got: [u8; 3], want: [u8; 3], tolerance: u8, what: &str) {
    assert!(
        got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= tolerance),
        "{what}: drew {got:?}, expected {want:?}"
    );
}

/// The frame of a case: the backdrop with the case's layers over it.
#[must_use]
pub fn frame_of(case: &Case) -> OutputFrame {
    let mut frame = backdrop();
    for layer in (case.layers)() {
        frame.push_layer(layer);
    }
    frame
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("effects_{name}.txt"))
}

/// The picture as text: a line a row, a word of `rrggbbaa` a pixel.
#[must_use]
pub fn render_text(image: &Image) -> String {
    let mut text = String::new();
    for y in 0..SIZE.height as i32 {
        let row: Vec<String> = (0..SIZE.width as i32)
            .map(|x| {
                let [r, g, b, a] = image.colour_at(x, y).expect("a pixel of the picture");
                format!("{r:02x}{g:02x}{b:02x}{a:02x}")
            })
            .collect();
        text.push_str(&row.join(" "));
        text.push('\n');
    }
    text
}

/// The pixels of a golden's text.
#[must_use]
pub fn parse(text: &str) -> Vec<[u8; 4]> {
    text.split_whitespace()
        .map(|word| {
            let channel = |at: usize| u8::from_str_radix(&word[at..at + 2], 16).expect("two hex digits");
            [channel(0), channel(2), channel(4), channel(6)]
        })
        .collect()
}

/// The pixels of the golden of `case`.
///
/// # Panics
///
/// When the file is missing or is not a whole picture.
#[must_use]
pub fn golden(case: &Case) -> Vec<[u8; 4]> {
    let path = golden_path(case.name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; run with GNOBLIN_UPDATE_GOLDEN=<renderer>",
            path.display()
        )
    });
    let golden = parse(&text);
    assert_eq!(
        golden.len(),
        (SIZE.width * SIZE.height) as usize,
        "{}: the golden is a whole picture",
        case.name
    );
    golden
}

/// Whether `GNOBLIN_UPDATE_GOLDEN` names `who`.
fn updating(who: &str) -> bool {
    std::env::var("GNOBLIN_UPDATE_GOLDEN").is_ok_and(|named| named.eq_ignore_ascii_case(who))
}

/// Panics when `image`, drawn by `who`, is not the golden of `case` within its tolerance, naming the first pixel that is
/// not, or when a probe of the case does not hold. With `GNOBLIN_UPDATE_GOLDEN` naming `who`, the probes are checked and
/// the file is written instead of compared.
pub fn check(case: &Case, who: &str, image: &Image) {
    assert_eq!(image.size(), SIZE, "{who}: the picture has the scene's size");
    (case.probes)(image);
    if updating(who) {
        let path = golden_path(case.name);
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the golden directory");
        std::fs::write(&path, render_text(image)).expect("the golden file");
        return;
    }
    for (index, want) in golden(case).iter().enumerate() {
        let (x, y) = (index as u32 % SIZE.width, index as u32 / SIZE.width);
        let got = image.colour_at(x as i32, y as i32).expect("a pixel of the picture");
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| got.abs_diff(*want) <= case.tolerance),
            "{who} differs from effects_{}.txt at ({x}, {y}): drew {got:?}, golden {want:?}",
            case.name
        );
    }
}

/// Draws every case with `draw` and checks each picture. `draw` returns `None` when this machine cannot draw (no
/// device); the cases are then skipped, loudly, which is the only case a skip is allowed.
pub fn check_all(who: &str, mut draw: impl FnMut(&OutputFrame) -> Option<Image>) {
    for case in &CASES {
        let Some(image) = draw(&frame_of(case)) else {
            println!("skipping the {who} goldens: this machine has no device for it");
            return;
        };
        check(case, who, &image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The goldens are not blank and not alike: a case that drew nothing, or the same picture as another, is a golden
    /// that pins nothing.
    #[test]
    fn each_golden_differs_from_the_plain_backdrop_and_from_the_others() {
        let plain_pixels: Vec<[u8; 4]> = (0..SIZE.width * SIZE.height)
            .map(|index| {
                if index % SIZE.width < 16 {
                    [30, 60, 160, 255]
                } else {
                    [240, 230, 210, 255]
                }
            })
            .collect();
        let goldens: Vec<(&str, Vec<[u8; 4]>)> = CASES.iter().map(|case| (case.name, golden(case))).collect();
        for (name, golden) in &goldens {
            assert_ne!(*golden, plain_pixels, "{name} draws something over the backdrop");
            for (other, other_golden) in &goldens {
                assert!(
                    name == other || golden != other_golden,
                    "{name} and {other} are one picture"
                );
            }
        }
    }

    #[test]
    fn every_golden_passes_its_own_probes() {
        for case in &CASES {
            let data: Vec<u8> = golden(case).into_iter().flatten().collect();
            let image = Image::new(SIZE, SIZE.width as usize * 4, data).expect("an image of the golden");
            (case.probes)(&image);
        }
    }
}
