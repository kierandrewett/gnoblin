// What a user shader pass adds after the shell's source: the body the shared entry point calls.
//
// `effect` is the shell's function. It gets the node's own premultiplied colour at this point
// of the layer (`c`), the point as a coordinate over the layer's rectangle with (0, 0) at the
// top-left (`uv`), the eight numbers it was given (`p`) and the pass's time in seconds (`t`),
// and returns a premultiplied colour.
//
// The wrapper clamps that colour to a valid premultiplied one (alpha 0 to 1, each channel no
// larger than alpha), so a shader cannot add light the blend has no name for, then applies the
// layer's filter grade, tint, opacity and rounded corners as the surface pass does. A shader therefore cannot
// draw outside the layer's silhouette.
fn shade(local: vec2<f32>) -> vec4<f32> {
    let node = textureSampleLevel(gnoblin_tex, gnoblin_sampler, gnoblin_uv(local), 0.0);
    let p = array<f32, 8>(
        gnoblin_user.p[0].x, gnoblin_user.p[0].y, gnoblin_user.p[0].z, gnoblin_user.p[0].w,
        gnoblin_user.p[1].x, gnoblin_user.p[1].y, gnoblin_user.p[1].z, gnoblin_user.p[1].w,
    );
    let shaded = effect(node, local, p, gnoblin_user.clock.x);
    let alpha = clamp(shaded.a, 0.0, 1.0);
    let colour = clamp(shaded.rgb, vec3<f32>(0.0), vec3<f32>(alpha));
    // The layer's filter grade comes after the shader, as it does after the layer's own pixels: the grade is of the
    // straight colour, so a graded pass unpremultiplies and premultiplies again, and an identity grade skips that.
    var graded = vec4<f32>(colour, alpha);
    if any(gnoblin.grade.xyz != vec3<f32>(1.0)) && alpha > 0.0 {
        graded = vec4<f32>(gnoblin_grade(colour / alpha) * alpha, alpha);
    }
    let premultiplied = graded * gnoblin.tint * gnoblin.opacity;
    let coverage = gnoblin_corner(local);
    return vec4<f32>(premultiplied.rgb * coverage, premultiplied.a * coverage);
}
