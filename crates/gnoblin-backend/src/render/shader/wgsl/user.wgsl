
// The entry point a user shader is compiled with.
//
// `shade` receives the layer's texture coordinate, already mapped from the quad and
// through the source rectangle, so a user shader samples the pixels it is shading with
// `textureSample(gnoblin_tex, gnoblin_sampler, uv)` and never has to know about crops,
// scale or orientation.
struct GnoblinUserOut {
    @location(0) colour: vec4<f32>,
}

@fragment
fn gnoblin_entry(@location(0) local: vec2<f32>) -> GnoblinUserOut {
    return GnoblinUserOut(shade(gnoblin_uv(local)));
}
