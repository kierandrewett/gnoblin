// The whole-output downsample pass of a supersampled output.
//
// An output composed at `mode * s` is shown at `mode`. This pass reads the finished virtual surface and writes the
// real-size target, so the display shows each pixel as the average of what covers it. It is a separate pass with its
// own uniform block, like the view pass: the layer passes never see its fields.
//
// When the virtual size is a whole multiple of the target's on both axes (a factor of 2 or 4) every target pixel is
// the exact mean of its `across x down` block, read with `textureLoad` so the filter state of the texture cannot
// blend a neighbour in. When it is not (a factor of 1.5), the pass takes one bilinear sample at the target pixel's
// centre, which is the same rule `Image::downsample` follows on the CPU. `Image::box_ratio` in
// `gnoblin-render-frame` decides which of the two applies, and the renderer writes the result into `taps`.
//
// A half is rounded up as the CPU path rounds it: the mean is raised by a thousandth of a level, far below the gap
// between two means that are not a tie, so only an exact half moves, and it moves up. The draw replaces the target.
//
// Textures are bottom-up in GL, so a top-down pixel row `y` of an `h` row surface is texel row `h - 1 - y`.
//
// Layout (two `vec4`, 32 bytes in WGSL and std140 alike):
//   sizes: target width, target height, source width, source height
//   taps:  source pixels per target pixel across and down, both zero when the pass samples bilinearly; two unused
// `DownsampleUniforms` in `gnoblin-render-gles` packs exactly this layout.

struct GnoblinDownsample {
    sizes: vec4<f32>,
    taps: vec4<f32>,
}

struct GnoblinDownsampleOut {
    @location(0) colour: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gnoblin_downsample: GnoblinDownsample;
@group(1) @binding(0) var gnoblin_tex: texture_2d<f32>;
@group(1) @binding(1) var gnoblin_sampler: sampler;

@fragment
fn gnoblin_entry(@location(0) local: vec2<f32>) -> GnoblinDownsampleOut {
    // The quad covers the whole target, so the unit point scaled by the size is the pixel.
    let target_size = gnoblin_downsample.sizes.xy;
    let source_size = gnoblin_downsample.sizes.zw;
    let at = floor(local * target_size);
    if gnoblin_downsample.taps.x > 0.5 {
        let across = i32(gnoblin_downsample.taps.x);
        let down = i32(gnoblin_downsample.taps.y);
        let origin = vec2<i32>(at) * vec2<i32>(across, down);
        let last_row = i32(source_size.y) - 1;
        var sum = vec4<f32>(0.0);
        for (var row = 0; row < down; row = row + 1) {
            for (var column = 0; column < across; column = column + 1) {
                sum = sum + textureLoad(gnoblin_tex, vec2<i32>(origin.x + column, last_row - (origin.y + row)), 0);
            }
        }
        let mean = sum / f32(across * down) + vec4<f32>(0.001 / 255.0);
        return GnoblinDownsampleOut(min(mean, vec4<f32>(1.0)));
    }
    let uv = vec2<f32>((at.x + 0.5) / target_size.x, 1.0 - (at.y + 0.5) / target_size.y);
    // An explicit level needs no uniform control flow, which the branch above is not guaranteed to give the driver.
    return GnoblinDownsampleOut(textureSampleLevel(gnoblin_tex, gnoblin_sampler, uv, 0.0));
}
