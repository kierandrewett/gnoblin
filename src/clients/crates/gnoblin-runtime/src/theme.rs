//! Light/dark theme preference. Read from `[appearance] theme` (dark|light),
//! overridable live via a runtime file the control-centre's Dark Style toggle
//! writes — so a flip propagates to clients that poll it without a restart.

use slint::Color;
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct ShellChrome {
    pub panel_bg: Color,
    pub panel_fg: Color,
    pub panel_border_bottom: Color,
    pub dock_bg: Color,
    pub dock_border: Color,
    pub dock_highlight: Color,
    pub surface_bg: Color,
    pub surface_raised_bg: Color,
    pub surface_hover_bg: Color,
    pub surface_active_bg: Color,
    pub critical_accent: Color,
    pub wallpaper_fallback_bg: Color,
    pub surface_border: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub menu_bg: Color,
    pub menu_border: Color,
    pub menu_highlight: Color,
    pub chrome_shadow: Color,
    pub chrome_shadow_source: Color,
    pub dock_corner_radius: f32,
    pub menu_corner_radius: f32,
    pub popout_corner_radius: f32,
    pub tooltip_corner_radius: f32,
    pub control_corner_radius: f32,
    pub chrome_hairline_width: f32,
    pub chrome_highlight_height: f32,
    pub dock_shadow_blur: f32,
    pub dock_shadow_offset_y: f32,
    pub menu_shadow_blur: f32,
    pub menu_shadow_offset_y: f32,
    pub popout_shadow_blur: f32,
    pub popout_shadow_offset_y: f32,
    pub tooltip_shadow_blur: f32,
    pub tooltip_shadow_offset_y: f32,
    pub control_shadow_blur: f32,
    pub control_shadow_offset_y: f32,
    pub window_shadow_blur: f32,
    pub window_shadow_offset_y: f32,
    pub font_family: String,
}

fn override_path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-theme"))
}

/// True for dark mode (the default). Runtime override wins over config.
pub fn is_dark() -> bool {
    if let Some(p) = override_path() {
        if let Ok(s) = std::fs::read_to_string(&p) {
            return s.trim() != "light";
        }
    }
    let cfg = gnoblin_core::config::Config::load();
    cfg.get("appearance", "theme")
        .map(|t| t.trim() != "light")
        .unwrap_or(true)
}

/// Persist the live theme choice (the runtime override).
pub fn set_dark(dark: bool) {
    if let Some(p) = override_path() {
        let _ = std::fs::write(&p, if dark { "dark" } else { "light" });
    }
}

pub fn shell_chrome(dark: bool) -> ShellChrome {
    let cfg = gnoblin_core::config::Config::load();
    let mut chrome = if dark {
        ShellChrome {
            panel_bg: argb(0xdb, 0x24, 0x24, 0x24),
            panel_fg: argb(0xe0, 0xff, 0xff, 0xff),
            panel_border_bottom: argb(0x14, 0xff, 0xff, 0xff),
            dock_bg: argb(0x94, 0x1c, 0x1d, 0x1f),
            dock_border: argb(0x10, 0xff, 0xff, 0xff),
            dock_highlight: argb(0x16, 0xff, 0xff, 0xff),
            surface_bg: argb(0xf0, 0x26, 0x26, 0x27),
            surface_raised_bg: argb(0xf2, 0x34, 0x34, 0x36),
            surface_hover_bg: argb(0x14, 0xff, 0xff, 0xff),
            surface_active_bg: argb(0xf2, 0x35, 0x84, 0xe4),
            critical_accent: argb(0xf2, 0xe0, 0x49, 0x2f),
            wallpaper_fallback_bg: argb(0xff, 0x1e, 0x1e, 0x2e),
            surface_border: argb(0x1a, 0xff, 0xff, 0xff),
            text_primary: argb(0xeb, 0xff, 0xff, 0xff),
            text_secondary: argb(0x9e, 0xff, 0xff, 0xff),
            menu_bg: argb(0xbc, 0x26, 0x26, 0x28),
            menu_border: argb(0x1a, 0xff, 0xff, 0xff),
            menu_highlight: argb(0x14, 0xff, 0xff, 0xff),
            chrome_shadow: argb(0x2a, 0x00, 0x00, 0x00),
            chrome_shadow_source: argb(0x03, 0x00, 0x00, 0x00),
            dock_corner_radius: 20.0,
            menu_corner_radius: 12.0,
            popout_corner_radius: 18.0,
            tooltip_corner_radius: 10.0,
            control_corner_radius: 8.0,
            chrome_hairline_width: 1.0,
            chrome_highlight_height: 1.0,
            dock_shadow_blur: 36.0,
            dock_shadow_offset_y: 9.0,
            menu_shadow_blur: 28.0,
            menu_shadow_offset_y: 8.0,
            popout_shadow_blur: 34.0,
            popout_shadow_offset_y: 9.0,
            tooltip_shadow_blur: 18.0,
            tooltip_shadow_offset_y: 6.0,
            control_shadow_blur: 4.0,
            control_shadow_offset_y: 1.0,
            window_shadow_blur: 36.0,
            window_shadow_offset_y: 12.0,
            font_family: "Adwaita Sans".to_string(),
        }
    } else {
        ShellChrome {
            panel_bg: argb(0xdb, 0xfa, 0xfa, 0xfa),
            panel_fg: argb(0xcc, 0x00, 0x00, 0x00),
            panel_border_bottom: argb(0x1f, 0x00, 0x00, 0x00),
            dock_bg: argb(0x9e, 0xfa, 0xfa, 0xfa),
            dock_border: argb(0x0c, 0x00, 0x00, 0x00),
            dock_highlight: argb(0xb8, 0xff, 0xff, 0xff),
            surface_bg: argb(0xf5, 0xfa, 0xfa, 0xfa),
            surface_raised_bg: argb(0xf5, 0xff, 0xff, 0xff),
            surface_hover_bg: argb(0x0f, 0x00, 0x00, 0x00),
            surface_active_bg: argb(0xf2, 0x35, 0x84, 0xe4),
            critical_accent: argb(0xf2, 0xe0, 0x49, 0x2f),
            wallpaper_fallback_bg: argb(0xff, 0xf3, 0xf4, 0xf6),
            surface_border: argb(0x24, 0x00, 0x00, 0x00),
            text_primary: argb(0xdb, 0x00, 0x00, 0x00),
            text_secondary: argb(0x94, 0x00, 0x00, 0x00),
            menu_bg: argb(0xd8, 0xfa, 0xfa, 0xfa),
            menu_border: argb(0x1a, 0x00, 0x00, 0x00),
            menu_highlight: argb(0xd1, 0xff, 0xff, 0xff),
            chrome_shadow: argb(0x18, 0x00, 0x00, 0x00),
            chrome_shadow_source: argb(0x02, 0x00, 0x00, 0x00),
            dock_corner_radius: 20.0,
            menu_corner_radius: 12.0,
            popout_corner_radius: 18.0,
            tooltip_corner_radius: 10.0,
            control_corner_radius: 8.0,
            chrome_hairline_width: 1.0,
            chrome_highlight_height: 1.0,
            dock_shadow_blur: 34.0,
            dock_shadow_offset_y: 8.0,
            menu_shadow_blur: 26.0,
            menu_shadow_offset_y: 7.0,
            popout_shadow_blur: 32.0,
            popout_shadow_offset_y: 8.0,
            tooltip_shadow_blur: 16.0,
            tooltip_shadow_offset_y: 5.0,
            control_shadow_blur: 4.0,
            control_shadow_offset_y: 1.0,
            window_shadow_blur: 34.0,
            window_shadow_offset_y: 10.0,
            font_family: "Adwaita Sans".to_string(),
        }
    };

    if let Some(c) = config_color(&cfg, "panel-background") {
        chrome.panel_bg = c;
    }
    if let Some(c) = config_color(&cfg, "panel-foreground") {
        chrome.panel_fg = c;
    }
    if let Some(c) = config_color(&cfg, "panel-border") {
        chrome.panel_border_bottom = c;
    }
    if let Some(c) = config_color(&cfg, "dock-background") {
        chrome.dock_bg = c;
    }
    if let Some(c) = config_color(&cfg, "dock-border") {
        chrome.dock_border = c;
    }
    if let Some(c) = config_color(&cfg, "dock-highlight") {
        chrome.dock_highlight = c;
    }
    if let Some(c) = config_color(&cfg, "surface-background") {
        chrome.surface_bg = c;
    }
    if let Some(c) = config_color(&cfg, "surface-raised-background") {
        chrome.surface_raised_bg = c;
    }
    if let Some(c) = config_color(&cfg, "surface-hover-background") {
        chrome.surface_hover_bg = c;
    }
    if let Some(c) = config_color(&cfg, "surface-active-background") {
        chrome.surface_active_bg = c;
    }
    if let Some(c) = config_color(&cfg, "critical-accent") {
        chrome.critical_accent = c;
    }
    if let Some(c) = config_color(&cfg, "wallpaper-fallback-background") {
        chrome.wallpaper_fallback_bg = c;
    }
    if let Some(c) = config_color(&cfg, "surface-border") {
        chrome.surface_border = c;
    }
    if let Some(c) = config_color(&cfg, "text-primary") {
        chrome.text_primary = c;
    }
    if let Some(c) = config_color(&cfg, "text-secondary") {
        chrome.text_secondary = c;
    }
    if let Some(c) = config_color(&cfg, "menu-background") {
        chrome.menu_bg = c;
    }
    if let Some(c) = config_color(&cfg, "menu-border") {
        chrome.menu_border = c;
    }
    if let Some(c) = config_color(&cfg, "menu-highlight") {
        chrome.menu_highlight = c;
    }
    if let Some(c) = config_color(&cfg, "chrome-shadow") {
        chrome.chrome_shadow = c;
    }
    if let Some(c) = config_color(&cfg, "chrome-shadow-source") {
        chrome.chrome_shadow_source = c;
    }
    if let Some(v) = config_px(&cfg, "dock-corner-radius") {
        chrome.dock_corner_radius = v;
    }
    if let Some(v) = config_px(&cfg, "menu-corner-radius") {
        chrome.menu_corner_radius = v;
    }
    if let Some(v) = config_px(&cfg, "popout-corner-radius") {
        chrome.popout_corner_radius = v;
    }
    if let Some(v) = config_px(&cfg, "tooltip-corner-radius") {
        chrome.tooltip_corner_radius = v;
    }
    if let Some(v) = config_px(&cfg, "control-corner-radius") {
        chrome.control_corner_radius = v;
    }
    if let Some(v) = config_px(&cfg, "chrome-hairline-width") {
        chrome.chrome_hairline_width = v;
    }
    if let Some(v) = config_px(&cfg, "chrome-highlight-height") {
        chrome.chrome_highlight_height = v;
    }
    if let Some(v) = config_px(&cfg, "dock-shadow-blur") {
        chrome.dock_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "dock-shadow-offset-y") {
        chrome.dock_shadow_offset_y = v;
    }
    if let Some(v) = config_px(&cfg, "menu-shadow-blur") {
        chrome.menu_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "menu-shadow-offset-y") {
        chrome.menu_shadow_offset_y = v;
    }
    if let Some(v) = config_px(&cfg, "popout-shadow-blur") {
        chrome.popout_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "popout-shadow-offset-y") {
        chrome.popout_shadow_offset_y = v;
    }
    if let Some(v) = config_px(&cfg, "tooltip-shadow-blur") {
        chrome.tooltip_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "tooltip-shadow-offset-y") {
        chrome.tooltip_shadow_offset_y = v;
    }
    if let Some(v) = config_px(&cfg, "control-shadow-blur") {
        chrome.control_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "control-shadow-offset-y") {
        chrome.control_shadow_offset_y = v;
    }
    if let Some(v) = config_px(&cfg, "window-shadow-blur") {
        chrome.window_shadow_blur = v;
    }
    if let Some(v) = config_px(&cfg, "window-shadow-offset-y") {
        chrome.window_shadow_offset_y = v;
    }
    chrome.font_family = configured_font_family(&cfg);
    chrome
}

fn config_color(cfg: &gnoblin_core::config::Config, key: &str) -> Option<Color> {
    cfg.get("appearance", key).and_then(parse_color)
}

fn config_px(cfg: &gnoblin_core::config::Config, key: &str) -> Option<f32> {
    cfg.get("appearance", key).and_then(parse_px)
}

fn configured_font_family(cfg: &gnoblin_core::config::Config) -> String {
    cfg.get("appearance", "font-family")
        .and_then(config_font_family)
        .or_else(system_font_family)
        .unwrap_or_else(|| "Adwaita Sans".to_string())
}

fn config_font_family(raw: &str) -> Option<String> {
    let family = strip_quotes(raw.trim());
    if family.eq_ignore_ascii_case("system") || family.eq_ignore_ascii_case("auto") {
        return None;
    }
    (!family.is_empty()).then(|| family.to_string())
}

fn system_font_family() -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    parse_system_font_name(&raw)
}

fn parse_system_font_name(raw: &str) -> Option<String> {
    let mut family = strip_quotes(raw.trim()).trim().to_string();
    if family.is_empty() {
        return None;
    }
    if let Some((head, tail)) = family.rsplit_once(' ') {
        if tail.parse::<f32>().is_ok() {
            family = head.trim().to_string();
        }
    }
    (!family.is_empty()).then_some(family)
}

fn strip_quotes(raw: &str) -> &str {
    raw.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(raw)
}

pub fn parse_color(raw: &str) -> Option<Color> {
    let h = raw.trim().trim_start_matches('#');
    match h.len() {
        3 => {
            let mut chars = h.chars();
            let r = nibble(chars.next()?)?;
            let g = nibble(chars.next()?)?;
            let b = nibble(chars.next()?)?;
            Some(argb(0xff, r * 17, g * 17, b * 17))
        }
        4 => {
            let mut chars = h.chars();
            let r = nibble(chars.next()?)?;
            let g = nibble(chars.next()?)?;
            let b = nibble(chars.next()?)?;
            let a = nibble(chars.next()?)?;
            Some(argb(a * 17, r * 17, g * 17, b * 17))
        }
        6 => Some(argb(
            0xff,
            byte(&h[0..2])?,
            byte(&h[2..4])?,
            byte(&h[4..6])?,
        )),
        8 => Some(argb(
            byte(&h[6..8])?,
            byte(&h[0..2])?,
            byte(&h[2..4])?,
            byte(&h[4..6])?,
        )),
        _ => None,
    }
}

pub fn parse_px(raw: &str) -> Option<f32> {
    let value = raw.trim().strip_suffix("px").unwrap_or(raw.trim()).trim();
    let px: f32 = value.parse().ok()?;
    px.is_finite().then_some(px.max(0.0))
}

const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
    Color::from_argb_u8(a, r, g, b)
}

fn byte(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

fn nibble(c: char) -> Option<u8> {
    c.to_digit(16).map(|v| v as u8)
}

#[cfg(test)]
mod tests {
    use super::{argb, parse_color, parse_px, parse_system_font_name, shell_chrome};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn parses_rgb_and_rgba_hex() {
        assert_eq!(parse_color("#123456"), Some(argb(0xff, 0x12, 0x34, 0x56)));
        assert_eq!(parse_color("#12345680"), Some(argb(0x80, 0x12, 0x34, 0x56)));
    }

    #[test]
    fn parses_short_hex() {
        assert_eq!(parse_color("#abc"), Some(argb(0xff, 0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_color("#abcd"), Some(argb(0xdd, 0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn rejects_invalid_colours() {
        assert_eq!(parse_color("rgba(0,0,0,.3)"), None);
        assert_eq!(parse_color("#12"), None);
    }

    #[test]
    fn parses_configurable_pixel_lengths() {
        assert_eq!(parse_px("12"), Some(12.0));
        assert_eq!(parse_px("12.5px"), Some(12.5));
        assert_eq!(parse_px("-4px"), Some(0.0));
        assert_eq!(parse_px("nope"), None);
    }

    #[test]
    fn parses_gnome_system_font_family() {
        assert_eq!(
            parse_system_font_name("'Cantarell 11'"),
            Some("Cantarell".to_string())
        );
        assert_eq!(
            parse_system_font_name("'Adwaita Sans 10.5'"),
            Some("Adwaita Sans".to_string())
        );
        assert_eq!(
            parse_system_font_name("'Noto Sans Bold 12'"),
            Some("Noto Sans Bold".to_string())
        );
    }

    #[test]
    fn shell_chrome_reads_all_configurable_appearance_values() {
        let _guard = env_lock().lock().unwrap();
        let old = std::env::var("GNOBLIN_CONFIG").ok();
        let path = std::env::temp_dir().join(format!(
            "gnoblin-theme-test-{}-{}.conf",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        std::fs::write(
            &path,
            "[appearance]\n\
             panel-background = \"#01020304\"\n\
             panel-foreground = \"#11121314\"\n\
             panel-border = \"#21222324\"\n\
             dock-background = \"#31323334\"\n\
             dock-border = \"#41424344\"\n\
             dock-highlight = \"#51525354\"\n\
             surface-background = \"#61626364\"\n\
             surface-raised-background = \"#71727374\"\n\
             surface-hover-background = \"#81828384\"\n\
             surface-active-background = \"#91929394\"\n\
             critical-accent = \"#95969798\"\n\
             wallpaper-fallback-background = \"#a5a6a7a8\"\n\
             surface-border = \"#a1a2a3a4\"\n\
             text-primary = \"#b1b2b3b4\"\n\
             text-secondary = \"#c1c2c3c4\"\n\
             menu-background = \"#d1d2d3d4\"\n\
             menu-border = \"#e1e2e3e4\"\n\
             menu-highlight = \"#f1f2f3f4\"\n\
             chrome-shadow = \"#05060708\"\n\
             chrome-shadow-source = \"#15161718\"\n\
             dock-corner-radius = \"21px\"\n\
             menu-corner-radius = \"13.5\"\n\
             popout-corner-radius = \"19px\"\n\
             tooltip-corner-radius = \"11px\"\n\
             control-corner-radius = \"7px\"\n\
             chrome-hairline-width = \"0.75px\"\n\
             chrome-highlight-height = \"1.5px\"\n\
             dock-shadow-blur = \"37px\"\n\
             dock-shadow-offset-y = \"10px\"\n\
             menu-shadow-blur = \"29px\"\n\
             menu-shadow-offset-y = \"8.5px\"\n\
             popout-shadow-blur = \"35px\"\n\
             popout-shadow-offset-y = \"9.5px\"\n\
             tooltip-shadow-blur = \"17px\"\n\
             tooltip-shadow-offset-y = \"5.5px\"\n\
             control-shadow-blur = \"4.5px\"\n\
             control-shadow-offset-y = \"1.25px\"\n\
             window-shadow-blur = \"39px\"\n\
             window-shadow-offset-y = \"11.5px\"\n\
             font-family = \"Config Sans\"\n",
        )
        .unwrap();
        std::env::set_var("GNOBLIN_CONFIG", &path);

        let chrome = shell_chrome(true);
        assert_eq!(chrome.panel_bg, argb(0x04, 0x01, 0x02, 0x03));
        assert_eq!(chrome.panel_fg, argb(0x14, 0x11, 0x12, 0x13));
        assert_eq!(chrome.panel_border_bottom, argb(0x24, 0x21, 0x22, 0x23));
        assert_eq!(chrome.dock_bg, argb(0x34, 0x31, 0x32, 0x33));
        assert_eq!(chrome.dock_border, argb(0x44, 0x41, 0x42, 0x43));
        assert_eq!(chrome.dock_highlight, argb(0x54, 0x51, 0x52, 0x53));
        assert_eq!(chrome.surface_bg, argb(0x64, 0x61, 0x62, 0x63));
        assert_eq!(chrome.surface_raised_bg, argb(0x74, 0x71, 0x72, 0x73));
        assert_eq!(chrome.surface_hover_bg, argb(0x84, 0x81, 0x82, 0x83));
        assert_eq!(chrome.surface_active_bg, argb(0x94, 0x91, 0x92, 0x93));
        assert_eq!(chrome.critical_accent, argb(0x98, 0x95, 0x96, 0x97));
        assert_eq!(chrome.wallpaper_fallback_bg, argb(0xa8, 0xa5, 0xa6, 0xa7));
        assert_eq!(chrome.surface_border, argb(0xa4, 0xa1, 0xa2, 0xa3));
        assert_eq!(chrome.text_primary, argb(0xb4, 0xb1, 0xb2, 0xb3));
        assert_eq!(chrome.text_secondary, argb(0xc4, 0xc1, 0xc2, 0xc3));
        assert_eq!(chrome.menu_bg, argb(0xd4, 0xd1, 0xd2, 0xd3));
        assert_eq!(chrome.menu_border, argb(0xe4, 0xe1, 0xe2, 0xe3));
        assert_eq!(chrome.menu_highlight, argb(0xf4, 0xf1, 0xf2, 0xf3));
        assert_eq!(chrome.chrome_shadow, argb(0x08, 0x05, 0x06, 0x07));
        assert_eq!(chrome.chrome_shadow_source, argb(0x18, 0x15, 0x16, 0x17));
        assert_eq!(chrome.dock_corner_radius, 21.0);
        assert_eq!(chrome.menu_corner_radius, 13.5);
        assert_eq!(chrome.popout_corner_radius, 19.0);
        assert_eq!(chrome.tooltip_corner_radius, 11.0);
        assert_eq!(chrome.control_corner_radius, 7.0);
        assert_eq!(chrome.chrome_hairline_width, 0.75);
        assert_eq!(chrome.chrome_highlight_height, 1.5);
        assert_eq!(chrome.dock_shadow_blur, 37.0);
        assert_eq!(chrome.dock_shadow_offset_y, 10.0);
        assert_eq!(chrome.menu_shadow_blur, 29.0);
        assert_eq!(chrome.menu_shadow_offset_y, 8.5);
        assert_eq!(chrome.popout_shadow_blur, 35.0);
        assert_eq!(chrome.popout_shadow_offset_y, 9.5);
        assert_eq!(chrome.tooltip_shadow_blur, 17.0);
        assert_eq!(chrome.tooltip_shadow_offset_y, 5.5);
        assert_eq!(chrome.control_shadow_blur, 4.5);
        assert_eq!(chrome.control_shadow_offset_y, 1.25);
        assert_eq!(chrome.window_shadow_blur, 39.0);
        assert_eq!(chrome.window_shadow_offset_y, 11.5);
        assert_eq!(chrome.font_family, "Config Sans");

        if let Some(old) = old {
            std::env::set_var("GNOBLIN_CONFIG", old);
        } else {
            std::env::remove_var("GNOBLIN_CONFIG");
        }
        let _ = std::fs::remove_file(path);
    }
}
