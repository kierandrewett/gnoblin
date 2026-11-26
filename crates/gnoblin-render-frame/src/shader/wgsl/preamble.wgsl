// The preamble every fragment stage is compiled with.
//
// One uniform block serves every pass, so a pass uploads one small buffer.
//
// Layout rule: every member is a `vec4` except the trailing `corner` and `opacity`
// scalars and the `vec2` padding. The struct is therefore 112 bytes, a multiple of 16,
// which is the alignment that WGSL's uniform layout and GLSL's `std140` both give it.
// `Uniforms::pack` in `uniforms.rs` is the CPU side of that agreement.
//
// The `textureSample` pair is declared here rather than in each body so a pass
// can sample the layer it is shading without declaring anything.

struct Gnoblin {
    // The destination rectangle in physical pixels: x, y, width, height.
    rect: vec4<f32>,
    // The texture coordinate mapping: origin x, origin y, scale x, scale y. A pass
    // that samples a texture maps its local 0..1 onto it, which is also how a
    // vertically flipped source is stated: a negative scale and a shifted origin.
    src: vec4<f32>,
    // The pass's straight colour (a solid's fill, a shadow's colour).
    colour: vec4<f32>,
    // The layer's multiplicative colour.
    tint: vec4<f32>,
    // Values whose meaning belongs to the pass: a shadow's inset and blur, a blur's
    // tap step, a border's growth and its band. A solid or a surface that is rounded to a
    // silhouette larger than its own quad (a window's content under its titlebar's room)
    // holds that silhouette here: its origin relative to the quad's, then its size, so
    // `params.z > 0` says there is one.
    params: vec4<f32>,
    // The colour grade: saturate, brightness, contrast, then the grain amount. The first
    // three are 1 and the last 0 when a pass is not graded. `gnoblin_grade` and
    // `gnoblin_grain` read it; `grade.rs` is the CPU twin of both.
    grade: vec4<f32>,
    // Corner radius in physical pixels.
    corner: f32,
    // Group opacity, 0 to 1.
    opacity: f32,
    // Keeps the block a multiple of 16 bytes.
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> gnoblin: Gnoblin;
@group(1) @binding(0) var gnoblin_tex: texture_2d<f32>;
@group(1) @binding(1) var gnoblin_sampler: sampler;

// The texture coordinate of a point of the quad.
fn gnoblin_uv(local: vec2<f32>) -> vec2<f32> {
    return gnoblin.src.xy + local * gnoblin.src.zw;
}

// The coverage of a rounded rectangle at a point of the quad, 0 outside and 1 inside.
//
// It is a signed distance to the rounded box, converted to coverage over one pixel so
// the corner is antialiased. A radius of zero gives a hard edge at the rectangle's own
// boundary, and the radius is clamped so a radius larger than half the shorter side
// cannot invert the box. The box is the quad's own rectangle, or the silhouette in
// `params` when the layer is one part of a larger rounded frame.
fn gnoblin_corner(local: vec2<f32>) -> f32 {
    let point = local * gnoblin.rect.zw;
    if gnoblin.params.z > 0.0 {
        return gnoblin_rounded_box(point - gnoblin.params.xy, gnoblin.params.zw, gnoblin.corner);
    }
    return gnoblin_rounded_box(point, gnoblin.rect.zw, gnoblin.corner);
}

// The coverage of a rounded box centred on `size / 2` at `point`, in pixels.
//
// `gnoblin_sdf` is the signed distance to the box's boundary: negative inside, zero on it,
// positive outside. Turning it into coverage over one pixel is what antialiases a corner
// without a multisampled target.
fn gnoblin_rounded_box(point: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let distance = gnoblin_sdf(point, size, radius);
    return 1.0 - smoothstep(-0.5, 0.5, distance);
}

// The signed distance from a point to the boundary of a rounded box, in pixels.
//
// It is the distance to the box shrunk by the radius, minus the radius: a box with a zero
// radius is the plain rectangle, so a square corner needs no separate path.
fn gnoblin_sdf(point: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let half = size * 0.5;
    let corner = clamp(radius, 0.0, min(half.x, half.y));
    let offset = abs(point - half) - (half - vec2<f32>(corner));
    return length(max(offset, vec2<f32>(0.0))) + min(max(offset.x, offset.y), 0.0) - corner;
}

// The grade of a straight sRGB colour: saturate, then brightness, then contrast, each step
// clamped to 0..1. Saturate is CSS's `feColorMatrix type="saturate"`, which is a mix from the
// colour's luma (weights 0.213, 0.715, 0.072) towards the colour. No transfer function is
// applied, as for every other pass. `grade::apply` in `gnoblin-render-frame` is the same
// arithmetic on the CPU.
fn gnoblin_grade(colour: vec3<f32>) -> vec3<f32> {
    let luma = dot(colour, vec3<f32>(0.213, 0.715, 0.072));
    let saturated = clamp(vec3<f32>(luma) + (colour - vec3<f32>(luma)) * gnoblin.grade.x, vec3<f32>(0.0), vec3<f32>(1.0));
    let lit = clamp(saturated * gnoblin.grade.y, vec3<f32>(0.0), vec3<f32>(1.0));
    return clamp((lit - vec3<f32>(0.5)) * gnoblin.grade.z + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(1.0));
}

// One round of the PCG hash on an integer.
fn gnoblin_pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// The grain to add to each colour channel at a point of the quad: `(h - 0.5) * noise * 0.0625`,
// where `h` in 0..1 is the PCG hash of the pixel's position from the quad's top-left. The
// position is the pixel's, not the screen's, so grain moves with the layer and every renderer
// and every frame draws the same grain. `grade::grain` is the CPU twin.
fn gnoblin_grain(local: vec2<f32>) -> f32 {
    let pixel = vec2<u32>(floor(local * gnoblin.rect.zw));
    let hash = gnoblin_pcg(pixel.x ^ gnoblin_pcg(pixel.y));
    return (f32(hash) / 4294967295.0 - 0.5) * gnoblin.grade.w * 0.0625;
}
