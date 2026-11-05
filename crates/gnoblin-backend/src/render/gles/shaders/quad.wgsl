// The vertex stage every pass shares.
//
// It is written in WGSL because the shader language of D12 is WGSL: the fragment
// stages below are the effects and the user shaders, and this stage is the one quad
// they all draw, so writing it in another language would mean two compilers for one
// pass.
//
// The geometry arrives as attributes rather than uniforms: a draw's rectangle and
// the target's size change per quad, so per-vertex data keeps the fragment stage the
// only stage with a uniform block. One block per program is what makes binding it a
// single `uniform_block_binding(program, 0, 0)` and keeps a program's two stages from
// declaring the same block twice under different names.
//
// `position` is the unit quad corner, `quad` is the destination in physical pixels and
// `viewport` is the target's size.
//
// # The clip-space convention this stage is written in
//
// Pixel y grows downwards (the compositor's convention) and the clip space written here is
// y-down too: `pixel.y = 0` is `clip.y = -1`. That is the convention Vulkan uses, and D12's
// Vulkan backend will take this module to SPIR-V unchanged. `naga`'s GLSL backend compiles
// the same module for GLES by writing `gl_Position.yz = vec2(-gl_Position.y, ...)` after the
// shader (`WriterFlags::ADJUST_COORDINATE_SPACE`, on by default), which turns it into GL's
// y-up clip space and GL's -1..1 depth range. Writing the GL convention here instead would
// put the fixup the wrong way round on the day the Vulkan backend lands.

struct GnoblinVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
}

@vertex
fn gnoblin_entry(
    @location(0) position: vec2<f32>,
    @location(1) quad: vec4<f32>,
    @location(2) viewport: vec2<f32>,
) -> GnoblinVertex {
    let pixel = quad.xy + position * quad.zw;
    let clip = vec2<f32>(pixel.x / viewport.x * 2.0 - 1.0, pixel.y / viewport.y * 2.0 - 1.0);
    return GnoblinVertex(vec4<f32>(clip, 0.0, 1.0), position);
}
