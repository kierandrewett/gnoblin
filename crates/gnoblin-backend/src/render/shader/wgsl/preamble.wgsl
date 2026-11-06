// The preamble every fragment stage is compiled with.
//
// One uniform block serves every pass, generated once here and shared by the effect
// shaders and the user shader, so a pass uploads one small buffer and a user shader
// gets its declared uniforms appended to the same struct at the offset the layout
// rules give them.
//
// Layout rule: every member is a `vec4` except the three scalars at the end, so the
// struct is 96 bytes, a multiple of 16, which is the alignment WGSL's uniform layout
// and GLSL's `std140` both give it. A user field therefore starts at offset 96 in
// both languages, and `UniformLayout` in `program.rs` is the CPU side of that
// agreement.
//
// The `textureSample` pair is declared here rather than in each body so a user shader
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
    // tap step.
    params: vec4<f32>,
    // Corner radius in physical pixels.
    corner: f32,
    // Group opacity, 0 to 1.
    opacity: f32,
    // Keeps the block a multiple of 16 bytes so a user field's offset is the same in
    // both languages.
    padding: vec2<f32>,
    // gnoblin:user-fields
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
// cannot invert the box.
fn gnoblin_corner(local: vec2<f32>) -> f32 {
    return gnoblin_rounded_box(local * gnoblin.rect.zw, gnoblin.rect.zw, gnoblin.corner);
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
