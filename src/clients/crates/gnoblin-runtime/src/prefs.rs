//! The standard desktop "enable animations" preference
//! (`org.gnome.desktop.interface enable-animations`) — the same signal
//! GTK/libadwaita and other Wayland clients already honour. One setting drives
//! motion everywhere, so a software/headless session (e.g. the devkit) can turn
//! animations off for the whole desktop, not just gnoblin's clients.
//!
//! Read via the `gsettings` CLI (part of glib, always present where gnoblin
//! runs) rather than the XDG Settings portal: the read happens on the client's
//! show() path, and a portal call can block on D-Bus auto-activation for many
//! seconds when no portal is running (a bare devkit session) — which would
//! stall the first paint. The CLI reads the identical GSetting and returns
//! immediately. Defaults to animations-on if the value can't be read.

use gnoblin_core::config::Config;
use std::process::Command;
use std::time::Duration;

const CURVE_LINEAR: i32 = 0;
const CURVE_EASE: i32 = 1;
const CURVE_EASE_IN: i32 = 2;
const CURVE_EASE_OUT: i32 = 3;
const CURVE_EASE_IN_OUT: i32 = 4;
const CURVE_EASE_IN_QUAD: i32 = 5;
const CURVE_EASE_OUT_QUAD: i32 = 6;
const CURVE_EASE_IN_OUT_QUAD: i32 = 7;
const CURVE_EASE_IN_CUBIC: i32 = 8;
const CURVE_EASE_OUT_CUBIC: i32 = 9;
const CURVE_EASE_IN_OUT_CUBIC: i32 = 10;
const CURVE_EASE_IN_QUART: i32 = 11;
const CURVE_EASE_OUT_QUART: i32 = 12;
const CURVE_EASE_IN_OUT_QUART: i32 = 13;
const CURVE_EASE_IN_QUINT: i32 = 14;
const CURVE_EASE_OUT_QUINT: i32 = 15;
const CURVE_EASE_IN_OUT_QUINT: i32 = 16;
const CURVE_EASE_IN_SINE: i32 = 17;
const CURVE_EASE_OUT_SINE: i32 = 18;
const CURVE_EASE_IN_OUT_SINE: i32 = 19;
const CURVE_EASE_IN_EXPO: i32 = 20;
const CURVE_EASE_OUT_EXPO: i32 = 21;
const CURVE_EASE_IN_OUT_EXPO: i32 = 22;
const CURVE_EASE_IN_CIRC: i32 = 23;
const CURVE_EASE_OUT_CIRC: i32 = 24;
const CURVE_EASE_IN_OUT_CIRC: i32 = 25;
const CURVE_EASE_IN_BACK: i32 = 26;
const CURVE_EASE_OUT_BACK: i32 = 27;
const CURVE_EASE_IN_OUT_BACK: i32 = 28;
const CURVE_STANDARD: i32 = 29;
const CURVE_SPRING: i32 = 30;
// Signature reveal vocabulary (see MotionCurves c31-c33 in Tokens.slint).
const CURVE_SMOOTH: i32 = 31;
const CURVE_POP: i32 = 32;
const CURVE_POP_BACK: i32 = 33;

#[derive(Clone, Copy, Debug)]
pub struct ShellMotion {
    pub scale: f32,
    pub fast_ms: f32,
    pub medium_ms: f32,
    pub overlay_ms: f32,
    pub overlay_open_ms: f32,
    pub overlay_close_ms: f32,
    pub fade_ms: f32,
    pub page_ms: f32,
    pub overlay_slide: f32,
    pub overlay_scale_from: f32,
    pub fast_style: i32,
    pub medium_style: i32,
    pub overlay_style: i32,
    pub ease_out_style: i32,
    pub ease_in_style: i32,
    pub ease_in_out_style: i32,
    pub overlay_open_style: i32,
    pub overlay_close_style: i32,
    pub fade_style: i32,
    pub page_style: i32,
}

impl ShellMotion {
    fn defaults(scale: f32) -> Self {
        Self {
            scale,
            fast_ms: 120.0,
            medium_ms: 200.0,
            overlay_ms: 250.0,
            // Asymmetric reveal: a longer, fully-visible open that glides to
            // rest, and a brisk close that gets out of the way. Both ride the
            // same smooth decelerate curve so open/close feel like one gesture.
            overlay_open_ms: 250.0,
            overlay_close_ms: 150.0,
            fade_ms: 200.0,
            page_ms: 420.0,
            overlay_slide: 8.0,
            overlay_scale_from: 0.97,
            fast_style: CURVE_EASE_OUT,
            // Resizes and on-screen position moves glide on the signature curve.
            medium_style: CURVE_SMOOTH,
            overlay_style: CURVE_SMOOTH,
            ease_out_style: CURVE_EASE_OUT,
            ease_in_style: CURVE_EASE_IN,
            ease_in_out_style: CURVE_EASE_IN_OUT,
            overlay_open_style: CURVE_SMOOTH,
            overlay_close_style: CURVE_SMOOTH,
            fade_style: CURVE_EASE_IN_OUT,
            page_style: CURVE_EASE_OUT_EXPO,
        }
    }
}

fn config_bool(cfg: &Config, key: &str) -> Option<bool> {
    cfg.get("animations", key).and_then(parse_bool)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn read_gsetting() -> Option<bool> {
    // A plain `gsettings get` of a non-relocatable schema is a quick dconf read,
    // but guard against a pathological hang by capping the child's lifetime.
    let mut child = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Poll for completion up to ~500ms, then give up (and reap).
    for _ in 0..50 {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    let out = match child.try_wait() {
        Ok(Some(_)) => child.wait_with_output().ok()?,
        _ => {
            let _ = child.kill();
            return None;
        }
    };

    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Full Slint motion palette for the shared `Theme` global.
///
/// Compositor effects use `[animations] open/maximize/...`; native Slint chrome
/// uses `[animations] slint-*` because these are client-side widget animations
/// rather than Mutter window actors.
pub fn shell_motion() -> ShellMotion {
    let cfg = Config::load();
    let enabled = config_bool(&cfg, "enabled").unwrap_or_else(|| read_gsetting().unwrap_or(true));
    shell_motion_from_config(&cfg, enabled)
}

fn shell_motion_from_config(cfg: &Config, enabled: bool) -> ShellMotion {
    let mut motion = ShellMotion::defaults(if enabled { 1.0 } else { 0.0 });

    apply_motion_spec(
        cfg,
        "slint-fast",
        &mut motion.fast_ms,
        Some(&mut motion.fast_style),
    );
    apply_motion_spec(
        cfg,
        "slint-medium",
        &mut motion.medium_ms,
        Some(&mut motion.medium_style),
    );
    apply_motion_spec(
        cfg,
        "slint-overlay",
        &mut motion.overlay_ms,
        Some(&mut motion.overlay_style),
    );
    apply_motion_spec(
        cfg,
        "slint-overlay-open",
        &mut motion.overlay_open_ms,
        Some(&mut motion.overlay_open_style),
    );
    apply_motion_spec(
        cfg,
        "slint-overlay-close",
        &mut motion.overlay_close_ms,
        Some(&mut motion.overlay_close_style),
    );
    apply_motion_spec(
        cfg,
        "slint-fade",
        &mut motion.fade_ms,
        Some(&mut motion.fade_style),
    );
    apply_motion_spec(
        cfg,
        "slint-page",
        &mut motion.page_ms,
        Some(&mut motion.page_style),
    );

    if let Some(curve) = config_curve(cfg, "slint-ease-out") {
        motion.ease_out_style = curve;
    }
    if let Some(curve) = config_curve(cfg, "slint-ease-in") {
        motion.ease_in_style = curve;
    }
    if let Some(curve) = config_curve(cfg, "slint-ease-in-out") {
        motion.ease_in_out_style = curve;
    }
    if let Some(px) = config_px(cfg, "slint-overlay-slide") {
        motion.overlay_slide = px;
    }
    if let Some(scale) = config_float(cfg, "slint-overlay-scale-from") {
        motion.overlay_scale_from = scale.clamp(0.0, 1.0);
    }

    apply_anim_speed_debug(&mut motion);

    motion
}

/// Debug knob mirroring the compositor's `GNOBLIN_ANIM_SPEED`: a global playback
/// speed for every Slint animation, so one `export GNOBLIN_ANIM_SPEED=0.1` slows
/// shell chrome and Mutter window effects together for a `just devkit` session.
/// Slint's `Theme.motion-scale` is a duration multiplier, so we apply the inverse
/// of playback speed (0.1x speed => 10x duration). Left untouched when animations
/// are disabled (scale 0) so it can't accidentally re-enable them.
fn apply_anim_speed_debug(motion: &mut ShellMotion) {
    if motion.scale <= 0.0 {
        return;
    }
    if let Ok(raw) = std::env::var("GNOBLIN_ANIM_SPEED") {
        if let Ok(speed) = raw.trim().parse::<f32>() {
            if speed > 0.0 {
                motion.scale /= speed;
            }
        }
    }
}

fn apply_motion_spec(cfg: &Config, key: &str, duration_ms: &mut f32, curve: Option<&mut i32>) {
    let Some(spec) = cfg.get("animations", key).and_then(parse_motion_spec) else {
        return;
    };
    if let Some(v) = spec.duration_ms {
        *duration_ms = v;
    }
    if let (Some(dst), Some(v)) = (curve, spec.curve) {
        *dst = v;
    }
}

fn config_curve(cfg: &Config, key: &str) -> Option<i32> {
    cfg.get("animations", key).and_then(parse_curve)
}

fn config_float(cfg: &Config, key: &str) -> Option<f32> {
    cfg.get("animations", key).and_then(parse_float)
}

fn config_px(cfg: &Config, key: &str) -> Option<f32> {
    cfg.get("animations", key)
        .and_then(|raw| parse_float(raw.trim().trim_end_matches("px")))
        .map(|v| v.max(0.0))
}

#[derive(Default)]
struct MotionSpec {
    duration_ms: Option<f32>,
    curve: Option<i32>,
}

fn parse_motion_spec(raw: &str) -> Option<MotionSpec> {
    let fields = split_spec_fields(raw);
    let mut spec = MotionSpec::default();

    if let Some(first) = fields.first() {
        if let Some(v) = parse_duration_ms(first) {
            spec.duration_ms = Some(v);
        } else if let Some(curve) = parse_curve(first) {
            spec.curve = Some(curve);
        }
    }
    if let Some(second) = fields.get(1) {
        spec.curve = parse_curve(second).or(spec.curve);
    }

    (spec.duration_ms.is_some() || spec.curve.is_some()).then_some(spec)
}

fn split_spec_fields(raw: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut depth = 0;

    for (idx, ch) in raw.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                fields.push(raw[start..idx].trim());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    fields.push(raw[start..].trim());
    fields.retain(|field| !field.is_empty());
    fields
}

fn parse_duration_ms(raw: &str) -> Option<f32> {
    let value = raw.trim().to_ascii_lowercase();
    let parsed = if let Some(ms) = value.strip_suffix("ms") {
        parse_float(ms)?
    } else if let Some(seconds) = value.strip_suffix('s') {
        parse_float(seconds)? * 1000.0
    } else {
        parse_float(&value)?
    };
    Some(parsed.max(0.0))
}

fn parse_curve(raw: &str) -> Option<i32> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "linear" => Some(CURVE_LINEAR),
        "ease" => Some(CURVE_EASE),
        "ease-in" => Some(CURVE_EASE_IN),
        "ease-out" => Some(CURVE_EASE_OUT),
        "ease-in-out" => Some(CURVE_EASE_IN_OUT),
        "ease-in-quad" => Some(CURVE_EASE_IN_QUAD),
        "ease-out-quad" => Some(CURVE_EASE_OUT_QUAD),
        "ease-in-out-quad" => Some(CURVE_EASE_IN_OUT_QUAD),
        "ease-in-cubic" => Some(CURVE_EASE_IN_CUBIC),
        "ease-out-cubic" => Some(CURVE_EASE_OUT_CUBIC),
        "ease-in-out-cubic" => Some(CURVE_EASE_IN_OUT_CUBIC),
        "ease-in-quart" => Some(CURVE_EASE_IN_QUART),
        "ease-out-quart" => Some(CURVE_EASE_OUT_QUART),
        "ease-in-out-quart" => Some(CURVE_EASE_IN_OUT_QUART),
        "ease-in-quint" => Some(CURVE_EASE_IN_QUINT),
        "ease-out-quint" => Some(CURVE_EASE_OUT_QUINT),
        "ease-in-out-quint" => Some(CURVE_EASE_IN_OUT_QUINT),
        "ease-in-sine" => Some(CURVE_EASE_IN_SINE),
        "ease-out-sine" => Some(CURVE_EASE_OUT_SINE),
        "ease-in-out-sine" => Some(CURVE_EASE_IN_OUT_SINE),
        "ease-in-expo" => Some(CURVE_EASE_IN_EXPO),
        "ease-out-expo" => Some(CURVE_EASE_OUT_EXPO),
        "ease-in-out-expo" => Some(CURVE_EASE_IN_OUT_EXPO),
        "ease-in-circ" => Some(CURVE_EASE_IN_CIRC),
        "ease-out-circ" => Some(CURVE_EASE_OUT_CIRC),
        "ease-in-out-circ" => Some(CURVE_EASE_IN_OUT_CIRC),
        "ease-in-back" => Some(CURVE_EASE_IN_BACK),
        "ease-out-back" | "ease-out-bounce" | "ease-out-elastic" => Some(CURVE_EASE_OUT_BACK),
        "ease-in-out-back" | "ease-in-out-bounce" | "ease-in-out-elastic" => {
            Some(CURVE_EASE_IN_OUT_BACK)
        }
        "ease-in-bounce" | "ease-in-elastic" => Some(CURVE_EASE_IN_BACK),
        "standard" | "material-standard" => Some(CURVE_STANDARD),
        "spring" | "libadwaita-spring" => Some(CURVE_SPRING),
        "smooth" | "smooth-out" | "reveal" => Some(CURVE_SMOOTH),
        "pop" | "overshoot" | "soft-overshoot" => Some(CURVE_POP),
        "pop-back" | "overshoot-strong" | "spring-back" => Some(CURVE_POP_BACK),
        _ => None,
    }
}

fn parse_float(raw: &str) -> Option<f32> {
    raw.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use gnoblin_core::config::Config;

    use super::{
        parse_curve, parse_duration_ms, parse_motion_spec, shell_motion_from_config,
        CURVE_EASE_IN_CIRC, CURVE_EASE_IN_OUT_BACK, CURVE_EASE_IN_OUT_EXPO, CURVE_EASE_IN_QUINT,
        CURVE_EASE_OUT_QUINT, CURVE_EASE_OUT_SINE, CURVE_LINEAR, CURVE_SPRING, CURVE_STANDARD,
    };

    #[test]
    fn parses_ms_and_seconds_durations() {
        assert_eq!(parse_duration_ms("120"), Some(120.0));
        assert_eq!(parse_duration_ms("90ms"), Some(90.0));
        assert_eq!(parse_duration_ms("0.2s"), Some(200.0));
        assert_eq!(parse_duration_ms("-3ms"), Some(0.0));
    }

    #[test]
    fn parses_motion_specs_with_named_curves() {
        let spec = parse_motion_spec("145, ease-out-quint").unwrap();
        assert_eq!(spec.duration_ms, Some(145.0));
        assert_eq!(spec.curve, Some(CURVE_EASE_OUT_QUINT));

        assert_eq!(parse_curve("spring"), Some(CURVE_SPRING));
        assert_eq!(parse_curve("cubic-bezier(0.2, -0.1, 0.7, 1.2)"), None);
    }

    #[test]
    fn shell_motion_keeps_band_curves_independent_from_generic_curves() {
        let cfg = Config::from_text(
            "[animations]\n\
             slint-fast = 90, linear\n\
             slint-medium = 210, ease-in-quint\n\
             slint-overlay = 310, spring\n\
             slint-overlay-open = 125, standard\n\
             slint-overlay-close = 95, ease-out-sine\n\
             slint-fade = 77, ease-in-out-back\n\
             slint-ease-out = ease-out-quint\n\
             slint-ease-in = ease-in-circ\n\
             slint-ease-in-out = ease-in-out-expo\n\
             slint-overlay-slide = 13.5px\n\
             slint-overlay-scale-from = 0.82\n",
        );

        let motion = shell_motion_from_config(&cfg, true);
        assert_eq!(motion.scale, 1.0);
        assert_eq!(motion.fast_ms, 90.0);
        assert_eq!(motion.medium_ms, 210.0);
        assert_eq!(motion.overlay_ms, 310.0);
        assert_eq!(motion.overlay_open_ms, 125.0);
        assert_eq!(motion.overlay_close_ms, 95.0);
        assert_eq!(motion.fade_ms, 77.0);
        assert_eq!(motion.overlay_slide, 13.5);
        assert_eq!(motion.overlay_scale_from, 0.82);
        assert_eq!(motion.fast_style, CURVE_LINEAR);
        assert_eq!(motion.medium_style, CURVE_EASE_IN_QUINT);
        assert_eq!(motion.overlay_style, CURVE_SPRING);
        assert_eq!(motion.overlay_open_style, CURVE_STANDARD);
        assert_eq!(motion.overlay_close_style, CURVE_EASE_OUT_SINE);
        assert_eq!(motion.fade_style, CURVE_EASE_IN_OUT_BACK);
        assert_eq!(motion.ease_out_style, CURVE_EASE_OUT_QUINT);
        assert_eq!(motion.ease_in_style, CURVE_EASE_IN_CIRC);
        assert_eq!(motion.ease_in_out_style, CURVE_EASE_IN_OUT_EXPO);

        let disabled = shell_motion_from_config(&cfg, false);
        assert_eq!(disabled.scale, 0.0);
        assert_eq!(disabled.overlay_scale_from, 0.82);
    }
}
