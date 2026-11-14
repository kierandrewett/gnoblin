// A border: a ring along the edge of a rounded rectangle, drawn over a layer.
//
// The quad this runs on is the rounded rectangle (the layer's silhouette) grown by
// `params.xy` on every side, so `params.xy` is that growth and the rectangle itself is the
// quad inset by it. The ring is the band of points whose signed distance to the rectangle
// lies between `params.z` and `params.w`, in pixels: an outside border has a positive band,
// an inside one a negative band. The signed distance of a rounded rectangle grown by `t` is
// its own distance minus `t`, so the band needs no second shape and no branch: it is the
// product of two one-pixel antialiased edges.
//
// The output is premultiplied, like every pass.
fn shade(local: vec2<f32>) -> vec4<f32> {
    let grow = gnoblin.params.xy;
    let inner = max(gnoblin.rect.zw - grow * 2.0, vec2<f32>(1.0));
    let point = local * gnoblin.rect.zw - grow;
    let distance = gnoblin_sdf(point, inner, gnoblin.corner);
    // Inside the band's far edge and outside its near edge. Each edge is antialiased over one
    // pixel, so a stroke thinner than a pixel is fainter rather than aliased.
    let within = clamp(0.5 - (distance - gnoblin.params.w), 0.0, 1.0);
    let beyond = clamp(0.5 + (distance - gnoblin.params.z), 0.0, 1.0);
    let colour = gnoblin.colour * gnoblin.tint;
    let alpha = colour.a * gnoblin.opacity * within * beyond;
    return vec4<f32>(colour.rgb * alpha, alpha);
}
