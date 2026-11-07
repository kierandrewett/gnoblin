//! The GLES renderer's tests: one per failure mode in the module docs, plus the parity
//! probes the Renderer card asks for.
//!
//! Every test builds its own renderer, so a test that leaves GL state behind cannot help a
//! later one pass. The renderer needs a GLES context, and this machine has one (Mesa on a
//! surfaceless EGL display, llvmpipe in CI and a real GPU on a developer's machine); a
//! machine without EGL prints the reason once per test and skips, which is the only case a
//! skip is allowed: the code is still built, and [`super::Gles::new`] reports
//! [`crate::render::RenderError::Unavailable`] rather than failing the run.

use super::*;
use crate::render::Software;
use crate::render::frame::{Model, Solid};
use gnoblin_render::{BlurParams, Effects, Mat4, Shadow, UniformType, UniformValue, Vec3};

/// A blur with the given radius and one pass, which is one horizontal and one vertical sweep.
/// The reference formulas in these tests describe that single sweep, so the schema's default
/// of three passes is pinned off here and tested where a pass count is the point.
fn blur(radius: f32) -> BlurParams {
    BlurParams {
        radius,
        passes: 1,
        ..BlurParams::default()
    }
}

/// A renderer, or a printed reason when this machine cannot make a GLES context.
fn gles(physical: Size) -> Option<Gles> {
    match Gles::new(physical) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            println!("skipping the GLES test: {error} (no surfaceless EGL context on this machine)");
            None
        }
    }
}

/// A frame that paints `rect` in `colour` over `background`, damaging the whole output.
fn frame_with(background: Color, size: Size, rect: Rect, colour: Color) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(background);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    frame.push_layer(Layer::Solid(Solid::new(rect, colour)));
    frame
}

/// The colour of a pixel, which is what a probe is.
fn probe(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image.colour_at(x, y).unwrap_or_else(|| {
        panic!(
            "a pixel at {x},{y} of a {}x{} image",
            image.size().width,
            image.size().height
        )
    })
}

/// The perceived brightness of a pixel, for a "darker than" probe.
fn luma(pixel: [u8; 4]) -> u32 {
    u32::from(pixel[0]) * 3 + u32::from(pixel[1]) * 6 + u32::from(pixel[2])
}

#[test]
fn a_readback_is_top_left_first() {
    let Some(mut renderer) = gles(Size::new(64, 48)) else {
        return;
    };
    let mut frame = frame_with(
        Color::BLACK,
        Size::new(64, 48),
        Rect::new(0, 0, 8, 8),
        Color::from_rgba8(255, 0, 0, 255),
    );
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 40, 8, 8),
        Color::from_rgba8(0, 0, 255, 255),
    )));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(image.size(), Size::new(64, 48));
    assert_eq!(
        probe(&image, 2, 2),
        [255, 0, 0, 255],
        "the top-left rectangle is at the top: a bottom-up readback would put it at the bottom"
    );
    assert_eq!(
        probe(&image, 2, 44),
        [0, 0, 255, 255],
        "and the rectangle drawn near the bottom stays there"
    );
    assert_eq!(probe(&image, 40, 20), [0, 0, 0, 255], "the background elsewhere");
}

#[test]
fn an_idle_frame_draws_nothing() {
    let Some(mut renderer) = gles(Size::new(32, 32)) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::from_rgba8(1, 2, 3, 255));
    renderer.draw(&frame, Scale::IDENTITY).expect("nothing drawn");
    assert_eq!(renderer.draws(), 0, "a frame with no damage costs no pass");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 0, 0),
        [0, 0, 0, 0],
        "an undamaged frame paints no pixel, not even the background, which is the state a \
         fresh target is in"
    );

    let frame = frame_with(Color::BLACK, Size::new(32, 32), Rect::new(0, 0, 4, 4), Color::WHITE);
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    assert_eq!(renderer.draws(), 1, "a damaged frame counts one draw");
}

#[test]
fn damage_limits_what_is_repainted() {
    let Some(mut renderer) = gles(Size::new(16, 16)) else {
        return;
    };
    let whole = Rect::new(0, 0, 16, 16);
    renderer
        .draw(
            &frame_with(
                Color::from_rgba8(255, 0, 0, 255),
                Size::new(16, 16),
                whole,
                Color::from_rgba8(0, 0, 255, 255),
            ),
            Scale::IDENTITY,
        )
        .expect("the first frame");
    // A second frame repaints only the left half: the right half keeps the first frame's
    // pixels, which is what "damage only" means.
    let mut frame = OutputFrame::new();
    frame.set_background(Color::from_rgba8(0, 255, 0, 255));
    frame.push_damage(Rect::new(0, 0, 8, 16));
    renderer.draw(&frame, Scale::IDENTITY).expect("the second frame");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(probe(&image, 2, 8), [0, 255, 0, 255], "the damaged half was repainted");
    assert_eq!(
        probe(&image, 12, 8),
        [0, 0, 255, 255],
        "the undamaged half kept the first frame's blue layer"
    );
}

#[test]
fn a_translucent_layer_blends_with_the_background() {
    let size = Size::new(8, 8);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::from_rgba8(0, 0, 0, 255));
    frame.push_damage(Rect::new(0, 0, 8, 8));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 8, 8),
        Color::from_rgba8(255, 255, 255, 128),
    )));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    let gl = probe(&image, 4, 4);
    let [red, green, blue, alpha] = gl;
    assert!(
        red.abs_diff(128) <= 2 && green.abs_diff(128) <= 2 && blue.abs_diff(128) <= 2,
        "half of white over black is mid grey, within a rounding step: {red} {green} {blue}"
    );
    assert_eq!(alpha, 255, "the opaque background keeps the frame opaque");

    // The software renderer draws the same frame, and the two must agree: opacity is one of
    // the properties both implement, so a difference here is a bug in one of them.
    let mut software = Software::new(size).expect("a software renderer");
    software.draw(&frame, Scale::IDENTITY).expect("drawn");
    let software_probe = probe(&software.screenshot().expect("a readback"), 4, 4);
    for (gl, software) in gl.iter().zip(software_probe.iter()) {
        assert!(
            gl.abs_diff(*software) <= 2,
            "GLES {gl:?} and software {software_probe:?} disagree about a translucent layer"
        );
    }
}

#[test]
fn a_tint_multiplies_the_layer() {
    let size = Size::new(8, 8);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 8, 8));
    let mut solid = Solid::new(Rect::new(0, 0, 8, 8), Color::from_rgba8(255, 255, 255, 255));
    // Half of red and green, all of blue: a tint that scales every channel differently is
    // the probe that catches a tint applied to the wrong component or not at all.
    solid.tint = Color::new(0.5, 0.5, 1.0, 1.0);
    frame.push_layer(Layer::Solid(solid));

    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let gl = probe(&renderer.screenshot().expect("a readback"), 4, 4);
    assert_eq!(gl, [128, 128, 255, 255], "each channel is scaled by its tint component");

    let mut software = Software::new(size).expect("a software renderer");
    software.draw(&frame, Scale::IDENTITY).expect("drawn");
    let software_probe = probe(&software.screenshot().expect("a readback"), 4, 4);
    for (gl, software) in gl.iter().zip(software_probe.iter()) {
        assert!(
            gl.abs_diff(*software) <= 2,
            "GLES {gl:?} and software {software_probe:?} disagree about a tint"
        );
    }
}

#[test]
fn a_corner_radius_reveals_the_background() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::from_rgba8(0, 0, 255, 255));
    frame.push_damage(Rect::new(0, 0, 64, 64));
    let mut solid = Solid::new(Rect::new(8, 8, 48, 48), Color::from_rgba8(255, 0, 0, 255));
    solid.corner_radius = 12.0;
    frame.push_layer(Layer::Solid(solid));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 9, 9),
        [0, 0, 255, 255],
        "the corner is the background, which is what a rounded window reveals"
    );
    assert_eq!(probe(&image, 32, 32), [255, 0, 0, 255], "the centre is the layer");
    assert_eq!(
        probe(&image, 9, 32),
        [255, 0, 0, 255],
        "the middle of an edge is still the layer: only the corners are cut"
    );
    assert_eq!(
        probe(&image, 4, 32),
        [0, 0, 255, 255],
        "outside the layer is the background"
    );
}

#[test]
fn a_shadow_darkens_next_to_the_edge_and_fades() {
    let size = Size::new(96, 96);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::WHITE);
    frame.push_damage(Rect::new(0, 0, 96, 96));
    let mut solid = Solid::new(Rect::new(24, 24, 48, 24), Color::from_rgba8(255, 0, 0, 255));
    let mut effects = Effects::new();
    effects
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 4.0, 0.0),
            blur: 8.0,
            spread: 0.0,
            opacity: 0.5,
            color: Color::BLACK,
        }))
        .expect("a shadow");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");

    // The window's bottom edge is at y = 48 and the shadow is offset four pixels down, so the
    // shadow's own edge is at y = 52 and its falloff runs to about y = 60.
    let near = luma(probe(&image, 48, 54));
    let far = luma(probe(&image, 48, 59));
    let away = luma(probe(&image, 48, 80));
    assert!(
        near < away && far < away,
        "the shadow is darker than the background: near {near}, far {far}, away {away}"
    );
    assert!(near < far, "and it decays with distance: near {near}, far {far}");
    assert_eq!(
        luma(probe(&image, 48, 40)),
        luma([255, 0, 0, 255]),
        "the window itself is untouched by its own shadow"
    );
}

#[test]
fn a_shadow_does_not_leave_the_damage() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // Fill the output first, so a pixel outside the next frame's damage has a colour to keep.
    renderer
        .draw(
            &frame_with(
                Color::from_rgba8(0, 255, 0, 255),
                size,
                Rect::new(0, 0, 64, 64),
                Color::from_rgba8(0, 255, 0, 255),
            ),
            Scale::IDENTITY,
        )
        .expect("the first frame");

    // A shadow that reaches well beyond the damage rectangle: only the damaged half may
    // change.
    let mut frame = OutputFrame::new();
    frame.set_background(Color::WHITE);
    frame.push_damage(Rect::new(0, 0, 32, 64));
    let mut solid = Solid::new(Rect::new(8, 8, 8, 8), Color::from_rgba8(255, 0, 0, 255));
    let mut effects = Effects::new();
    effects
        .push(Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 12.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
        }))
        .expect("a shadow");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer.draw(&frame, Scale::IDENTITY).expect("the second frame");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 40, 12),
        [0, 255, 0, 255],
        "a pixel the frame did not damage kept its colour, so the shadow could not reach it"
    );
    assert_eq!(
        probe(&image, 30, 12),
        [255, 255, 255, 255],
        "the damaged half was repainted, background and all"
    );
}

/// The sharp black and white edge of the blur tests, with a layer of `blur` over it, read back
/// from a fresh renderer. `None` where this machine has no GLES context.
fn blurred_edge(blur: BlurParams) -> Option<Image> {
    let size = Size::new(64, 64);
    let mut renderer = gles(size)?;
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 64, 64));
    // A sharp black and white edge, then a layer whose only content is the blur of what is
    // behind it.
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 32, 64),
        Color::from_rgba8(0, 0, 0, 255),
    )));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(32, 0, 32, 64),
        Color::from_rgba8(255, 255, 255, 255),
    )));
    let mut glass = Solid::new(Rect::new(0, 0, 64, 64), Color::TRANSPARENT);
    let mut effects = Effects::new();
    effects.push(Pass::Blur(blur)).expect("a blur");
    glass.effects = effects;
    frame.push_layer(Layer::Solid(glass));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    Some(renderer.screenshot().expect("a readback"))
}

#[test]
fn a_blur_blurs_a_sharp_edge_behind_the_layer() {
    let Some(image) = blurred_edge(blur(12.0)) else {
        return;
    };
    let edge = u32::from(probe(&image, 32, 32)[0]);
    let close = u32::from(probe(&image, 28, 32)[0]);
    let far = u32::from(probe(&image, 12, 32)[0]);
    let beyond = u32::from(probe(&image, 52, 32)[0]);
    assert!(
        (60..=195).contains(&edge),
        "the sharp edge is a gradient at its middle: {edge}"
    );
    assert!(
        close > 4 && close < edge,
        "four pixels to the black side is a ramp, not a step: {close} next to {edge}"
    );
    assert!(far <= 4, "the far side of the blur is still black: {far}");
    assert!(beyond >= 251, "and the white side is still white: {beyond}");
}

/// Each pass is a further sweep over the last, so more passes spread the same edge wider: a
/// point eight pixels into the black side is lighter after four passes than after one.
#[test]
fn more_blur_passes_spread_the_edge_further() {
    let (Some(one), Some(four)) = (
        blurred_edge(BlurParams {
            radius: 12.0,
            passes: 1,
            ..BlurParams::default()
        }),
        blurred_edge(BlurParams {
            radius: 12.0,
            passes: 4,
            ..BlurParams::default()
        }),
    ) else {
        return;
    };
    let near_one = u32::from(probe(&one, 24, 32)[0]);
    let near_four = u32::from(probe(&four, 24, 32)[0]);
    assert!(
        near_four > near_one + 4,
        "four passes reach further from the edge than one: {near_four} against {near_one}"
    );
}

/// The tint is mixed over the blurred backdrop after the last sweep. A half red tint over the
/// white side keeps the red and takes half of the green and blue away.
#[test]
fn a_blur_tint_grades_the_blurred_backdrop() {
    let Some(image) = blurred_edge(BlurParams {
        radius: 12.0,
        tint: Color::from_rgba8(255, 0, 0, 128),
        ..BlurParams::default()
    }) else {
        return;
    };
    let [red, green, blue, _] = probe(&image, 52, 32);
    assert!(red >= 240, "the red tint keeps the red of the white side: {red}");
    assert!((110..=145).contains(&green), "half of the green is mixed away: {green}");
    assert!(
        green.abs_diff(blue) <= 4,
        "green and blue take the same share: {green} against {blue}"
    );
}

#[test]
fn a_blur_at_the_edge_repeats_the_edge() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    // A red background under a white rectangle: the blur samples past the output's left edge,
    // where the only defined colour is the frame's own.
    frame.set_background(Color::from_rgba8(255, 0, 0, 255));
    frame.push_damage(Rect::new(0, 0, 64, 64));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 32, 64),
        Color::from_rgba8(255, 255, 255, 255),
    )));
    let mut glass = Solid::new(Rect::new(0, 0, 16, 64), Color::TRANSPARENT);
    let mut effects = Effects::new();
    effects.push(Pass::Blur(blur(10.0))).expect("a blur");
    glass.effects = effects;
    frame.push_layer(Layer::Solid(glass));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");

    let [red, green, blue, alpha] = probe(&image, 0, 32);
    assert_eq!(alpha, 255);
    assert!(
        green >= 90 && blue >= 90,
        "the edge pixel is a blend of the boundary's white and the background's red, not the \
         scratch target's clearing colour: {red} {green} {blue}"
    );
    assert!(
        red == 255,
        "and the background's red channel is what the edge repeats: {red} {green} {blue}"
    );
}

#[test]
fn a_user_shader_shades_its_layer() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    renderer.set_user_shader(Some(UserShader {
        source: "fn shade(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(0.0, 1.0, 0.0, 1.0);\n}\n".to_owned(),
        schema: UniformSchema::default(),
    }));
    if let Some(error) = renderer.take_shader_error() {
        panic!("the shader compiles: {error}");
    }

    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 32, 32));
    let mut solid = Solid::new(Rect::new(0, 0, 32, 32), Color::WHITE);
    let mut effects = Effects::new();
    effects
        .push(Pass::Shader(gnoblin_render::Shader::new(UniformSchema::default())))
        .expect("a shader pass");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 16, 16),
        [0, 255, 0, 255],
        "the layer is the colour the shader wrote, not the solid's own"
    );
}

#[test]
fn a_user_shader_reads_its_uniforms() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // Two fields of different alignment, so a wrong offset shows: `u_amount` is four bytes at
    // 96 and `u_colour` is sixteen at 112.
    let schema =
        UniformSchema::new(vec![("u_amount", UniformType::Float), ("u_colour", UniformType::Vec4)]).expect("a schema");
    renderer.set_user_shader(Some(UserShader {
        source: concat!(
            "fn shade(uv: vec2<f32>) -> vec4<f32> {\n",
            "    return vec4<f32>(gnoblin.u_colour.rgb * gnoblin.u_amount, gnoblin.u_amount);\n",
            "}\n"
        )
        .to_owned(),
        schema: schema.clone(),
    }));
    assert!(renderer.take_shader_error().is_none(), "the shader compiles");

    let mut pass = gnoblin_render::Shader::new(schema);
    pass.set_uniform("u_amount", UniformValue::Float(0.5)).expect("a float");
    pass.set_uniform("u_colour", UniformValue::Vec4([0.0, 0.0, 1.0, 1.0]))
        .expect("a vec4");
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 32, 32));
    let mut solid = Solid::new(Rect::new(0, 0, 32, 32), Color::WHITE);
    let mut effects = Effects::new();
    effects.push(Pass::Shader(pass)).expect("a shader pass");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    let [red, green, blue, alpha] = probe(&image, 16, 16);
    assert_eq!((red, green, alpha), (0, 0, 255), "half-alpha blue over black");
    assert!(
        (126..=129).contains(&blue),
        "the float reached the shader as 0.5, so the premultiplied blue is half of it: {blue}"
    );
}

#[test]
fn a_user_shader_refuses_a_mistyped_uniform() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // The registered shader declares a float; the pass declares a vec4. Uploading the vec4
    // into the float's four bytes would hand the shader a component nobody set.
    let registered = UniformSchema::new(vec![("u_amount", UniformType::Float)]).expect("a schema");
    renderer.set_user_shader(Some(UserShader {
        source: "fn shade(uv: vec2<f32>) -> vec4<f32> {\n    return vec4<f32>(gnoblin.u_amount);\n}\n".to_owned(),
        schema: registered,
    }));
    assert!(renderer.take_shader_error().is_none(), "the shader itself compiles");

    let mismatched = UniformSchema::new(vec![("u_amount", UniformType::Vec4)]).expect("a schema");
    let mut pass = gnoblin_render::Shader::new(mismatched);
    pass.set_uniform("u_amount", UniformValue::Vec4([1.0, 0.0, 0.0, 1.0]))
        .expect("a vec4 for the pass's own schema");
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 16, 16));
    let mut solid = Solid::new(Rect::new(0, 0, 16, 16), Color::from_rgba8(10, 20, 30, 255));
    let mut effects = Effects::new();
    effects.push(Pass::Shader(pass)).expect("a shader pass");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame is still drawn");

    assert_eq!(
        probe(&renderer.screenshot().expect("a readback"), 8, 8),
        [10, 20, 30, 255],
        "the layer was drawn by the default pass, not by a shader fed the wrong type"
    );
    let error = renderer.take_shader_error().expect("the refusal is reported");
    assert!(
        error.detail().contains("schema"),
        "the report names the disagreement: {error}"
    );
    assert!(renderer.take_shader_error().is_none(), "and is reported once");
}

#[test]
fn a_broken_user_shader_leaves_the_frame_drawn() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    renderer.set_user_shader(Some(UserShader {
        source: "fn shade(uv: vec2<f32>) -> vec4<f32> {\n    let colour = ;\n    return colour;\n}\n".to_owned(),
        schema: UniformSchema::default(),
    }));
    let error = renderer.take_shader_error().expect("the refusal is reported");
    assert_eq!(
        error.line(),
        2,
        "the line of the user's own source, not of the preamble: {error}"
    );

    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_damage(Rect::new(0, 0, 16, 16));
    let mut solid = Solid::new(Rect::new(0, 0, 16, 16), Color::from_rgba8(0, 255, 0, 255));
    let mut effects = Effects::new();
    effects
        .push(Pass::Shader(gnoblin_render::Shader::new(UniformSchema::default())))
        .expect("a shader pass");
    solid.effects = effects;
    frame.push_layer(Layer::Solid(solid));
    renderer
        .draw(&frame, Scale::IDENTITY)
        .expect("the frame is still drawn");
    assert_eq!(
        probe(&renderer.screenshot().expect("a readback"), 8, 8),
        [0, 255, 0, 255],
        "the layer fell back to the default pass"
    );
    renderer.draw(&frame, Scale::IDENTITY).expect("and so is the next one");
    assert!(
        renderer.take_shader_error().is_none(),
        "a broken shader is reported once, not once per frame"
    );
}

#[test]
fn a_syntax_error_names_the_user_line() {
    // The compiler's own check: an error in the user's second line must be reported as that
    // line, which is what subtracting the preamble's lines is for.
    let schema = UniformSchema::default();
    let mut device = match Device::new() {
        Ok(device) => device,
        Err(error) => {
            println!("skipping the shader-compiler test: {error}");
            return;
        }
    };
    let error = program::compile(
        &mut device,
        "fn shade(uv: vec2<f32>) -> vec4<f32> {\n    return not_a_function(uv);\n}\n",
        &schema,
    )
    .expect_err("an unknown function is refused");
    assert_eq!(error.line(), 2, "{error}");
    assert!(renderer_line_is_reported(&error), "{error}");
}

/// Whether an error's text names a line at all, so the report a caller prints is usable.
fn renderer_line_is_reported(error: &ShaderError) -> bool {
    error.to_string().contains("line 2")
}

#[test]
fn a_frame_does_not_inherit_the_last_frames_state() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // A frame that leaves a scissor, a blend and a corner mask behind.
    let mut first = OutputFrame::new();
    first.set_background(Color::from_rgba8(0, 0, 0, 255));
    first.push_damage(Rect::new(0, 0, 8, 8));
    let mut solid = Solid::new(Rect::new(0, 0, 8, 8), Color::from_rgba8(255, 255, 255, 128));
    solid.corner_radius = 8.0;
    first.push_layer(Layer::Solid(solid));
    renderer.draw(&first, Scale::IDENTITY).expect("the first frame");

    let second = frame_with(
        Color::from_rgba8(255, 0, 0, 255),
        size,
        Rect::new(0, 0, 32, 32),
        Color::from_rgba8(255, 0, 0, 255),
    );
    renderer.draw(&second, Scale::IDENTITY).expect("the second frame");
    let image = renderer.screenshot().expect("a readback");
    for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31), (16, 16)] {
        assert_eq!(
            probe(&image, x, y),
            [255, 0, 0, 255],
            "every pixel is the second frame's, including the corners the first frame's scissor \
             and mask did not cover"
        );
    }
}

#[test]
fn a_resize_replaces_the_target() {
    let Some(mut renderer) = gles(Size::new(8, 8)) else {
        return;
    };
    let frame = frame_with(
        Color::BLACK,
        Size::new(8, 8),
        Rect::new(0, 0, 8, 8),
        Color::from_rgba8(255, 0, 0, 255),
    );
    renderer.draw(&frame, Scale::IDENTITY).expect("the first frame");
    renderer.resize(Size::new(32, 16)).expect("a bigger target");
    assert_eq!(renderer.size(), Size::new(32, 16));
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(image.size(), Size::new(32, 16), "the readback follows the target");
    assert_eq!(
        probe(&image, 0, 0),
        [0, 0, 0, 0],
        "a replaced target holds no pixels, so the composer must damage everything"
    );
}

#[test]
fn a_texture_layer_is_sampled_the_right_way_up() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    // Four opaque, premultiplied pixels: red at the top left, green top right, blue bottom
    // left, white bottom right. Every probe says which corner of the texture reached which
    // corner of the quad, which is the whole of the mapping.
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let Some(texture) = renderer.device.texture((2, 2), &pixels).ok() else {
        println!("skipping the texture test: the driver refused a texture");
        return;
    };
    let quad = Rect::new(0, 0, 16, 16);
    renderer
        .draw_texture(texture, quad, [0.0, 0.0, 1.0, 1.0], Color::WHITE, 0.0)
        .expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 2, 2),
        [255, 0, 0, 255],
        "the texture's top left is the quad's top left"
    );
    assert_eq!(probe(&image, 13, 2), [0, 255, 0, 255], "top right");
    assert_eq!(probe(&image, 2, 13), [0, 0, 255, 255], "bottom left");
    assert_eq!(probe(&image, 13, 13), [255, 255, 255, 255], "bottom right");

    // A half tint on a premultiplied texture halves the colour and leaves the alpha, which is
    // the rule the software path applies to a tinted client buffer.
    renderer
        .draw_texture(texture, quad, [0.0, 0.0, 1.0, 1.0], Color::new(0.5, 0.5, 0.5, 1.0), 0.0)
        .expect("drawn again");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 13, 13),
        [128, 128, 128, 255],
        "the tint scaled every channel"
    );
}

#[test]
fn a_scale_converts_the_frame_to_physical_pixels() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = frame_with(
        Color::BLACK,
        size,
        Rect::new(0, 0, 4, 4),
        Color::from_rgba8(255, 0, 0, 255),
    );
    frame.push_damage(Rect::new(0, 0, 16, 16));
    renderer.draw(&frame, Scale::integral(2)).expect("drawn");
    let image = renderer.screenshot().expect("a readback");
    assert_eq!(
        probe(&image, 7, 7),
        [255, 0, 0, 255],
        "the layer covers 8 physical pixels"
    );
    assert_eq!(probe(&image, 8, 8), [0, 0, 0, 255], "and stops at the scaled edge");
}

#[test]
fn the_same_frame_draws_the_same_pixels() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let mut frame = OutputFrame::new();
    frame.set_background(Color::from_rgba8(16, 32, 48, 255));
    frame.push_damage(Rect::new(0, 0, 64, 64));
    frame.push_layer(Layer::Solid(Solid::new(
        Rect::new(0, 0, 32, 32),
        Color::from_rgba8(200, 40, 10, 255),
    )));
    let mut translucent = Solid::new(Rect::new(16, 16, 32, 32), Color::from_rgba8(0, 0, 0, 128));
    translucent.tint = Color::new(1.0, 0.5, 0.25, 1.0);
    frame.push_layer(Layer::Solid(translucent));

    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let gl = renderer.screenshot().expect("a readback");
    let mut software = Software::new(size).expect("a software renderer");
    software.draw(&frame, Scale::IDENTITY).expect("drawn");
    let reference = software.screenshot().expect("a readback");
    for (x, y) in [(2, 2), (20, 20), (40, 40), (60, 5), (5, 60), (30, 30)] {
        let probe_gl = probe(&gl, x, y);
        let probe_software = probe(&reference, x, y);
        for (gl_channel, software_channel) in probe_gl.iter().zip(probe_software.iter()) {
            assert!(
                gl_channel.abs_diff(*software_channel) <= 2,
                "at {x},{y}: GLES {probe_gl:?} and software {probe_software:?} disagree"
            );
        }
    }
}

/// A rectangle drawn in three dimensions: `matrix` maps its own plane, origin at its top-left
/// corner, to the output before the perspective divide.
fn spatial_solid(rect: Rect, colour: Color, matrix: Mat4, opaque: bool) -> Solid {
    let mut solid = Solid::new(rect, colour);
    solid.model = Some(Model { matrix, opaque });
    solid
}

/// A frame of `layers` painted over `background`, damaging the whole output.
fn layered_frame(background: Color, size: Size, layers: Vec<Solid>) -> OutputFrame {
    let mut frame = OutputFrame::new();
    frame.set_background(background);
    frame.push_damage(Rect::new(0, 0, size.width, size.height));
    for solid in layers {
        frame.push_layer(Layer::Solid(solid));
    }
    frame
}

/// The plane `z = slope * x + offset` over a rectangle's own coordinates.
fn tilted_plane(slope: f64, offset: f64) -> Mat4 {
    Mat4 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [slope, 0.0, 0.0, offset],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

#[test]
fn an_intersecting_pair_is_depth_tested_per_pixel() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let red = || {
        spatial_solid(
            Rect::new(0, 0, 16, 16),
            Color::from_rgba8(255, 0, 0, 255),
            Mat4::IDENTITY,
            true,
        )
    };
    let blue = || {
        spatial_solid(
            Rect::new(0, 0, 16, 16),
            Color::from_rgba8(0, 0, 255, 255),
            tilted_plane(0.1, -0.8),
            true,
        )
    };
    for layers in [vec![red(), blue()], vec![blue(), red()]] {
        renderer
            .draw(&layered_frame(Color::BLACK, size, layers), Scale::IDENTITY)
            .expect("drawn");
        let image = renderer.screenshot().expect("a readback");
        assert_eq!(
            probe(&image, 3, 4),
            [0, 0, 255, 255],
            "left of the crossing blue is nearer, whatever the submission order"
        );
        assert_eq!(probe(&image, 12, 4), [255, 0, 0, 255], "right of it red is nearer");
    }
}

#[test]
fn transparent_layers_blend_in_depth_order_as_the_flat_path_does() {
    let size = Size::new(16, 16);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let red = Color::from_rgba8(255, 0, 0, 128);
    let blue = Color::from_rgba8(0, 0, 255, 128);
    // Painter's order puts blue over red; the projected pair puts red behind blue and must blend
    // to the same pixel, whichever order it is submitted in.
    let flat = layered_frame(
        Color::BLACK,
        size,
        vec![
            Solid::new(Rect::new(0, 0, 16, 16), red),
            Solid::new(Rect::new(0, 0, 16, 16), blue),
        ],
    );
    renderer.draw(&flat, Scale::IDENTITY).expect("the flat pair");
    let expected = probe(&renderer.screenshot().expect("a readback"), 8, 8);
    let far = || spatial_solid(Rect::new(0, 0, 16, 16), red, Mat4::translation(0.0, 0.0, 0.5), false);
    let near = || spatial_solid(Rect::new(0, 0, 16, 16), blue, Mat4::translation(0.0, 0.0, -0.5), false);
    for layers in [vec![near(), far()], vec![far(), near()]] {
        renderer
            .draw(&layered_frame(Color::BLACK, size, layers), Scale::IDENTITY)
            .expect("the projected pair");
        let got = probe(&renderer.screenshot().expect("a readback"), 8, 8);
        assert!(
            got.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
            "projected {got:?}, flat path {expected:?}"
        );
    }
}

#[test]
fn a_rotated_quad_under_perspective_lands_on_its_projected_corners() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let (centre, distance, angle) = (32.0_f64, 20.0_f64, 30.0_f64);
    // A 32 by 32 quad centred on the output, turned about the vertical axis through its centre.
    let world = Mat4::translation(centre, centre, 0.0)
        .mul(&Mat4::rotation_y(angle))
        .mul(&Mat4::translation(-16.0, -16.0, 0.0));
    let matrix = Mat4::perspective(centre, centre, distance, 1.0, 100.0).mul(&world);
    let quad = spatial_solid(
        Rect::new(16, 16, 32, 32),
        Color::from_rgba8(255, 0, 0, 255),
        matrix,
        true,
    );
    renderer
        .draw(&layered_frame(Color::BLACK, size, vec![quad]), Scale::IDENTITY)
        .expect("drawn");
    let image = renderer.screenshot().expect("a readback");

    // A point at offset (a, b) from the centre is at (a cos, b, -a sin); the eye at `distance`
    // projects (X, Y, Z) to ((d X - c Z) / (d - Z), (d Y - c Z) / (d - Z)).
    let (sin, cos) = angle.to_radians().sin_cos();
    let corners: Vec<(f64, f64)> = [(-16.0, -16.0), (16.0, -16.0), (16.0, 16.0), (-16.0, 16.0)]
        .iter()
        .map(|&(a, b)| {
            let (x, y, z) = (centre + a * cos, centre + b, -a * sin);
            let w = distance - z;
            ((distance * x - centre * z) / w, (distance * y - centre * z) / w)
        })
        .collect();
    let edge = |i: usize| (corners[i], corners[(i + 1) % 4]);
    let area: f64 = (0..4)
        .map(|i| {
            let ((ax, ay), (bx, by)) = edge(i);
            ax * by - bx * ay
        })
        .sum();
    let orientation = area.signum();
    let (mut inside, mut outside) = (0, 0);
    for py in 0..64 {
        for px in 0..64 {
            let p = (f64::from(px) + 0.5, f64::from(py) + 0.5);
            // The signed distance to the nearest edge, positive inside. Pixels within a pixel of
            // an edge are skipped: rasterisation rounds there.
            let margin = (0..4)
                .map(|i| {
                    let ((ax, ay), (bx, by)) = edge(i);
                    let (ex, ey) = (bx - ax, by - ay);
                    orientation * (ex * (p.1 - ay) - ey * (p.0 - ax)) / (ex * ex + ey * ey).sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            if margin > 1.0 {
                assert_eq!(
                    probe(&image, px, py),
                    [255, 0, 0, 255],
                    "inside the projected quad at {px},{py}"
                );
                inside += 1;
            } else if margin < -1.0 {
                assert_eq!(
                    probe(&image, px, py),
                    [0, 0, 0, 255],
                    "outside the projected quad at {px},{py}"
                );
                outside += 1;
            }
        }
    }
    assert!(
        inside > 300 && outside > 300,
        "the check must see both sides: inside {inside}, outside {outside}"
    );
}

/// A shadow or blur on a projected layer is refused rather than drawn in the flat plane, so the
/// frame paints exactly the pixels it paints without the effects.
#[test]
fn an_effect_on_a_projected_layer_draws_nothing_extra() {
    let size = Size::new(32, 32);
    let Some(mut renderer) = gles(size) else {
        return;
    };
    let red = Color::from_rgba8(255, 0, 0, 255);
    let plain = layered_frame(
        Color::WHITE,
        size,
        vec![spatial_solid(Rect::new(8, 8, 16, 16), red, Mat4::IDENTITY, true)],
    );
    let mut solid = spatial_solid(Rect::new(8, 8, 16, 16), red, Mat4::IDENTITY, true);
    let mut effects = Effects::new();
    effects
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 4.0, 0.0),
            blur: 8.0,
            spread: 0.0,
            opacity: 0.5,
            color: Color::BLACK,
        }))
        .expect("a shadow");
    effects.push(Pass::Blur(blur(6.0))).expect("a blur");
    solid.effects = effects;
    let effectful = layered_frame(Color::WHITE, size, vec![solid]);

    renderer.draw(&plain, Scale::IDENTITY).expect("drawn");
    let expected = renderer.screenshot().expect("a readback");
    renderer.draw(&effectful, Scale::IDENTITY).expect("drawn");
    let actual = renderer.screenshot().expect("a readback");
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(probe(&actual, x, y), probe(&expected, x, y), "at {x},{y}");
        }
    }
}

/// The blur of the glass tests' backdrop, as `shader/wgsl/blur.wgsl` computes it: nine taps
/// `step` pixels apart with a Gaussian of sigma 1.5 taps, normalised. The backdrop is constant
/// down a column, so the vertical pass leaves it as it is.
fn reference_blur(backdrop: fn(i32) -> f32, x: i32, step: i32) -> f32 {
    let mut total = 0.0;
    let mut weights = 0.0;
    for index in -4..=4 {
        let weight = (-0.5 * (index * index) as f32 / 2.25).exp();
        total += weight * backdrop(x + index * step);
        weights += weight;
    }
    total / weights
}

/// The glass tests' backdrop: black left of x = 32, white from there on.
fn edge_backdrop(x: i32) -> f32 {
    if x >= 32 { 1.0 } else { 0.0 }
}

/// The glass tests' node colour: a straight (200, 100, 50) at `alpha`.
const GLASS_RGB: [f32; 3] = [200.0, 100.0, 50.0];

/// Levels of 8-bit error the glass tests allow: two blur passes and the blend each round.
const GLASS_TOLERANCE: f32 = 2.0;

/// The frame's colour at (x, 32) under isolated-group compositing, in 0..1 per channel:
/// `out = (1 - g) * D + g * (P + (1 - a) * B)` with `P` premultiplied.
fn glass_expected(opacity: f32, alpha: u8, x: i32) -> [f32; 3] {
    let a = f32::from(alpha) / 255.0;
    let d = edge_backdrop(x);
    let b = reference_blur(edge_backdrop, x, 3);
    GLASS_RGB.map(|channel| {
        let p = channel / 255.0 * a;
        (1.0 - opacity) * d + opacity * (p + (1.0 - a) * b)
    })
}

/// Renders the edge backdrop with a blurred node over `quad` at group `opacity`, and returns
/// the readback.
fn glass_over_edge(quad: Rect, opacity: f32, alpha: u8) -> Option<Image> {
    let size = Size::new(64, 64);
    let mut renderer = gles(size)?;
    let mut frame = OutputFrame::new();
    frame.set_background(Color::BLACK);
    frame.push_layer(Layer::Solid(Solid::new(Rect::new(32, 0, 32, 64), Color::WHITE)));
    let mut effects = Effects::new();
    effects.push(Pass::Blur(blur(9.0))).expect("a blur");
    let glass = LayerParts {
        quad,
        radius: 0.0,
        effects: &effects,
        colour: Some(Color::from_rgba8(200, 100, 50, alpha)),
        tint: Color::WHITE,
        opacity,
        src: [0.0, 0.0, 1.0, 1.0],
    };
    renderer
        .paint(&frame, Scale::IDENTITY, &[Rect::new(0, 0, 64, 64)], Some(&glass))
        .expect("drawn");
    Some(renderer.screenshot().expect("a readback"))
}

/// Asserts the probe at (x, 32) matches the isolated-group formula to `GLASS_TOLERANCE`.
fn assert_glass_pixel(image: &Image, x: i32, opacity: f32, alpha: u8) {
    let expected = glass_expected(opacity, alpha, x);
    let got = probe(image, x, 32);
    for channel in 0..3 {
        let want = expected[channel] * 255.0;
        assert!(
            (f32::from(got[channel]) - want).abs() <= GLASS_TOLERANCE,
            "g {opacity}, a {alpha}, x {x}, channel {channel}: got {}, want {want:.1}",
            got[channel]
        );
    }
}

#[test]
fn a_blurred_node_composites_as_one_isolated_group() {
    let quad = Rect::new(16, 0, 32, 64);
    // Translucent glass fully blurred behind; opaque glass at half opacity; a transparent
    // cut-out at half opacity, which is the blur alone; and a layer at zero opacity, which is
    // the backdrop alone.
    for (opacity, alpha) in [(1.0, 77), (0.5, 255), (0.5, 0), (0.0, 77)] {
        for x in [24, 40] {
            let Some(image) = glass_over_edge(quad, opacity, alpha) else {
                return;
            };
            assert_glass_pixel(&image, x, opacity, alpha);
        }
    }
}

#[test]
fn a_blur_reaches_nothing_outside_its_node() {
    // A probe just left of the edge is black in the backdrop, but its blur is not: the taps
    // reach the white side. Outside the node it must stay the backdrop's black.
    assert!(reference_blur(edge_backdrop, 28, 3) * 255.0 > 15.0);
    let Some(image) = glass_over_edge(Rect::new(40, 0, 8, 64), 1.0, 255) else {
        return;
    };
    let [red, _, _, _] = probe(&image, 28, 32);
    assert!(red <= 2, "outside the node the backdrop is untouched: {red}");
}

#[test]
fn a_blurred_node_fades_monotonically_with_group_opacity() {
    let quad = Rect::new(16, 0, 32, 64);
    let mut reds = Vec::new();
    for opacity in [1.0, 0.75, 0.5, 0.25, 0.0] {
        let Some(image) = glass_over_edge(quad, opacity, 77) else {
            return;
        };
        reds.push(probe(&image, 40, 32)[0]);
    }
    assert!(
        reds.windows(2).all(|pair| pair[1] > pair[0]),
        "the white side rises as the group fades to the backdrop: {reds:?}"
    );
}
