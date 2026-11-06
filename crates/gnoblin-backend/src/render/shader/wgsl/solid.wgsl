// A rectangle of one colour, with the layer's corner radius and opacity.
//
// It fills the background, a solid layer and the built-in pointer, and it is the pass
// a rounded corner of a solid window goes through. The output is premultiplied because
// every pass blends with `ONE, ONE_MINUS_SRC_ALPHA`: a straight colour blended that way
// would be too bright.
fn shade(local: vec2<f32>) -> vec4<f32> {
    // The tint scales the straight colour's every channel, alpha included, which is the same
    // multiplication the software path applies to a solid before premultiplying it.
    let colour = gnoblin.colour * gnoblin.tint;
    let coverage = gnoblin_corner(local);
    let alpha = colour.a * gnoblin.opacity * coverage;
    return vec4<f32>(colour.rgb * alpha, alpha);
}
