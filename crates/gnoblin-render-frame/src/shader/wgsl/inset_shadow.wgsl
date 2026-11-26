// An inset shadow: the layer's rounded shape, shaded from its edge inwards.
//
// The quad this runs on is the shape itself (the layer's rectangle, or its silhouette), so
// `rect.zw` is the shape's size. The hole is the shape moved by `params.xy` (the offset) and
// shrunk by `params.w` (the spread) on every side, with radius `max(corner - spread, 0)`. A
// pixel on or outside the hole's edge has the full shadow and it falls to nothing `params.z`
// (the blur) pixels inside the edge: the outer shadow's falloff reflected. The shadow is
// multiplied by the shape's antialiased coverage, so nothing leaks past a rounded corner.
//
// The output is premultiplied, like every pass. All lengths are physical pixels.
fn shade(local: vec2<f32>) -> vec4<f32> {
    let offset = gnoblin.params.xy;
    let blur = max(gnoblin.params.z, 1.0);
    let spread = gnoblin.params.w;
    let point = local * gnoblin.rect.zw;
    let hole = max(gnoblin.rect.zw - vec2<f32>(spread * 2.0), vec2<f32>(1.0));
    let distance = gnoblin_sdf(point - offset - vec2<f32>(spread), hole, max(gnoblin.corner - spread, 0.0));
    let strength = smoothstep(-blur, 0.0, distance);
    let coverage = gnoblin_rounded_box(point, gnoblin.rect.zw, gnoblin.corner);
    let alpha = gnoblin.colour.a * gnoblin.opacity * strength * coverage;
    return vec4<f32>(gnoblin.colour.rgb * alpha, alpha);
}
