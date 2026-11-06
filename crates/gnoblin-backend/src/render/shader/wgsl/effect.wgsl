
// The entry point every effect body is compiled with.
//
// It passes the quad's local coordinate to `shade` and returns the colour it wrote; the
// pass itself decides what the coordinate means.
struct GnoblinEffectOut {
    @location(0) colour: vec4<f32>,
}

@fragment
fn gnoblin_entry(@location(0) local: vec2<f32>) -> GnoblinEffectOut {
    return GnoblinEffectOut(shade(local));
}
