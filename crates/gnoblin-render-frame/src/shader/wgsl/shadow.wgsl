// A drop shadow: the layer's rounded rectangle, offset, with an edge that falls off
// over the blur radius.
//
// The quad this runs on is the layer's rectangle grown by the blur on every side and
// moved by the shadow's offset, so `params.xy` is that growth and the layer's own
// rectangle is the quad inset by it. A pixel inside that rectangle is fully covered; a
// pixel outside it fades over `params.z` pixels, which is what makes a shadow darker
// next to the window than further away.
//
// The output is premultiplied, like every pass.
fn shade(local: vec2<f32>) -> vec4<f32> {
    let inset = gnoblin.params.xy;
    let blur = max(gnoblin.params.z, 1.0);
    let inner = max(gnoblin.rect.zw - inset * 2.0, vec2<f32>(1.0));
    let point = local * gnoblin.rect.zw - inset;
    // The shadow is fully opaque inside the layer's own rounded rectangle and fades over the
    // blur distance outside it, so it is darkest next to the window and dimmer further away.
    let distance = gnoblin_sdf(point, inner, gnoblin.corner);
    let coverage = 1.0 - smoothstep(0.0, blur, distance);
    let alpha = gnoblin.colour.a * gnoblin.opacity * coverage;
    return vec4<f32>(gnoblin.colour.rgb * alpha, alpha);
}
