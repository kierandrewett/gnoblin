// A layer's own pixels: cropped, scaled, graded, tinted and clipped to its corner radius.
//
// The crop and the scale come from `src`, which the renderer derives from the layer's
// source rectangle and the texture's size, so this stage knows nothing about a client
// buffer's stride or orientation.
//
// The sample is premultiplied. The grade is defined on straight colour, so a graded layer is
// unpremultiplied, graded and premultiplied again; an identity grade skips that and costs
// nothing visible. The tint then multiplies the premultiplied colour, which keeps it
// premultiplied: a channel can only shrink, and the alpha is scaled separately by the group
// opacity.
fn shade(local: vec2<f32>) -> vec4<f32> {
    var sample = textureSample(gnoblin_tex, gnoblin_sampler, gnoblin_uv(local));
    if any(gnoblin.grade.xyz != vec3<f32>(1.0)) && sample.a > 0.0 {
        let straight = gnoblin_grade(sample.rgb / sample.a);
        sample = vec4<f32>(straight * sample.a, sample.a);
    }
    let premultiplied = sample * gnoblin.tint * gnoblin.opacity;
    let coverage = gnoblin_corner(local);
    return vec4<f32>(premultiplied.rgb * coverage, premultiplied.a * coverage);
}
