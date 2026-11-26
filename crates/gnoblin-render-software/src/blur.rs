//! The backdrop blur, on the CPU.
//!
//! It is the GLES blur written out: `wgsl/blur.wgsl` is nine taps of a Gaussian of sigma 1.5 taps, spaced `radius / 3`
//! pixels apart and read through linear filtering, run horizontally and then vertically, `passes` times over. The shader
//! reads between texels, so every tap is a weighted pair of texels; because a tap's fractional offset is the same for
//! every pixel, the nine taps fold into one finite filter over whole-pixel offsets, and the sweep is a plain
//! convolution with a clamped edge, which is what the shader's `CLAMP_TO_EDGE` sampler does too. That is the same
//! arithmetic as the GPU's up to the rounding of its 8-bit buffer, which this code imitates by rounding after every sweep.
//!
//! The last vertical sweep is the one that grades: saturate, brightness, contrast, then the tint mixed over the result
//! by its alpha, then the grain, using `gnoblin_render_frame::grade`, the same function the GPU's shader is checked
//! against, so a graded backdrop has the same colours on every renderer.
//!
//! # Downscale
//!
//! A blur with `downscale` d below 1 is the GLES blur's own reduction (`effects.rs`, "The backdrop blur's downscale and
//! grain"). The region the blur reads, which is the layer's rectangle grown by the kernel's reach and clipped to the
//! output, is halved through a bilinear tap (half, rounded up, never below the goal) until it is `ceil(size * d)` on a
//! side. The sweeps run there with the tap step `radius / 3 * d`, and the result is read back up to the region's pixels
//! through a bilinear tap at each pixel's centre, where it is graded. Every bilinear tap keeps its fraction in 1/256, as
//! a GPU does, and every picture is rounded to bytes, as the GPU's targets are. The picture the sweeps run on is the
//! region's own, so a downscaled blur is a function of the region and not of the damage; [`Blur::reduces`] says when the
//! caller must hand [`Blur::run`] the whole region.
//!
//! # Cost
//!
//! A sweep is a handful of multiply-adds a channel a pixel, written as loops over whole rows so the compiler can
//! vectorise them. A frame redraws the blur only inside the damage, plus the reach of the kernel around it.

use std::collections::BTreeMap;

use gnoblin_render::MIN_BLUR_DOWNSCALE;
use gnoblin_render_frame::grade::{self, Grade};
use gnoblin_types::Color;

use crate::raster::{Area, Patch};

/// The taps of one sweep: how many of them there are either side of the centre.
const HALF_TAPS: i32 = 4;

/// The shader's tap weight, `exp(-0.5 * i * i / 2.25) / 3.7514`: a Gaussian of sigma 1.5 taps, normalised.
fn weight(index: i32) -> f32 {
    (-0.5 * (index * index) as f32 / 2.25).exp() / 3.7514
}

/// The finite filter of one sweep: pairs of a whole-pixel offset and the weight read there.
///
/// A tap at `index * step` pixels reads the two texels either side of it, weighted by how near each is. A GPU keeps
/// that fraction in eight bits, so it is rounded to 1/256 here.
fn kernel(step: f32) -> Vec<(i32, f32)> {
    let mut taps: BTreeMap<i32, f32> = BTreeMap::new();
    for index in -HALF_TAPS..=HALF_TAPS {
        let distance = index as f32 * step;
        let floor = distance.floor();
        let fraction = ((distance - floor) * 256.0).round() / 256.0;
        let at = floor as i32;
        *taps.entry(at).or_default() += weight(index) * (1.0 - fraction);
        *taps.entry(at + 1).or_default() += weight(index) * fraction;
    }
    taps.into_iter().filter(|(_, weight)| *weight > 0.0).collect()
}

/// A blur and how its result is graded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Blur {
    /// How many times the pair of sweeps runs.
    pub(crate) passes: u32,
    /// The blur radius in physical pixels, at full resolution.
    radius: f32,
    /// The fraction of full resolution the sweeps run at, in `MIN_BLUR_DOWNSCALE..=1`.
    downscale: f32,
    /// The grade of the result: saturate, brightness, contrast and the grain.
    pub(crate) grade: Grade,
    /// A colour mixed over the graded result by its alpha.
    pub(crate) tint: Color,
}

impl Blur {
    /// The blur of a layer with the given radius in physical pixels, `passes` passes, the grade and the tint, at
    /// `downscale` of full resolution.
    pub(crate) fn new(radius: f32, passes: u32, downscale: f32, grade: Grade, tint: Color) -> Self {
        Self {
            passes,
            radius,
            downscale: downscale.clamp(MIN_BLUR_DOWNSCALE, 1.0),
            grade,
            tint,
        }
    }

    /// The same blur with its downscale undone, for a caller that hands [`Blur::run`] a patch that is not the whole region.
    pub(crate) fn at_full_resolution(self) -> Self {
        Self { downscale: 1.0, ..self }
    }

    /// The picture the sweeps run on for a region of `size`: `ceil(size * downscale)` on a side, at least a pixel and at
    /// most the region's own size.
    fn small(&self, size: (usize, usize)) -> (usize, usize) {
        let scaled = |side: usize| ((side as f32 * self.downscale).ceil() as usize).clamp(1, side.max(1));
        (scaled(size.0), scaled(size.1))
    }

    /// Whether the sweeps run on a picture smaller than a region of `size`. Such a blur is a function of the whole
    /// region, so the caller reads [`Blur::region`] and not less.
    pub(crate) fn reduces(&self, size: (i32, i32)) -> bool {
        let size = (size.0.max(0) as usize, size.1.max(0) as usize);
        self.small(size) != size
    }

    /// The tap step of the sweeps, in pixels of the picture they run on: a third of the radius, at least half a pixel,
    /// and in a reduced picture scaled with it so the blur's width on the output does not depend on the downscale.
    fn step(&self, reduced: bool) -> f32 {
        let step = (self.radius / 3.0).max(0.5);
        if reduced { step * self.downscale } else { step }
    }

    /// The region the GLES renderer copies out for a layer at `rect`, which lies inside `bounds`: `rect` grown by the
    /// reach of every sweep, clipped to `bounds`.
    pub(crate) fn region(&self, rect: Area, bounds: Area) -> Option<Area> {
        let reach = self.passes as i32 * ((self.radius / 3.0 * 4.0).ceil() as i32 + 1);
        crate::raster::grown(rect, reach).intersection(bounds)
    }

    /// How far a sweep reaches either side of a pixel at full resolution, in pixels.
    fn reach(&self) -> i32 {
        kernel(self.step(false))
            .iter()
            .map(|(offset, _)| offset.abs())
            .max()
            .unwrap_or(0)
    }

    /// How far the blur of a pixel depends on pixels around it at full resolution: every sweep reaches as far again.
    pub(crate) fn margin(&self) -> i32 {
        self.reach() * self.passes as i32 + 2
    }

    /// Blurs a filter's picture of premultiplied pixels with one horizontal and one vertical sweep, as the GLES filter
    /// does (`effects.rs`, `picture_into`): the tap step is `radius / 3`, at least half a pixel, the edge is clamped, and
    /// nothing is graded here (the composition grades the picture as a whole). The picture is made with a margin of the
    /// sweep's reach around what it paints, so its clamped edge is transparent.
    pub(crate) fn sweep_picture(picture: &mut Patch, radius: f32) {
        let size = (picture.rect.size.w as usize, picture.rect.size.h as usize);
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let taps = kernel((radius / 3.0).max(0.5));
        let pad = taps
            .iter()
            .map(|(offset, _)| offset.unsigned_abs() as usize)
            .max()
            .unwrap_or(0);
        let mut current: Vec<f32> = picture.bytes.iter().map(|byte| f32::from(*byte)).collect();
        let mut next = vec![0.0_f32; current.len()];
        let mut padded = vec![0.0_f32; (size.0 + 2 * pad) * 4];
        horizontal(&current, &mut next, size, &taps, &mut padded, true);
        vertical(&next, &mut current, size, &taps, true);
        for (byte, value) in picture.bytes.iter_mut().zip(&current) {
            *byte = value.clamp(0.0, 255.0) as u8;
        }
    }

    /// Blurs `patch` and writes the result into the pixels of `keep`, which must lie inside it; the pixels of the patch
    /// outside `keep` are the margin the blur reads, and are left as they were.
    ///
    /// `origin` is the layer's top-left corner on the output: the grain is a function of the distance from it.
    ///
    /// At full resolution the blur of a pixel is the same whatever part of the frame is being redrawn, as long as the
    /// patch holds [`Blur::margin`] pixels around `keep`. A reduced blur is the blur of the patch as a whole.
    pub(crate) fn run(&self, patch: &mut Patch, keep: Area, origin: (i32, i32)) {
        let size = (patch.rect.size.w as usize, patch.rect.size.h as usize);
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let small = self.small(size);
        let mut current: Vec<f32> = patch.bytes.iter().map(|byte| f32::from(*byte)).collect();
        let mut at = size;
        while at != small {
            let next = (halved(at.0, small.0), halved(at.1, small.1));
            current = resample(&current, at, next);
            at = next;
        }
        let taps = kernel(self.step(small != size));
        let pad = taps
            .iter()
            .map(|(offset, _)| offset.unsigned_abs() as usize)
            .max()
            .unwrap_or(0);
        let mut next = vec![0.0_f32; current.len()];
        let mut padded = vec![0.0_f32; (at.0 + 2 * pad) * 4];
        for _ in 0..self.passes {
            horizontal(&current, &mut next, at, &taps, &mut padded, true);
            vertical(&next, &mut current, at, &taps, true);
        }
        self.finish(patch, &current, at, keep, origin);
    }

    /// Samples the blurred picture, `small` texels across and down, back up to the pixels of `keep`, grades them, and
    /// stores them in `patch`.
    fn finish(&self, patch: &mut Patch, blurred: &[f32], small: (usize, usize), keep: Area, origin: (i32, i32)) {
        let width = patch.rect.size.w as usize;
        let height = patch.rect.size.h as usize;
        let graded = !self.grade.is_identity();
        let tinted = self.tint.a > 0.0;
        let grained = self.grade.noise > 0.0;
        let (left_of_keep, top_of_keep) = (
            (keep.loc.x - patch.rect.loc.x) as usize,
            (keep.loc.y - patch.rect.loc.y) as usize,
        );
        // Where each column of `keep` reads the blurred picture: the two texels either side of its centre and the share
        // of the second. The rows are worked out as they come.
        let columns: Vec<(usize, usize, f32)> = (0..keep.size.w as usize)
            .map(|column| axis_at(small.0, width, left_of_keep + column))
            .collect();
        // The row of the blurred picture a row of `keep` reads, mixed down from the two texel rows either side of it.
        let mut line = vec![0.0_f32; small.0 * 4];
        for row in 0..keep.size.h as usize {
            let y = keep.loc.y + row as i32;
            let (top, bottom, share_y) = axis_at(small.1, height, top_of_keep + row);
            let (upper, lower) = (
                &blurred[top * small.0 * 4..(top + 1) * small.0 * 4],
                &blurred[bottom * small.0 * 4..(bottom + 1) * small.0 * 4],
            );
            for ((mixed, above), below) in line.iter_mut().zip(upper).zip(lower) {
                *mixed = (above + (below - above) * share_y) / 255.0;
            }
            let start = ((top_of_keep + row) * width + left_of_keep) * 4;
            let out = &mut patch.bytes[start..start + columns.len() * 4];
            for ((x, (left, right, share_x)), pixel_out) in
                (keep.loc.x..).zip(&columns).zip(out.as_chunks_mut::<4>().0.iter_mut())
            {
                let (a, b) = (&line[left * 4..left * 4 + 4], &line[right * 4..right * 4 + 4]);
                let mut pixel = [0, 1, 2, 3].map(|c| a[c] + (b[c] - a[c]) * share_x);
                if graded || tinted || grained {
                    let mut colour = Color::new(pixel[0], pixel[1], pixel[2], pixel[3]);
                    colour = grade::apply(self.grade, colour);
                    let mix = self.tint.a.clamp(0.0, 1.0);
                    let grain = if grained {
                        grade::grain(
                            self.grade.noise,
                            (x - origin.0).max(0) as u32,
                            (y - origin.1).max(0) as u32,
                        )
                    } else {
                        0.0
                    };
                    let channel = |value: f32, tint: f32| (value + (tint - value) * mix + grain).clamp(0.0, 1.0);
                    pixel = [
                        channel(colour.r, self.tint.r),
                        channel(colour.g, self.tint.g),
                        channel(colour.b, self.tint.b),
                        pixel[3],
                    ];
                }
                for (byte, channel) in pixel_out.iter_mut().zip(pixel) {
                    *byte = crate::raster::to_byte(channel);
                }
            }
        }
    }
}

/// The size one halving of `size` reaches towards `goal`: half, rounded up, but not below the goal.
fn halved(size: usize, goal: usize) -> usize {
    if size > goal { size.div_ceil(2).max(goal) } else { size }
}

/// Where texel `index` of a picture `to` texels long reads a picture `from` texels long along one axis, through a
/// bilinear tap at its centre: the two texels either side of that point, clamped to the picture, and the share of the
/// second, kept in 1/256 as a GPU keeps it.
fn axis_at(from: usize, to: usize, index: usize) -> (usize, usize, f32) {
    let at = (index as f32 + 0.5) * from as f32 / to as f32 - 0.5;
    let first = at.floor();
    let share = ((at - first) * 256.0).round() / 256.0;
    let last = from as i32 - 1;
    (
        (first as i32).clamp(0, last) as usize,
        (first as i32 + 1).clamp(0, last) as usize,
        share,
    )
}

/// `source`, `from` texels across and down, as a picture of `to` texels read through a bilinear tap at the centre of each
/// of its texels and rounded to bytes: one step of the GLES blur's reduction.
fn resample(source: &[f32], from: (usize, usize), to: (usize, usize)) -> Vec<f32> {
    let mut out = vec![0.0_f32; to.0 * to.1 * 4];
    if from.0 == to.0 * 2 && from.1 == to.1 * 2 {
        // An exact halving taps the middle of every 2x2 block, which is the block's mean.
        let stride = from.0 * 4;
        for (row, texels) in out.chunks_exact_mut(to.0 * 4).enumerate() {
            let upper = &source[2 * row * stride..(2 * row + 1) * stride];
            let lower = &source[(2 * row + 1) * stride..(2 * row + 2) * stride];
            for ((texel, above), below) in texels
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(upper.as_chunks::<8>().0)
                .zip(lower.as_chunks::<8>().0)
            {
                for (channel, value) in texel.iter_mut().enumerate() {
                    let sum = above[channel] + above[channel + 4] + below[channel] + below[channel + 4];
                    *value = (sum * 0.25 + 0.5) as i32 as f32;
                }
            }
        }
        return out;
    }
    let columns: Vec<(usize, usize, f32)> = (0..to.0).map(|index| axis_at(from.0, to.0, index)).collect();
    // The row of the source a row of the result reads, mixed down from the two rows either side of it.
    let mut line = vec![0.0_f32; from.0 * 4];
    for (row, texels) in out.chunks_exact_mut(to.0 * 4).enumerate() {
        let (top, bottom, share_y) = axis_at(from.1, to.1, row);
        let upper = &source[top * from.0 * 4..(top + 1) * from.0 * 4];
        let lower = &source[bottom * from.0 * 4..(bottom + 1) * from.0 * 4];
        for ((mixed, above), below) in line.iter_mut().zip(upper).zip(lower) {
            *mixed = above + (below - above) * share_y;
        }
        for ((left, right, share_x), texel) in columns.iter().zip(texels.as_chunks_mut::<4>().0.iter_mut()) {
            let (a, b) = (&line[left * 4..left * 4 + 4], &line[right * 4..right * 4 + 4]);
            for (channel, value) in texel.iter_mut().enumerate() {
                *value = (a[channel] + (b[channel] - a[channel]) * share_x + 0.5) as i32 as f32;
            }
        }
    }
    out
}

/// The most taps a sweep's filter has: nine taps, each reading two whole-pixel offsets.
const MAX_TAPS: usize = 2 * (2 * HALF_TAPS as usize + 1);

/// How many floats `accumulate` keeps in registers at a time.
const LANES: usize = 16;

/// `out = sum of weight * tap` over the taps, where each tap is a slice at least as long as `out`.
///
/// The sum is made `LANES` floats at a time, held in an array the compiler keeps in registers, so every tap is read
/// once and `out` is written once, rather than `out` being read and written back by every tap.
fn accumulate(out: &mut [f32], taps: &[&[f32]], weights: &[f32]) {
    let (chunks, rest) = out.as_chunks_mut::<LANES>();
    let mut from = 0;
    for chunk in chunks {
        let mut sum = [0.0_f32; LANES];
        for (tap, weight) in taps.iter().zip(weights) {
            for (total, value) in sum.iter_mut().zip(&tap[from..from + LANES]) {
                *total += weight * value;
            }
        }
        *chunk = sum;
        from += LANES;
    }
    for (index, total) in rest.iter_mut().enumerate() {
        *total = taps
            .iter()
            .zip(weights)
            .map(|(tap, weight)| weight * tap[from + index])
            .sum();
    }
}

/// Rounds every value to a whole number, as the GPU's 8-bit buffer would store it. The values are never negative, so
/// adding a half and truncating rounds; `f32::round` is a library call on a baseline x86-64 build.
fn store(out: &mut [f32]) {
    for value in out {
        *value = (*value + 0.5) as i32 as f32;
    }
}

/// One horizontal sweep of `source` into `target`, both `size` pixels of four channels.
///
/// A row is padded by its first and last pixel, which is the clamped edge, so every tap reads inside the padded row and
/// the sum over a row has no branch in it.
fn horizontal(
    source: &[f32],
    target: &mut [f32],
    size: (usize, usize),
    taps: &[(i32, f32)],
    padded: &mut [f32],
    round: bool,
) {
    let (width, height) = size;
    let pad = taps
        .iter()
        .map(|(offset, _)| offset.unsigned_abs() as usize)
        .max()
        .unwrap_or(0);
    let weights: Vec<f32> = taps.iter().map(|(_, weight)| *weight).collect();
    for y in 0..height {
        let row = &source[y * width * 4..(y + 1) * width * 4];
        let out = &mut target[y * width * 4..(y + 1) * width * 4];
        for chunk in padded[..pad * 4].as_chunks_mut::<4>().0.iter_mut() {
            chunk.copy_from_slice(&row[..4]);
        }
        padded[pad * 4..(pad + width) * 4].copy_from_slice(row);
        for chunk in padded[(pad + width) * 4..(2 * pad + width) * 4]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
        {
            chunk.copy_from_slice(&row[(width - 1) * 4..]);
        }
        let mut reads: [&[f32]; MAX_TAPS] = [&[]; MAX_TAPS];
        for (read, (offset, _)) in reads.iter_mut().zip(taps) {
            let start = ((pad as i32 + offset) as usize) * 4;
            *read = &padded[start..start + width * 4];
        }
        accumulate(out, &reads[..taps.len()], &weights);
        if round {
            store(out);
        }
    }
}

/// One vertical sweep of `source` into `target`: a row of the result is the weighted sum of the rows the taps reach,
/// the rows past the top and bottom being the first and the last.
fn vertical(source: &[f32], target: &mut [f32], size: (usize, usize), taps: &[(i32, f32)], round: bool) {
    let (width, height) = size;
    let stride = width * 4;
    let weights: Vec<f32> = taps.iter().map(|(_, weight)| *weight).collect();
    for y in 0..height {
        let out = &mut target[y * stride..(y + 1) * stride];
        let mut reads: [&[f32]; MAX_TAPS] = [&[]; MAX_TAPS];
        for (read, (offset, _)) in reads.iter_mut().zip(taps) {
            let at = (y as i32 + offset).clamp(0, height as i32 - 1) as usize;
            *read = &source[at * stride..(at + 1) * stride];
        }
        accumulate(out, &reads[..taps.len()], &weights);
        if round {
            store(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::Rectangle;

    fn area(x: i32, y: i32, w: i32, h: i32) -> Area {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn patch(rect: Area, pixel: impl Fn(i32, i32) -> [u8; 4]) -> Patch {
        let mut bytes = Vec::new();
        for y in 0..rect.size.h {
            for x in 0..rect.size.w {
                bytes.extend_from_slice(&pixel(x, y));
            }
        }
        Patch { rect, bytes }
    }

    fn red(patch: &Patch, x: i32, y: i32) -> u8 {
        patch.bytes[((y * patch.rect.size.w + x) * 4) as usize]
    }

    #[test]
    fn the_kernel_weights_sum_to_the_shaders_normalisation() {
        for step in [0.5, 1.0, 1.333, 2.0, 5.5] {
            let sum: f32 = kernel(step).iter().map(|(_, weight)| weight).sum();
            assert!((sum - 1.0).abs() < 1e-3, "step {step} sums to {sum}");
        }
    }

    #[test]
    fn a_whole_pixel_step_reads_whole_texels() {
        let taps = kernel(2.0);
        assert_eq!(taps.len(), 9, "nine taps on nine texels");
        assert!(taps.iter().all(|(offset, _)| offset % 2 == 0));
    }

    #[test]
    fn a_flat_patch_stays_flat() {
        let rect = area(0, 0, 20, 12);
        let mut flat = patch(rect, |_, _| [90, 140, 200, 255]);
        Blur::new(6.0, 3, 1.0, Grade::IDENTITY, Color::TRANSPARENT).run(&mut flat, rect, (0, 0));
        assert!(flat.bytes.chunks(4).all(|pixel| pixel == [90, 140, 200, 255]));
    }

    #[test]
    fn an_edge_is_softened_and_the_mean_is_kept() {
        let rect = area(0, 0, 48, 8);
        let mut edge = patch(rect, |x, _| if x < 24 { [0, 0, 0, 255] } else { [240, 240, 240, 255] });
        Blur::new(8.0, 2, 1.0, Grade::IDENTITY, Color::TRANSPARENT).run(&mut edge, rect, (0, 0));
        let row: Vec<u8> = (0..48).map(|x| red(&edge, x, 4)).collect();
        assert!(
            row.windows(2).all(|pair| pair[0] <= pair[1]),
            "monotonic across the edge: {row:?}"
        );
        assert!(
            row[23] > 40 && row[24] < 200,
            "the edge is blurred: {} {}",
            row[23],
            row[24]
        );
        assert!(row[23].abs_diff(240 - row[24]) <= 2, "and symmetric about it");
        assert_eq!(row[0], 0);
        assert_eq!(row[47], 240);
    }

    #[test]
    fn only_the_kept_rectangle_is_written() {
        let rect = area(0, 0, 32, 8);
        let mut edge = patch(rect, |x, _| if x < 16 { [0, 0, 0, 255] } else { [240, 240, 240, 255] });
        Blur::new(8.0, 2, 1.0, Grade::IDENTITY, Color::TRANSPARENT).run(&mut edge, area(14, 0, 4, 8), (0, 0));
        assert_eq!(red(&edge, 4, 4), 0, "the margin keeps its own pixels");
        assert!(
            red(&edge, 15, 4) > 10 && red(&edge, 15, 4) < 230,
            "the kept pixels are blurred"
        );
    }

    #[test]
    fn the_last_sweep_grades_tints_and_grains() {
        let rect = area(0, 0, 8, 8);
        let flat = |colour: [u8; 4]| patch(rect, move |_, _| colour);
        // Contrast 0 turns every colour into mid grey.
        let mut grey = flat([10, 200, 90, 255]);
        let flatten = Grade {
            contrast: 0.0,
            ..Grade::IDENTITY
        };
        Blur::new(2.0, 1, 1.0, flatten, Color::TRANSPARENT).run(&mut grey, rect, (0, 0));
        assert_eq!(&grey.bytes[..4], [128, 128, 128, 255]);
        // A tint of alpha 0.5 mixes halfway to its colour.
        let mut tinted = flat([0, 0, 0, 255]);
        Blur::new(2.0, 1, 1.0, Grade::IDENTITY, Color::new(1.0, 1.0, 1.0, 0.5)).run(&mut tinted, rect, (0, 0));
        assert_eq!(&tinted.bytes[..4], [128, 128, 128, 255]);
        // Grain moves a flat colour by the amount `grade::grain` gives that pixel.
        let mut grained = flat([128, 128, 128, 255]);
        let noisy = Grade {
            noise: 1.0,
            ..Grade::IDENTITY
        };
        Blur::new(2.0, 1, 1.0, noisy, Color::TRANSPARENT).run(&mut grained, rect, (0, 0));
        for (x, y) in [(0, 0), (3, 5), (7, 2)] {
            let want = crate::raster::to_byte(128.0 / 255.0 + grade::grain(1.0, x as u32, y as u32));
            assert_eq!(red(&grained, x, y), want, "the grain at ({x}, {y})");
        }
    }

    #[test]
    fn the_grain_is_counted_from_the_layers_corner() {
        let rect = area(10, 10, 4, 4);
        let mut moved = patch(rect, |_, _| [128, 128, 128, 255]);
        let noisy = Grade {
            noise: 1.0,
            ..Grade::IDENTITY
        };
        Blur::new(2.0, 1, 1.0, noisy, Color::TRANSPARENT).run(&mut moved, rect, (10, 10));
        let want = crate::raster::to_byte(128.0 / 255.0 + grade::grain(1.0, 1, 2));
        assert_eq!(red(&moved, 1, 2), want);
    }

    #[test]
    fn a_blur_at_a_fraction_of_the_resolution_is_close_to_the_full_one() {
        let rect = area(0, 0, 96, 24);
        let edge = |x: i32, _| {
            if x < 48 {
                [10, 10, 10, 255]
            } else {
                [240, 200, 120, 255]
            }
        };
        let blur = |downscale: f32| {
            let mut picture = patch(rect, edge);
            Blur::new(24.0, 3, downscale, Grade::IDENTITY, Color::TRANSPARENT).run(&mut picture, rect, (0, 0));
            picture
        };
        let (full, half, quarter) = (blur(1.0), blur(0.5), blur(0.25));
        for (name, other) in [("half", &half), ("quarter", &quarter)] {
            let worst = full
                .bytes
                .iter()
                .zip(&other.bytes)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .expect("pixels");
            assert!(
                worst <= 16,
                "the {name} resolution blur is within {worst} levels of the full one: a step of 230 levels is the worst case"
            );
        }
    }

    /// The blur of a pixel does not depend on how much of the frame is redrawn around it: a patch cut to a small damage
    /// and the same pixels of a patch of the whole picture are the same.
    #[test]
    fn the_blur_of_a_pixel_does_not_depend_on_the_patch_it_is_made_in() {
        let whole = area(0, 0, 160, 40);
        let pixel = |x: i32, y: i32| {
            [
                (x * 7 % 256) as u8,
                (y * 13 % 256) as u8,
                ((x + y) * 3 % 256) as u8,
                255,
            ]
        };
        let blur = Blur::new(36.0, 2, 1.0, Grade::IDENTITY, Color::TRANSPARENT);
        let mut full = patch(whole, pixel);
        blur.run(&mut full, whole, (0, 0));
        for keep in [
            area(71, 9, 13, 11),
            area(64, 8, 8, 8),
            area(0, 0, 5, 5),
            area(150, 30, 10, 10),
        ] {
            let grown = Rectangle::new(
                (keep.loc.x - blur.margin(), keep.loc.y - blur.margin()).into(),
                (keep.size.w + 2 * blur.margin(), keep.size.h + 2 * blur.margin()).into(),
            )
            .intersection(whole)
            .expect("overlaps");
            let mut part = patch(grown, |x, y| pixel(x + grown.loc.x, y + grown.loc.y));
            blur.run(&mut part, keep, (0, 0));
            for y in keep.loc.y..keep.loc.y + keep.size.h {
                for x in keep.loc.x..keep.loc.x + keep.size.w {
                    let a = &full.bytes[((y * 160 + x) * 4) as usize..][..4];
                    let b = &part.bytes[(((y - grown.loc.y) * grown.size.w + x - grown.loc.x) * 4) as usize..][..4];
                    assert_eq!(a, b, "pixel ({x}, {y}) of the damage {keep:?}");
                }
            }
        }
    }
}
