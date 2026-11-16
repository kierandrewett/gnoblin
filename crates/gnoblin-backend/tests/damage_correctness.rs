//! The compositor's damage tracking is correct when a frame repainted from the damage equals a frame repainted whole.
//!
//! A renderer repaints only the rectangles a frame names and keeps every other pixel of its buffer, so a rectangle
//! the composer forgets to damage leaves the old pixels on screen for good: a window that moved leaves its old place
//! behind, a closed one stays up, a faded one keeps its old opacity. The damage is allowed to be too big and never too
//! small, which is checkable without knowing anything about where windows are: draw the same scene twice, once with
//! the damage the composer reported into a buffer that has lived through every earlier frame and once whole into a
//! second buffer, and the two must be the same picture.
//!
//! Each scenario is a seeded random walk over a scene of flat-coloured windows and a pointer: windows open, move,
//! resize, close, hide and show, change opacity and corner radius, take a new picture or a new place in the stack, and
//! the cursor moves, hides and comes back. After every step both buffers are read back and compared pixel for pixel.
//! A difference is a bug in the damage, and the failure names the seed and the steps that led to it, which replay it.
//!
//! The software renderer is the reference and is compared exactly. The GLES renderer (llvmpipe here) and the Vulkan
//! renderer (lavapipe) run the same walks, each against itself, so what differs is only what the damage changed.

use std::fmt::Write as _;
use std::sync::Arc;

use gnoblin_backend::render::compose::{Composer, Content};
use gnoblin_backend::render::renderer::{AnyRenderer, RendererKind};
use gnoblin_render_frame::frame::{OutputFrame, Overlay, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Point, Rect, RectF, Scale, Size, WindowId};
use smithay::utils::Transform;

/// The output's logical size: small enough that a walk is quick on a software GL, big enough for several windows.
const WIDTH: u32 = 192;
const HEIGHT: u32 = 128;

/// The cursor's edge in logical pixels.
const CURSOR: u32 = 10;

/// A xorshift generator: the walks must be the same on every machine, and the crate has no random-number dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // A zero state would stay zero; the constant spreads small seeds apart.
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: u32) -> u32 {
        u32::try_from(self.next() % u64::from(bound.max(1))).unwrap_or(0)
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: i32, high: i32) -> i32 {
        low + i32::try_from(self.below(u32::try_from(high - low + 1).unwrap_or(1))).unwrap_or(0)
    }

    fn unit(&mut self) -> f32 {
        self.below(1001) as f32 / 1000.0
    }
}

/// A picture a window can show: flat in the middle with a differently coloured border, so a moved or rescaled window
/// differs from where it was at its edges as well as in its colour, and part of it is translucent.
fn picture(rng: &mut Rng) -> Arc<Image> {
    let size = Size::new(16, 12);
    let fill = [rng.below(256) as u8, rng.below(256) as u8, rng.below(256) as u8];
    let edge = [255 - fill[0], 255 - fill[1], 255 - fill[2]];
    let alpha = if rng.below(4) == 0 { 140 } else { 255 };
    let mut data = Vec::with_capacity(size.width as usize * size.height as usize * 4);
    for y in 0..size.height {
        for x in 0..size.width {
            let border = x == 0 || y == 0 || x + 1 == size.width || y + 1 == size.height;
            let rgb = if border { edge } else { fill };
            // Premultiplied, as a snapshot is.
            for channel in rgb {
                data.push((u32::from(channel) * alpha / 255) as u8);
            }
            data.push(alpha as u8);
        }
    }
    Arc::new(Image::new(size, size.width as usize * 4, data).expect("an image of that size"))
}

/// The cursor's picture: an opaque white square.
fn cursor_overlay(at: Rect) -> Option<Overlay> {
    let pixels = [255_u8; 4].repeat((CURSOR * CURSOR) as usize);
    let image = Image::new(Size::new(CURSOR, CURSOR), CURSOR as usize * 4, pixels)?;
    Overlay::new(at, &image)
}

/// The scene, the buffer that is repainted from damage, and the buffer that is repainted whole.
struct Rig {
    composer: Composer,
    output: OutputId,
    scale: Scale,
    incremental: AnyRenderer,
    reference: AnyRenderer,
    frame: OutputFrame,
    windows: Vec<(WindowId, Rect)>,
    next: u64,
    cursor: Option<Rect>,
    /// The steps taken so far, for the failure message.
    log: String,
}

impl Rig {
    /// A rig on `kind` at `scale`, or `None` where this machine cannot create the renderer.
    fn new(kind: RendererKind, scale: Scale) -> Option<Self> {
        let physical = Size::new(
            (f64::from(WIDTH) * scale.as_f64()).round() as u32,
            (f64::from(HEIGHT) * scale.as_f64()).round() as u32,
        );
        let incremental = AnyRenderer::new(kind, physical).ok()?;
        let reference = AnyRenderer::new(kind, physical).ok()?;
        let mut composer = Composer::new();
        // A fade would keep damaging after the walk's last step, and its timing is not what is under test.
        composer.set_reduced_motion(true);
        let output = OutputId::new(0);
        composer.set_output(output, Rect::new(0, 0, WIDTH, HEIGHT));
        composer.damage_output(output);
        Some(Self {
            composer,
            output,
            scale,
            incremental,
            reference,
            frame: OutputFrame::new(),
            windows: Vec::new(),
            next: 1,
            cursor: None,
            log: String::new(),
        })
    }

    fn note(&mut self, what: impl std::fmt::Display) {
        let _ = writeln!(self.log, "  {what}");
    }

    /// One random change to the scene.
    fn step(&mut self, rng: &mut Rng) {
        let count = self.windows.len();
        match rng.below(11) {
            0 | 1 if count < 6 => self.open(rng),
            2 if count > 0 => self.relocate(rng, false),
            3 if count > 0 => self.relocate(rng, true),
            4 if count > 0 => self.close(rng),
            5 if count > 0 => {
                let (window, _) = self.windows[rng.below(count as u32) as usize];
                let opacity = rng.unit();
                self.note(format!("opacity {window:?} {opacity}"));
                self.composer.set_window_opacity(window, opacity);
            }
            6 => self.move_cursor(rng),
            7 if count > 1 => {
                let mut order: Vec<WindowId> = self.windows.iter().map(|(window, _)| *window).collect();
                for index in (1..order.len()).rev() {
                    order.swap(index, rng.below(index as u32 + 1) as usize);
                }
                self.note(format!("stacking {order:?}"));
                self.composer.set_stacking(&order);
            }
            8 if count > 0 => {
                let (window, _) = self.windows[rng.below(count as u32) as usize];
                let visible = rng.below(2) == 0;
                self.note(format!("visible {window:?} {visible}"));
                self.composer.set_window_visible(window, visible);
            }
            9 if count > 0 => {
                let (window, area) = self.windows[rng.below(count as u32) as usize];
                self.note(format!("commit {window:?}"));
                self.composer.set_window_content(window, Some(content(area.size())));
                self.composer
                    .set_window_source(window, Some(SurfaceSource::Snapshot(picture(rng))));
            }
            10 if count > 0 => {
                let (window, _) = self.windows[rng.below(count as u32) as usize];
                let radius = rng.below(14) as f32;
                self.note(format!("corner radius {window:?} {radius}"));
                self.composer.set_window_corner_radius(window, radius);
            }
            _ => self.move_cursor(rng),
        }
    }

    fn open(&mut self, rng: &mut Rng) {
        let window = WindowId::new(self.next);
        self.next += 1;
        let area = Rect::new(
            rng.between(-20, WIDTH as i32 - 10),
            rng.between(-20, HEIGHT as i32 - 10),
            u32::try_from(rng.between(20, 90)).unwrap_or(20),
            u32::try_from(rng.between(16, 70)).unwrap_or(16),
        );
        self.note(format!("open {window:?} at {area:?}"));
        self.composer.insert_window(window, area);
        self.composer.set_window_content(window, Some(content(area.size())));
        self.composer
            .set_window_source(window, Some(SurfaceSource::Snapshot(picture(rng))));
        self.windows.push((window, area));
    }

    /// Moves a window, and with `resize` gives it a new size too, as a client that follows its configure does.
    fn relocate(&mut self, rng: &mut Rng, resize: bool) {
        let index = rng.below(self.windows.len() as u32) as usize;
        let (window, old) = self.windows[index];
        let size = if resize {
            Size::new(
                u32::try_from(rng.between(20, 90)).unwrap_or(20),
                u32::try_from(rng.between(16, 70)).unwrap_or(16),
            )
        } else {
            old.size()
        };
        let area = Rect::new(
            rng.between(-20, WIDTH as i32 - 10),
            rng.between(-20, HEIGHT as i32 - 10),
            size.width,
            size.height,
        );
        self.note(format!(
            "{} {window:?} {old:?} -> {area:?}",
            if resize { "resize" } else { "move" }
        ));
        self.composer.place_window(window, area);
        if resize {
            self.composer.set_window_content(window, Some(content(size)));
        }
        self.windows[index].1 = area;
    }

    fn close(&mut self, rng: &mut Rng) {
        let index = rng.below(self.windows.len() as u32) as usize;
        let (window, _) = self.windows.remove(index);
        if rng.below(2) == 0 {
            self.note(format!("close {window:?}"));
            self.composer.close_window(window);
        } else {
            self.note(format!("remove {window:?}"));
            self.composer.remove_window(window);
        }
    }

    fn move_cursor(&mut self, rng: &mut Rng) {
        self.cursor = if rng.below(6) == 0 {
            None
        } else {
            Some(Rect::new(
                rng.between(-4, WIDTH as i32 - 1),
                rng.between(-4, HEIGHT as i32 - 1),
                CURSOR,
                CURSOR,
            ))
        };
        self.note(format!("cursor {:?}", self.cursor));
    }

    /// Describes the output as the next frame and draws it into `renderer`.
    fn draw(&mut self, whole: bool) -> Result<(), String> {
        self.composer.set_cursor(self.cursor, false);
        if whole {
            self.composer.damage_output(self.output);
        }
        self.composer.begin_frame();
        let changed = self.composer.compose(self.output, &mut self.frame);
        if changed {
            self.frame.set_cursor_image(self.cursor.and_then(cursor_overlay));
            let renderer = if whole {
                &mut self.reference
            } else {
                &mut self.incremental
            };
            renderer
                .draw(&self.frame, self.scale)
                .map_err(|error| format!("the renderer refused the frame: {error:?}"))?;
        }
        self.composer.end_frame();
        Ok(())
    }

    /// Draws the frame twice, from the damage and whole, and returns the first pixel where they differ.
    fn compare(&mut self, tolerance: i32) -> Result<Option<Difference>, String> {
        self.draw(false)?;
        self.draw(true)?;
        let seen = self.incremental.capture(true).map_err(|error| format!("{error:?}"))?;
        let expected = self.reference.capture(true).map_err(|error| format!("{error:?}"))?;
        Ok(first_difference(&seen, &expected, tolerance))
    }
}

/// Where a repainted frame and a whole repaint first disagree.
struct Difference {
    at: Point,
    seen: [u8; 4],
    expected: [u8; 4],
}

fn content(size: Size) -> Content {
    Content {
        src: RectF::new(0.0, 0.0, 16.0, 12.0),
        size,
        transform: Transform::Normal,
    }
}

/// The first pixel where two pictures of one size differ by more than `tolerance` in a channel.
fn first_difference(seen: &Image, expected: &Image, tolerance: i32) -> Option<Difference> {
    assert_eq!(seen.size(), expected.size(), "both buffers are the output's size");
    let size = seen.size();
    for y in 0..size.height {
        for x in 0..size.width {
            let at = Point::new(x as i32, y as i32);
            let (a, b) = (seen.pixel(at)?, expected.pixel(at)?);
            if a.iter()
                .zip(&b)
                .any(|(a, b)| (i32::from(*a) - i32::from(*b)).abs() > tolerance)
            {
                return Some(Difference {
                    at,
                    seen: a,
                    expected: b,
                });
            }
        }
    }
    None
}

/// Runs `seeds` walks of `steps` steps each on `kind` at `scale`.
fn walk(kind: RendererKind, scale: Scale, seeds: u64, steps: u32, tolerance: i32) {
    for seed in 1..=seeds {
        let Some(mut rig) = Rig::new(kind, scale) else {
            println!(
                "skipping the {} damage walk: this machine cannot create the renderer",
                kind.name()
            );
            return;
        };
        let mut rng = Rng::new(seed);
        for step in 0..steps {
            // A step is one to three changes between two frames, as a turn of the loop applies several.
            for _ in 0..=rng.below(3) {
                rig.step(&mut rng);
            }
            let mismatch = rig
                .compare(tolerance)
                .unwrap_or_else(|error| panic!("{} seed {seed} step {step}: {error}", kind.name()));
            if let Some(Difference { at, seen, expected }) = mismatch {
                panic!(
                    "{} at scale {scale:?}, seed {seed}, step {step}: the repainted frame shows {seen:?} at {at:?} where a whole \
                     repaint shows {expected:?}; the steps so far:\n{}",
                    kind.name(),
                    rig.log
                );
            }
        }
    }
}

#[test]
fn software_damage_matches_a_whole_repaint() {
    walk(RendererKind::Software, Scale::IDENTITY, 60, 30, 0);
}

#[test]
fn software_damage_matches_a_whole_repaint_at_scale_two() {
    walk(RendererKind::Software, Scale::integral(2), 24, 30, 0);
}

#[test]
fn software_damage_matches_a_whole_repaint_at_a_fractional_scale() {
    walk(RendererKind::Software, Scale::new(3, 2), 24, 30, 0);
}

#[test]
fn gles_damage_matches_a_whole_repaint() {
    walk(RendererKind::Gles, Scale::IDENTITY, 10, 24, 0);
    walk(RendererKind::Gles, Scale::integral(2), 4, 24, 0);
}

#[test]
fn vulkan_damage_matches_a_whole_repaint() {
    walk(RendererKind::Vulkan, Scale::IDENTITY, 6, 20, 0);
}
