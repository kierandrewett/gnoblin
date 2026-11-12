//! A node's `effects` list on the wire and as render passes.
//!
//! This is the only place an `effect` entry becomes a [`Pass`] and back. A shell sets the list
//! with a `list_effect` value; each entry is checked here, and a snapshot or a change batch
//! echoes the passes the scene kept. The `window.add_effect` call and the window's `effect`
//! objects this replaced are gone.
//!
//! Only `blur` and `shadow` exist. Any other kind, `shader` included, is an error with a reason,
//! and the caller refuses the whole list rather than draw part of it. A value outside the
//! renderer's documented range is refused by [`Pass::validate`], and its text is the reason.

use gnoblin_proto::tree::effect;
use gnoblin_render::{BlurParams, Pass, Shadow};
use gnoblin_types::{Color, Vec3};

/// The blur radius, in logical pixels, every blur effect draws with.
///
/// The `effect` record has no radius property, so the compositor fixes one. It sits inside the
/// renderer's accepted range and is the single value to change if the record gains a radius.
const BLUR_RADIUS: f32 = 12.0;

/// The render pass one wire entry draws as.
///
/// # Errors
///
/// The reason the entry is refused: an unknown kind (`shader` is one), a colour that is not
/// `#rrggbb` or `#rrggbbaa`, or a value the renderer's range check rejects.
pub(super) fn pass_from_wire(entry: &effect::Properties) -> Result<Pass, String> {
    match entry.kind.as_str() {
        "blur" => blur_pass(entry),
        "shadow" => shadow_pass(entry),
        "shader" => Err("user shaders were removed: use a `blur` or `shadow` effect, or draw a `Canvas`".to_owned()),
        other => Err(format!("unknown effect kind `{other}`")),
    }
}

/// The wire entry a render pass echoes as. A blur's colour is the tint and a shadow's is its
/// colour; each channel is quantised to eight bits, the form the compositor accepts.
pub(super) fn wire_from_pass(pass: &Pass) -> effect::Properties {
    match pass {
        Pass::Blur(blur) => effect::Properties {
            kind: "blur".to_owned(),
            downscale: u64::from(blur.downscale.recip().round().clamp(1.0, f32::MAX) as u32),
            passes: u64::from(blur.passes),
            blur_radius: f64::from(blur.radius),
            brightness: f64::from(blur.brightness),
            contrast: f64::from(blur.contrast),
            saturation: f64::from(blur.saturation),
            noise: f64::from(blur.noise),
            tint: hex_colour(blur.tint),
            ..effect::Properties::default()
        },
        Pass::Shadow(shadow) => effect::Properties {
            kind: "shadow".to_owned(),
            offset_x: shadow.offset.x,
            offset_y: shadow.offset.y,
            blur_radius: f64::from(shadow.blur),
            spread: f64::from(shadow.spread),
            colour: hex_colour(shadow.color),
            ..effect::Properties::default()
        },
    }
}

/// The blur pass an entry asks for, validated against the renderer's ranges.
///
/// `downscale` on the wire is the divisor (`1 / downscale` of full resolution), so the pass's
/// fraction is its reciprocal. A divisor of zero gives a non-finite fraction and is refused by
/// the range check rather than here.
fn blur_pass(entry: &effect::Properties) -> Result<Pass, String> {
    let tint = if entry.tint.is_empty() {
        Color::TRANSPARENT
    } else {
        parse_colour(&entry.tint)?
    };
    let pass = Pass::Blur(BlurParams {
        radius: BLUR_RADIUS,
        passes: u32::try_from(entry.passes).unwrap_or(u32::MAX),
        downscale: 1.0 / u32::try_from(entry.downscale).unwrap_or(u32::MAX) as f32,
        tint,
        brightness: entry.brightness as f32,
        contrast: entry.contrast as f32,
        saturation: entry.saturation as f32,
        noise: entry.noise as f32,
    });
    pass.validate().map_err(|error| error.to_string())?;
    Ok(pass)
}

/// The shadow pass an entry asks for, validated against the renderer's ranges.
fn shadow_pass(entry: &effect::Properties) -> Result<Pass, String> {
    let pass = Pass::Shadow(Shadow {
        offset: Vec3 {
            x: entry.offset_x,
            y: entry.offset_y,
            z: 0.0,
        },
        // The wire's blur radius is in logical pixels; the pass stores f32.
        blur: entry.blur_radius as f32,
        spread: entry.spread as f32,
        // The renderer multiplies the colour's alpha by this, so full opacity here leaves the
        // colour's own alpha in control.
        opacity: 1.0,
        color: parse_colour(&entry.colour)?,
    });
    pass.validate().map_err(|error| error.to_string())?;
    Ok(pass)
}

/// Parses `#rrggbb` or `#rrggbbaa` into a colour.
///
/// Any other text, the empty string included, is an error, so an effect that needs a colour
/// refuses it instead of drawing in a guessed one.
fn parse_colour(text: &str) -> Result<Color, String> {
    let refused = || format!("`{text}` is not a #rrggbb or #rrggbbaa colour");
    let hex = text.strip_prefix('#').ok_or_else(refused)?;
    let channel = |range: std::ops::Range<usize>| -> Option<u8> { u8::from_str_radix(hex.get(range)?, 16).ok() };
    let parsed = match hex.len() {
        6 => channel(0..2)
            .zip(channel(2..4))
            .zip(channel(4..6))
            .map(|((red, green), blue)| Color::from_rgba8(red, green, blue, 255)),
        8 => channel(0..2)
            .zip(channel(2..4))
            .zip(channel(4..6))
            .zip(channel(6..8))
            .map(|(((red, green), blue), alpha)| Color::from_rgba8(red, green, blue, alpha)),
        _ => None,
    };
    parsed.ok_or_else(refused)
}

/// `#rrggbbaa` for a colour, each channel rounded to eight bits.
fn hex_colour(colour: Color) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        channel(colour.r),
        channel(colour.g),
        channel(colour.b),
        channel(colour.a),
    )
}

#[cfg(test)]
mod tests {
    //! The wire entry to pass mapping, ported from the window effect stack this replaced.

    use super::*;

    /// An entry with the record's documented defaults for the given kind.
    fn entry(kind: &str) -> effect::Properties {
        effect::Properties {
            kind: kind.to_owned(),
            downscale: 2,
            passes: 3,
            algorithm: "kawase".to_owned(),
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            blur_radius: 12.0,
            colour: "#00000080".to_owned(),
            ..effect::Properties::default()
        }
    }

    #[test]
    fn parses_six_and_eight_digit_colours() {
        assert_eq!(parse_colour("#ff0000"), Ok(Color::from_rgba8(255, 0, 0, 255)));
        assert_eq!(parse_colour("#00000080"), Ok(Color::from_rgba8(0, 0, 0, 128)));
        for bad in ["", "ff0000", "#ff00", "#gg0000", "#ff0000f"] {
            assert!(parse_colour(bad).is_err(), "`{bad}` is not a colour");
        }
    }

    #[test]
    fn blur_and_shadow_map_to_their_passes() {
        assert_eq!(pass_from_wire(&entry("blur")).map(|pass| pass.name()), Ok("blur"));
        assert_eq!(pass_from_wire(&entry("shadow")).map(|pass| pass.name()), Ok("shadow"));
    }

    #[test]
    fn blur_carries_the_wire_values_into_the_pass() {
        let mut blur = entry("blur");
        blur.downscale = 4;
        blur.passes = 5;
        blur.tint = "#ff000080".to_owned();
        blur.brightness = 1.5;
        blur.noise = 0.25;
        let Ok(Pass::Blur(params)) = pass_from_wire(&blur) else {
            panic!("a valid blur maps to a blur pass");
        };
        assert_eq!(params.radius, BLUR_RADIUS);
        assert_eq!(params.passes, 5);
        assert_eq!(params.downscale, 0.25);
        assert_eq!(params.tint, Color::from_rgba8(255, 0, 0, 128));
        assert_eq!(params.brightness, 1.5);
        assert_eq!(params.noise, 0.25);
    }

    #[test]
    fn shadow_carries_its_spread_into_the_pass() {
        let mut shadow = entry("shadow");
        shadow.spread = 4.0;
        let Ok(Pass::Shadow(pass)) = pass_from_wire(&shadow) else {
            panic!("a valid shadow maps to a shadow pass");
        };
        assert_eq!(pass.spread, 4.0);
    }

    #[test]
    fn out_of_range_values_are_refused_with_the_renderers_reason() {
        let mut blur = entry("blur");
        blur.passes = 0;
        let reason = pass_from_wire(&blur).expect_err("zero passes is refused");
        assert!(reason.contains("blur.passes"), "{reason}");

        let mut zero = entry("blur");
        zero.downscale = 0;
        assert!(pass_from_wire(&zero).is_err(), "a zero divisor is not a fraction");

        let mut tinted = entry("blur");
        tinted.tint = "red".to_owned();
        assert!(pass_from_wire(&tinted).is_err_and(|reason| reason.contains("colour")));
    }

    #[test]
    fn a_shader_and_an_unknown_kind_are_refused() {
        let reason = pass_from_wire(&entry("shader")).expect_err("no shader effects");
        assert!(reason.contains("shader"), "{reason}");
        let reason = pass_from_wire(&entry("glow")).expect_err("no such kind");
        assert!(reason.contains("glow"), "{reason}");
    }

    #[test]
    fn a_bad_shadow_colour_is_refused() {
        let mut shadow = entry("shadow");
        shadow.colour = "red".to_owned();
        assert!(pass_from_wire(&shadow).is_err());
    }

    #[test]
    fn a_pass_echoes_as_the_entry_that_makes_it() {
        let mut blur = entry("blur");
        blur.tint = "#10203040".to_owned();
        blur.passes = 2;
        let pass = pass_from_wire(&blur).expect("a blur");
        let echoed = wire_from_pass(&pass);
        assert_eq!(echoed.kind, "blur");
        assert_eq!((echoed.downscale, echoed.passes), (2, 2));
        assert_eq!(echoed.tint, "#10203040");
        assert_eq!(pass_from_wire(&echoed), Ok(pass), "the echo sets back to the same pass");

        let mut shadow = entry("shadow");
        shadow.offset_y = 6.0;
        let pass = pass_from_wire(&shadow).expect("a shadow");
        let echoed = wire_from_pass(&pass);
        assert_eq!((echoed.kind.as_str(), echoed.offset_y), ("shadow", 6.0));
        assert_eq!(echoed.colour, "#00000080");
        assert_eq!(pass_from_wire(&echoed), Ok(pass));
    }
}
