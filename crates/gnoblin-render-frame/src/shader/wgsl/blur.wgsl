// One pass of a separable Gaussian blur of what is already behind the layer.
//
// The renderer copies the pixels the layer will cover into a scratch target, runs this
// stage horizontally and then vertically, and the two passes together are a Gaussian of
// sigma = radius / 2 pixels. Nine taps at `params.xy` apart with a Gaussian of sigma 1.5
// taps is what makes those two numbers agree: `gnoblin_weight` is that kernel,
// normalised.
//
// The result is opaque, because it *is* the backdrop: it replaces the pixels of the
// quad rather than blending with them.
//
// Sampling clamps at the target's edge (`CLAMP_TO_EDGE`), so a blur that reaches the
// output's boundary repeats the boundary pixel instead of reading whatever the scratch
// target held outside it.
fn gnoblin_weight(index: f32) -> f32 {
    return exp(-0.5 * index * index / 2.25) / 3.7514;
}

fn shade(local: vec2<f32>) -> vec4<f32> {
    let uv = gnoblin_uv(local);
    let step = gnoblin.params.xy;
    var total = vec4<f32>(0.0);
    for (var index = -4; index <= 4; index = index + 1) {
        let weight = gnoblin_weight(f32(index));
        total = total + textureSample(gnoblin_tex, gnoblin_sampler, uv + step * f32(index)) * weight;
    }
    // The grade is identity on every pass but the last: params.z is the brightness gain,
    // params.w the contrast gain about mid grey, and the tint is mixed over the graded
    // colour by its own alpha. Brightness 1, contrast 1 and tint alpha 0 change nothing,
    // which is what every sweep but the last carries.
    let gain = gnoblin.params.z;
    let contrast = gnoblin.params.w;
    let lit = total.rgb * gain;
    let graded = clamp((lit - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(1.0));
    let tinted = mix(graded, gnoblin.tint.rgb, gnoblin.tint.a);
    return vec4<f32>(tinted, total.a);
}
