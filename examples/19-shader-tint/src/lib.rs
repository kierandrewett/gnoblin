//! Example 19: a custom shader in Rust.
//!
//! A [`Filter::shader`] on the style of every window tints each of them: `effect` gets the window's colour and moves it a quarter of
//! the way towards a warm orange (`p[0]` to `p[2]` are the colour and `p[3]` how far to go). The library writes it to each window's node
//! as one `shader` effect.
//!
//! A shader is drawn only when the compositor's `compositor.user-shaders` setting is on, and only by a renderer that draws shaders. While
//! it is off the windows are drawn as they are, and `<Super>w` logs why: the style warnings of the windows, which name the
//! setting. The compositor checks the source before it accepts it: a source it refuses leaves the window's other effects as they were,
//! and the compositor's log says where the source is wrong.
//!
//! Build it with `cargo build -p gnoblin-example-shader-tint` and run it with `gnoblin ./target/debug/libshader_tint.so`. See
//! `docs/custom-shaders.md`.

use gnoblin_std::prelude::*;

/// The tint: `p[0]` to `p[2]` is the colour, `p[3]` the strength from 0 to 1. The window's colour is premultiplied, so the tint is
/// scaled by its alpha and a transparent corner stays transparent.
const TINT: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> {\n\
    let tint = vec3<f32>(p[0], p[1], p[2]) * c.a;\n\
    return vec4<f32>(mix(c.rgb, tint, p[3]), c.a);\n\
}\n";

#[gnoblin::main]
fn main(g: &Gnoblin) -> gnoblin_std::Result<()> {
    let std = g.std()?;
    std.clients()?
        .style()?
        .set_filter(&[Filter::shader(TINT, &[1.0, 0.55, 0.2, 0.25])])?;

    g.bind("<Super>w", move || {
        let said = (|| -> gnoblin_std::Result<Vec<String>> {
            let mut warnings = Vec::new();
            for client in g.clients()?.to_vec()? {
                warnings.extend(client.style_warnings()?);
            }
            Ok(warnings)
        })();
        match said {
            Ok(warnings) if warnings.is_empty() => g.log("shader tint: no window has style warnings"),
            Ok(warnings) => {
                for warning in warnings {
                    g.log(&format!("shader tint: {warning}"));
                }
            }
            Err(error) => g.log(&format!("shader tint: cannot read the style warnings: {error}")),
        }
    })?;

    g.log("shader tint shell is running");
    Ok(())
}
