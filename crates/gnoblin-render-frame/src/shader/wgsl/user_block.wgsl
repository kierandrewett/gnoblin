// What a user shader pass adds to the shared preamble, in front of the shell's source.
//
// The wrapper owns every binding. The user's source supplies `fn effect` and constants and
// helper functions, and nothing else may be declared: no variable, no texture, no sampler, no
// storage, no entry point. `shader::user::validate` refuses a source that does.
//
// `gnoblin_user` is a second uniform block, beside the shared `gnoblin` one. The eight
// numbers a shell writes are `p[0..8]` in the shader. A uniform array needs a 16-byte
// stride, so they are held as two `vec4`s and `shade` reads them back as an `array<f32, 8>`.
// `clock.x` is the pass's time in seconds; the other three lanes are reserved and zero.
// `UserBlock::pack` in `uniforms.rs` is the CPU side of this layout: 48 bytes.
struct GnoblinUser {
    p: array<vec4<f32>, 2>,
    clock: vec4<f32>,
}

@group(0) @binding(1) var<uniform> gnoblin_user: GnoblinUser;

// The backdrop: the pixels behind the layer, as `OutputFrame::backdrop_input` describes them
// (secure windows and the lock card are black), in a texture the size of the layer's own
// rectangle. Group 2 is declared in every user module; a renderer binds it only when the pass
// reads the backdrop (`ShaderPass::backdrop`). When the source never calls `backdrop`, the
// entry point does not use the group.
@group(2) @binding(0) var gnoblin_backdrop_tex: texture_2d<f32>;
@group(2) @binding(1) var gnoblin_backdrop_sampler: sampler;

// The premultiplied backdrop colour at `uv`, where (0, 0) is the layer's top-left corner and
// (1, 1) its bottom-right. A coordinate outside 0..1 reads the nearest edge pixel. The level is
// fixed at 0, so it can be called from any control flow.
fn backdrop(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(gnoblin_backdrop_tex, gnoblin_backdrop_sampler, uv, 0.0);
}
