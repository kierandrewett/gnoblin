//! Golden pixels for what a node's `effects` list and its colour properties draw.
//!
//! A node's `effects` list (blur and shadow) is the only effects path, and `tint` and `opacity` are the layer's own
//! colour properties. Each case below renders one small fixed scene and compares every pixel with a picture checked
//! in under `tests/golden/`, so a change to a blur kernel, a shadow falloff, a tint or the group-opacity rule shows as
//! a diff in a file, not as a probe that happens to still hold.
//!
//! A golden is text, one row of the picture a line and one pixel a word of eight hex digits (`rrggbbaa`), so a diff
//! can be read. A pixel matches within [`TOLERANCE`] on every channel: one rounding step of a GLES driver differs
//! from another's, and the check is for the effect, not for one driver's arithmetic. `GNOBLIN_UPDATE_GOLDEN=1` writes
//! the files from the GLES renderer; read the diff before committing it.
//!
//! Tint and opacity are properties the software renderer draws too, so those goldens are checked against it as well:
//! the two renderers agree on one picture. Blur and shadow are GLES's alone (the software renderer reports them
//! `unsupported-by-renderer` in the node's `skipped` list) and a case that needs one is checked against GLES only.
//!
//! A machine with no GLES context skips the GLES half of each case, as the rest of this crate's tests do.

use std::path::PathBuf;
use std::sync::Arc;

use super::tests::gles;
use super::*;
use gnoblin_render::{BlurParams, Effects, Shadow, Vec3};
use gnoblin_render_frame::frame::Solid;
use gnoblin_render_software::Software;

/// The most a channel of a pixel may differ from its golden.
const TOLERANCE: u8 = 3;
/// The size of every golden scene, in pixels.
const SIZE: Size = Size::new(32, 24);
/// Where the window sits in every scene.
const WINDOW: Rect = Rect::new(8, 7, 16, 9);

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

/// One named scene: the layers that go over the backdrop, and whether the software renderer draws them as well.
struct Case {
    name: &'static str,
    layers: fn() -> Vec<Layer>,
    software: bool,
}

const CASES: [Case; 5] = [
    Case {
        name: "shadow",
        layers: || vec![window(one(shadow()), Color::WHITE, 1.0)],
        software: false,
    },
    Case {
        name: "blur",
        layers: || vec![glass(), window(Effects::new(), Color::WHITE, 1.0)],
        software: false,
    },
    Case {
        name: "tint",
        layers: || vec![window(Effects::new(), TINT, 1.0)],
        software: true,
    },
    Case {
        name: "opacity",
        layers: || vec![window(Effects::new(), Color::WHITE, 0.5)],
        software: true,
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
    },
];

fn frame_of(case: &Case) -> OutputFrame {
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
fn render_text(image: &Image) -> String {
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
fn parse(text: &str) -> Vec<[u8; 4]> {
    text.split_whitespace()
        .map(|word| {
            let channel = |at: usize| u8::from_str_radix(&word[at..at + 2], 16).expect("two hex digits");
            [channel(0), channel(2), channel(4), channel(6)]
        })
        .collect()
}

/// Panics when `image` is not the golden `name` within [`TOLERANCE`], naming the first pixel that is not.
fn assert_golden(name: &str, who: &str, image: &Image) {
    assert_eq!(image.size(), SIZE, "{who}: the picture has the scene's size");
    let path = golden_path(name);
    if std::env::var_os("GNOBLIN_UPDATE_GOLDEN").is_some() && who == "GLES" {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("the golden directory");
        std::fs::write(&path, render_text(image)).expect("the golden file");
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with GNOBLIN_UPDATE_GOLDEN=1", path.display()));
    let golden = parse(&text);
    assert_eq!(
        golden.len(),
        (SIZE.width * SIZE.height) as usize,
        "{name}: the golden is a whole picture"
    );
    for (index, want) in golden.iter().enumerate() {
        let (x, y) = (index as u32 % SIZE.width, index as u32 / SIZE.width);
        let got = image.colour_at(x as i32, y as i32).expect("a pixel of the picture");
        assert!(
            got.iter().zip(want).all(|(got, want)| got.abs_diff(*want) <= TOLERANCE),
            "{who} differs from effects_{name}.txt at ({x}, {y}): drew {got:?}, golden {want:?}"
        );
    }
}

fn gles_picture(case: &Case) -> Option<Image> {
    let mut renderer = gles(SIZE)?;
    renderer.draw(&frame_of(case), Scale::IDENTITY).expect("drawn");
    Some(renderer.screenshot().expect("a readback"))
}

fn software_picture(case: &Case) -> Image {
    let mut renderer = Software::new(SIZE).expect("a software renderer");
    renderer.draw(&frame_of(case), Scale::IDENTITY).expect("drawn");
    renderer.screenshot().expect("a readback")
}

#[test]
fn the_effect_scenes_match_their_goldens_on_gles() {
    for case in &CASES {
        let Some(image) = gles_picture(case) else {
            return;
        };
        assert_golden(case.name, "GLES", &image);
    }
}

#[test]
fn tint_and_opacity_match_the_same_goldens_on_the_software_renderer() {
    for case in CASES.iter().filter(|case| case.software) {
        assert_golden(case.name, "software", &software_picture(case));
    }
}

/// The goldens are not blank and not alike: a case that drew nothing, or the same picture as another, is a golden that
/// pins nothing.
#[test]
fn each_effect_golden_differs_from_the_plain_backdrop_and_from_the_others() {
    let plain = parse(&render_text(&software_picture(&Case {
        name: "plain",
        layers: Vec::new,
        software: true,
    })));
    let goldens: Vec<(&str, Vec<[u8; 4]>)> = CASES
        .iter()
        .map(|case| {
            let text = std::fs::read_to_string(golden_path(case.name))
                .unwrap_or_else(|_| panic!("effects_{}.txt is missing; run with GNOBLIN_UPDATE_GOLDEN=1", case.name));
            (case.name, parse(&text))
        })
        .collect();
    for (name, golden) in &goldens {
        assert_ne!(*golden, plain, "{name} draws something over the backdrop");
        for (other, other_golden) in &goldens {
            assert!(
                name == other || golden != other_golden,
                "{name} and {other} are one picture"
            );
        }
    }
}
