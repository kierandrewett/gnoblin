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
use crate::render::frame::Solid;
use gnoblin_render::{Blur, Effects, Shadow, UniformType, UniformValue, Vec3};

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

#[test]
fn a_blur_blurs_a_sharp_edge_behind_the_layer() {
    let size = Size::new(64, 64);
    let Some(mut renderer) = gles(size) else {
        return;
    };
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
    effects.push(Pass::Blur(Blur { radius: 12.0 })).expect("a blur");
    glass.effects = effects;
    frame.push_layer(Layer::Solid(glass));
    renderer.draw(&frame, Scale::IDENTITY).expect("drawn");
    let image = renderer.screenshot().expect("a readback");

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
    effects.push(Pass::Blur(Blur { radius: 10.0 })).expect("a blur");
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
