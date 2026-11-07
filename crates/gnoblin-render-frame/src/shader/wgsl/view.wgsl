// The whole-output pass that applies the magnifier view and the colour matrix.
//
// It is a separate pass with its own small uniform block, not the 96-byte block of the
// preamble: nothing a user shader declares reaches it, and the layer passes never see its
// fields. The renderer copies the finished output into a scratch target and draws this
// stage over the whole output, sampling the copy, so every pixel is read from what was
// drawn before the view and the matrix were applied. The draw replaces the output rather
// than blending over it.
//
// The view is nearest-neighbour: output pixel `p` shows the source pixel at
// `centre + (p + 0.5 - centre) / zoom`, clamped to the output. A zoom of one with any centre
// maps every pixel to itself, which is why the renderer skips this pass for an identity view.
//
// The colour matrix acts on straight (unpremultiplied) colour and is then premultiplied
// again. A fully transparent pixel is passed through unchanged, since it has no colour to
// change, as the software path does. The matrix only runs when `flags.x` is one.
//
// Textures are bottom-up in GL, so a top-down pixel row `y` is read at v = 1 - (y + 0.5) / h,
// the same flip `flipped_mapping` states for the blur.
//
// Layout (all `vec4`, so the block is 128 bytes in WGSL and std140 alike):
//   centre:  centre x, centre y, zoom, unused
//   size:    output width, height in physical pixels, unused, unused
//   row0..3: the matrix's four weights per output channel
//   offsets: the matrix's offset per output channel
//   flags:   x is one when the matrix applies, otherwise zero
// `ViewUniforms` in `gnoblin-render-gles` packs exactly this layout.

struct GnoblinView {
    centre: vec4<f32>,
    size: vec4<f32>,
    row0: vec4<f32>,
    row1: vec4<f32>,
    row2: vec4<f32>,
    row3: vec4<f32>,
    offsets: vec4<f32>,
    flags: vec4<f32>,
}

struct GnoblinViewOut {
    @location(0) colour: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gnoblin_view: GnoblinView;
@group(1) @binding(0) var gnoblin_tex: texture_2d<f32>;
@group(1) @binding(1) var gnoblin_sampler: sampler;

// The source index shown at one axis of an output position, clamped to the output.
fn gnoblin_view_index(position: f32, centre: f32, zoom: f32, length: f32) -> f32 {
    let source = centre + (position + 0.5 - centre) / zoom;
    return clamp(floor(source), 0.0, length - 1.0);
}

// One premultiplied pixel with the colour matrix applied to its straight colour.
fn gnoblin_tint(pixel: vec4<f32>) -> vec4<f32> {
    if pixel.a <= 0.0 {
        return pixel;
    }
    let straight = vec4<f32>(min(pixel.rgb / pixel.a, vec3<f32>(1.0)), pixel.a);
    let out = clamp(
        vec4<f32>(
            dot(gnoblin_view.row0, straight),
            dot(gnoblin_view.row1, straight),
            dot(gnoblin_view.row2, straight),
            dot(gnoblin_view.row3, straight),
        ) + gnoblin_view.offsets,
        vec4<f32>(0.0),
        vec4<f32>(1.0),
    );
    return vec4<f32>(out.rgb * out.a, out.a);
}

@fragment
fn gnoblin_entry(@location(0) local: vec2<f32>) -> GnoblinViewOut {
    // The quad covers the whole output, so the unit point scaled by the size is the pixel.
    let size = gnoblin_view.size.xy;
    let at = floor(local * size);
    let centre = gnoblin_view.centre;
    let source = vec2<f32>(
        gnoblin_view_index(at.x, centre.x, centre.z, size.x),
        gnoblin_view_index(at.y, centre.y, centre.z, size.y),
    );
    // Bottom-up texture: flip v so a top-down source row is read at its own texel.
    let uv = vec2<f32>((source.x + 0.5) / size.x, 1.0 - (source.y + 0.5) / size.y);
    var pixel = textureSample(gnoblin_tex, gnoblin_sampler, uv);
    if gnoblin_view.flags.x > 0.5 {
        pixel = gnoblin_tint(pixel);
    }
    return GnoblinViewOut(pixel);
}
