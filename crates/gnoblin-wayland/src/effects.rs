//! Turns a window's observed `effect` stack into the scene's validated effect chain.
//!
//! The protocol side keeps each effect as the [`EffectSpec`] the client set. The scene
//! wants [`Effects`], whose passes are validated when pushed. This module is the one
//! place the two meet. It names only the scene's effect types, re-exported through
//! `gnoblin_backend::render`, so `gnoblin-wayland` never depends on the scene crate.
//!
//! Lossy by design: the schema describes blur settings (downscale, kernel, grain,
//! tint, colour grading) that the scene's [`Blur`] pass does not carry, and shadow
//! spread that its [`Shadow`] pass does not carry. Those values are kept on the spec
//! for the client to read back but do not reach the renderer.

use gnoblin_backend::render::{Blur, Color, Effects, Pass, Shadow};
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
        "shadow" => match parse_colour(&spec.colour) {
            Some(_) => None,
            None => Some(format!("`{}` is not a #rrggbb or #rrggbbaa colour", spec.colour)),
        },
        "blur" => None,
        other => Some(format!("unknown effect kind `{other}`")),
    }
}

/// The scene pass a spec draws as, or `None` when it has none.
///
/// A shader has no pass; an invalid shadow colour has none either and is reported by
/// [`inactive_reason`] first.
pub(crate) fn pass_for(spec: &EffectSpec) -> Option<Pass> {
    match spec.kind.as_str() {
        "blur" => Some(Pass::Blur(Blur { radius: BLUR_RADIUS })),
        "shadow" => {
            let color = parse_colour(&spec.colour)?;
            Some(Pass::Shadow(Shadow {
                offset: Vec3 {
                    x: spec.offset_x,
                    y: spec.offset_y,
                    z: 0.0,
                },
                // The schema's blur radius is in logical pixels; the scene stores f32.
                // Narrowing is exact for every value a client can reasonably set.
                blur: spec.blur_radius as f32,
                // The scene multiplies the colour's alpha by this, so full opacity here
                // leaves the colour's own alpha in control.
                opacity: 1.0,
                color,
            }))
        }
        _ => None,
    }
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
        if let Some(pass) = pass_for(spec) {
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
        assert!(pass_for(&sourced).is_none());
    }

    #[test]
    fn bad_shadow_colour_is_inactive_with_a_reason() {
        let mut shadow = spec("shadow");
        shadow.colour = "red".to_owned();
        assert!(inactive_reason(&shadow).is_some());
        assert!(pass_for(&shadow).is_none());
    }
}
