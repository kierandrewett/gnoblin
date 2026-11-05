// A layer's own pixels: cropped, scaled, tinted and clipped to its corner radius.
//
// The crop and the scale come from `src`, which the renderer derives from the layer's
// source rectangle and the texture's size, so this stage knows nothing about a client
// buffer's stride or orientation.
//
// The tint multiplies a *premultiplied* colour, which keeps it premultiplied: a channel
// can only shrink, and the alpha is scaled separately by the group opacity.
fn shade(local: vec2<f32>) -> vec4<f32> {
    let sample = textureSample(gnoblin_tex, gnoblin_sampler, gnoblin_uv(local));
    let premultiplied = sample * gnoblin.tint * gnoblin.opacity;
    let coverage = gnoblin_corner(local);
    return vec4<f32>(premultiplied.rgb * coverage, premultiplied.a * coverage);
}
