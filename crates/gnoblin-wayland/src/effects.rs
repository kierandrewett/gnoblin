//! Turns a window's observed `effect` stack into the scene's validated effect chain.
//!
//! The protocol side keeps each effect as the [`EffectSpec`] the client set. The scene
//! wants [`Effects`], whose passes are validated when pushed. This module is the one
//! place the two meet. It names only the scene's effect types, re-exported through
//! `gnoblin_backend::render`, so `gnoblin-wayland` never depends on the scene crate.
//!
//! Each spec is turned into its pass once, by [`pass_for`], and the same check decides
//! both whether the effect draws and the reason it does not: a value outside the scene's
//! documented range is refused by [`Pass::validate`] and the refusal text becomes the
//! spec's `reason`.

use gnoblin_backend::render::{BlurParams, Color, Effects, Pass, Shadow};
use gnoblin_types::Vec3;

use crate::observation::EffectSpec;

/// The blur radius, in logical pixels, every blur effect draws with.
///
/// The `effect` schema has no radius property, so the compositor fixes one. It sits
/// inside the scene's accepted range and is the single value to change if the schema
/// gains a radius.
pub(crate) const BLUR_RADIUS: f32 = 12.0;

/// Why a spec cannot draw, or `None` when it can.
///
/// The reason is what the client reads back in `reason` when `active` is false. A
/// disabled effect is not inactive: the caller never asks for its reason.
pub(crate) fn inactive_reason(spec: &EffectSpec) -> Option<String> {
    match spec.kind.as_str() {
        // The scene has no way to run a client's shader module yet: a shader pass needs
        // a compiled program and a uniform schema, and the spec supplies neither. Refuse
        // it rather than mark it active and draw nothing.
        "shader" => Some(if spec.source.is_empty() {
            "no shader source".to_owned()
        } else {
            "the renderer cannot run shader modules yet".to_owned()
        }),
        "blur" | "shadow" => pass_for(spec).err(),
        other => Some(format!("unknown effect kind `{other}`")),
    }
}

/// The scene pass a spec draws as.
///
/// `Ok(None)` for a shader, which has no pass yet. `Err` carries the reason the spec's
/// values are refused, so [`inactive_reason`] and [`chain`] share one check.
pub(crate) fn pass_for(spec: &EffectSpec) -> Result<Option<Pass>, String> {
    match spec.kind.as_str() {
        "blur" => blur_pass(spec).map(Some),
        "shadow" => shadow_pass(spec).map(Some),
        _ => Ok(None),
    }
}

/// The blur pass a spec asks for, validated against the scene's ranges.
///
/// `downscale` in the schema is the divisor (`1 / downscale` of full resolution), so the
/// scene's fraction is its reciprocal. A divisor of zero gives a non-finite fraction and
/// is refused by the scene's validation rather than here.
fn blur_pass(spec: &EffectSpec) -> Result<Pass, String> {
    let tint = if spec.tint.is_empty() {
        Color::TRANSPARENT
    } else {
        parse_colour(&spec.tint).ok_or_else(|| format!("`{}` is not a #rrggbb or #rrggbbaa colour", spec.tint))?
    };
    let pass = Pass::Blur(BlurParams {
        radius: BLUR_RADIUS,
        passes: spec.passes,
        downscale: 1.0 / spec.downscale as f32,
        tint,
        brightness: spec.brightness as f32,
        contrast: spec.contrast as f32,
        saturation: spec.saturation as f32,
        noise: spec.noise as f32,
    });
    pass.validate().map_err(|error| error.to_string())?;
    Ok(pass)
}

/// The shadow pass a spec asks for, validated against the scene's ranges.
fn shadow_pass(spec: &EffectSpec) -> Result<Pass, String> {
    let color =
        parse_colour(&spec.colour).ok_or_else(|| format!("`{}` is not a #rrggbb or #rrggbbaa colour", spec.colour))?;
    let pass = Pass::Shadow(Shadow {
        offset: Vec3 {
            x: spec.offset_x,
            y: spec.offset_y,
            z: 0.0,
        },
        // The schema's blur radius is in logical pixels; the scene stores f32.
        // Narrowing is exact for every value a client can reasonably set.
        blur: spec.blur_radius as f32,
        spread: spec.spread as f32,
        // The scene multiplies the colour's alpha by this, so full opacity here
        // leaves the colour's own alpha in control.
        opacity: 1.0,
        color,
    });
    pass.validate().map_err(|error| error.to_string())?;
    Ok(pass)
}

/// Builds the scene chain from a window's stack, in paint order.
///
/// Only enabled, active effects contribute. A pass the scene refuses is skipped, so the
/// chain is always one the renderer accepts; [`inactive_reason`] already marks such an
/// effect inactive, so the client sees why it does not draw.
#[must_use]
pub(crate) fn chain(specs: &[EffectSpec]) -> Effects {
    let mut effects = Effects::new();
    for spec in specs.iter().filter(|spec| spec.enabled && spec.active) {
        if let Ok(Some(pass)) = pass_for(spec) {
            // A refused pass leaves the chain unchanged, so skipping it is the whole
            // recovery; the spec is already marked inactive by the caller.
            let _ = effects.push(pass);
        }
    }
    effects
}

/// Parses `#rrggbb` or `#rrggbbaa` into a scene colour.
///
/// Returns `None` for any other text, including the empty string, so an effect that
/// needs a colour refuses it instead of drawing in a guessed one.
fn parse_colour(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    let channel = |range: std::ops::Range<usize>| -> Option<u8> { u8::from_str_radix(hex.get(range)?, 16).ok() };
    match hex.len() {
        6 => Some(Color::from_rgba8(channel(0..2)?, channel(2..4)?, channel(4..6)?, 255)),
        8 => Some(Color::from_rgba8(
            channel(0..2)?,
            channel(2..4)?,
            channel(4..6)?,
            channel(6..8)?,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the spec-to-chain mapping.

    use super::*;

    /// A spec with the schema defaults for the given kind.
    fn spec(kind: &str) -> EffectSpec {
        EffectSpec {
            kind: kind.to_owned(),
            enabled: true,
            active: true,
            reason: String::new(),
            downscale: 2,
            passes: 3,
            algorithm: "kawase".to_owned(),
            noise: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            tint: String::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 12.0,
            spread: 0.0,
            colour: "#00000080".to_owned(),
            source: String::new(),
            entry: "main".to_owned(),
        }
    }

    #[test]
    fn parses_six_and_eight_digit_colours() {
        assert_eq!(parse_colour("#ff0000"), Some(Color::from_rgba8(255, 0, 0, 255)));
        assert_eq!(parse_colour("#00000080"), Some(Color::from_rgba8(0, 0, 0, 128)));
        assert_eq!(parse_colour(""), None);
        assert_eq!(parse_colour("ff0000"), None);
        assert_eq!(parse_colour("#ff00"), None);
        assert_eq!(parse_colour("#gg0000"), None);
    }

    #[test]
    fn blur_and_shadow_map_to_passes_in_order() {
        let chain = chain(&[spec("blur"), spec("shadow")]);
        let names: Vec<&str> = chain.passes().iter().map(Pass::name).collect();
        assert_eq!(names, ["blur", "shadow"]);
    }

    #[test]
    fn blur_carries_the_schema_values_into_the_scene() {
        let mut blur = spec("blur");
        blur.downscale = 4;
        blur.passes = 5;
        blur.tint = "#ff000080".to_owned();
        blur.brightness = 1.5;
        blur.noise = 0.25;
        let Ok(Some(Pass::Blur(params))) = pass_for(&blur) else {
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
    fn shadow_carries_its_spread_into_the_scene() {
        let mut shadow = spec("shadow");
        shadow.spread = 4.0;
        let Ok(Some(Pass::Shadow(pass))) = pass_for(&shadow) else {
            panic!("a valid shadow maps to a shadow pass");
        };
        assert_eq!(pass.spread, 4.0);
    }

    #[test]
    fn out_of_range_values_are_inactive_with_the_scene_reason() {
        let mut blur = spec("blur");
        blur.passes = 0;
        let reason = inactive_reason(&blur).expect("zero passes is refused");
        assert!(reason.contains("blur.passes"), "{reason}");
        assert!(chain(&[blur]).is_empty());

        let mut tinted = spec("blur");
        tinted.tint = "red".to_owned();
        assert!(inactive_reason(&tinted).is_some_and(|reason| reason.contains("colour")));
    }

    #[test]
    fn disabled_and_inactive_effects_do_not_contribute() {
        let mut disabled = spec("blur");
        disabled.enabled = false;
        let mut inactive = spec("shadow");
        inactive.active = false;
        assert!(chain(&[disabled, inactive]).is_empty());
    }

    #[test]
    fn shader_is_inactive_with_a_reason() {
        let shader = spec("shader");
        assert_eq!(inactive_reason(&shader).as_deref(), Some("no shader source"));
        let mut sourced = spec("shader");
        sourced.source = "wobble".to_owned();
        assert_eq!(
            inactive_reason(&sourced).as_deref(),
            Some("the renderer cannot run shader modules yet")
        );
        assert!(matches!(pass_for(&sourced), Ok(None)));
    }

    #[test]
    fn bad_shadow_colour_is_inactive_with_a_reason() {
        let mut shadow = spec("shadow");
        shadow.colour = "red".to_owned();
        assert!(inactive_reason(&shadow).is_some());
        assert!(pass_for(&shadow).is_err());
    }
}
