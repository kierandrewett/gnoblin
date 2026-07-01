//! Shared layer-shell windowing and Slint render runtime for gnoblin clients.

pub mod app_context_menu;
pub mod datetime;
pub mod notifcenter;
pub mod prefs;
pub mod shell;
pub mod theme;

mod layer_shell_runtime;

pub use layer_shell_runtime::{run, BarApp, BarConfig, BarMargins};

/// Push the resolved motion palette to a client's Slint `Theme` global.
#[macro_export]
macro_rules! apply_shell_motion {
    ($component:expr) => {{
        let motion = $crate::prefs::shell_motion();
        let theme = $component.global::<Theme>();
        $crate::apply_shell_motion_to_theme!(theme, motion)
    }};
}

/// Apply the light/dark preference to a client's Slint `Theme` global.
#[macro_export]
macro_rules! apply_shell_theme {
    ($component:expr) => {{
        let dark = $crate::theme::is_dark();
        let theme = $component.global::<Theme>();
        theme.set_mode(if dark {
            TokenMode::Dark
        } else {
            TokenMode::Light
        });
        $crate::apply_shell_chrome_to_theme!(theme, $crate::theme::shell_chrome(dark));
    }};
}

#[macro_export]
macro_rules! apply_shell_chrome_to_theme {
    ($theme:expr, $chrome:expr) => {{
        let theme = $theme;
        let chrome = $chrome;
        theme.set_panel_bg(chrome.panel_bg);
        theme.set_panel_fg(chrome.panel_fg);
        theme.set_panel_border_bottom(chrome.panel_border_bottom);
        theme.set_dock_bg(chrome.dock_bg);
        theme.set_dock_border(chrome.dock_border);
        theme.set_dock_highlight(chrome.dock_highlight);
        theme.set_surface_bg(chrome.surface_bg);
        theme.set_surface_raised_bg(chrome.surface_raised_bg);
        theme.set_surface_hover_bg(chrome.surface_hover_bg);
        theme.set_surface_active_bg(chrome.surface_active_bg);
        theme.set_critical_accent(chrome.critical_accent);
        theme.set_wallpaper_fallback_bg(chrome.wallpaper_fallback_bg);
        theme.set_surface_border(chrome.surface_border);
        theme.set_text_primary(chrome.text_primary);
        theme.set_text_secondary(chrome.text_secondary);
        theme.set_menu_bg(chrome.menu_bg);
        theme.set_menu_border(chrome.menu_border);
        theme.set_menu_highlight(chrome.menu_highlight);
        theme.set_chrome_shadow(chrome.chrome_shadow);
        theme.set_chrome_shadow_source(chrome.chrome_shadow_source);
        theme.set_dock_corner_radius(chrome.dock_corner_radius);
        theme.set_menu_corner_radius(chrome.menu_corner_radius);
        theme.set_popout_corner_radius(chrome.popout_corner_radius);
        theme.set_tooltip_corner_radius(chrome.tooltip_corner_radius);
        theme.set_control_corner_radius(chrome.control_corner_radius);
        theme.set_chrome_hairline_width(chrome.chrome_hairline_width);
        theme.set_chrome_highlight_height(chrome.chrome_highlight_height);
        theme.set_dock_shadow_blur(chrome.dock_shadow_blur);
        theme.set_dock_shadow_offset_y(chrome.dock_shadow_offset_y);
        theme.set_menu_shadow_blur(chrome.menu_shadow_blur);
        theme.set_menu_shadow_offset_y(chrome.menu_shadow_offset_y);
        theme.set_popout_shadow_blur(chrome.popout_shadow_blur);
        theme.set_popout_shadow_offset_y(chrome.popout_shadow_offset_y);
        theme.set_tooltip_shadow_blur(chrome.tooltip_shadow_blur);
        theme.set_tooltip_shadow_offset_y(chrome.tooltip_shadow_offset_y);
        theme.set_control_shadow_blur(chrome.control_shadow_blur);
        theme.set_control_shadow_offset_y(chrome.control_shadow_offset_y);
        theme.set_window_shadow_blur(chrome.window_shadow_blur);
        theme.set_window_shadow_offset_y(chrome.window_shadow_offset_y);
        theme.set_shell_font_family(chrome.font_family.clone().into());
    }};
}

#[macro_export]
macro_rules! apply_shell_motion_to_theme {
    ($theme:expr, $motion:expr) => {{
        let theme = $theme;
        let motion = $motion;
        let mut changed = false;
        macro_rules! set_f32 {
            ($get:ident, $set:ident, $value:expr) => {{
                let value = $value;
                if (theme.$get() - value).abs() > f32::EPSILON {
                    theme.$set(value);
                    changed = true;
                }
            }};
        }
        macro_rules! set_i32 {
            ($get:ident, $set:ident, $value:expr) => {{
                let value = $value;
                if theme.$get() != value {
                    theme.$set(value);
                    changed = true;
                }
            }};
        }

        set_f32!(get_motion_scale, set_motion_scale, motion.scale);
        set_f32!(get_motion_fast_ms, set_motion_fast_ms, motion.fast_ms);
        set_f32!(get_motion_medium_ms, set_motion_medium_ms, motion.medium_ms);
        set_f32!(
            get_motion_overlay_ms,
            set_motion_overlay_ms,
            motion.overlay_ms
        );
        set_f32!(
            get_motion_overlay_open_ms,
            set_motion_overlay_open_ms,
            motion.overlay_open_ms
        );
        set_f32!(
            get_motion_overlay_close_ms,
            set_motion_overlay_close_ms,
            motion.overlay_close_ms
        );
        set_f32!(get_motion_fade_ms, set_motion_fade_ms, motion.fade_ms);
        set_f32!(get_motion_page_ms, set_motion_page_ms, motion.page_ms);
        set_f32!(
            get_motion_overlay_slide_value,
            set_motion_overlay_slide_value,
            motion.overlay_slide
        );
        set_f32!(
            get_motion_overlay_scale_from_value,
            set_motion_overlay_scale_from_value,
            motion.overlay_scale_from
        );

        set_i32!(
            get_motion_fast_style,
            set_motion_fast_style,
            motion.fast_style
        );
        set_i32!(
            get_motion_medium_style,
            set_motion_medium_style,
            motion.medium_style
        );
        set_i32!(
            get_motion_overlay_style,
            set_motion_overlay_style,
            motion.overlay_style
        );
        set_i32!(
            get_motion_ease_out_style,
            set_motion_ease_out_style,
            motion.ease_out_style
        );
        set_i32!(
            get_motion_ease_in_style,
            set_motion_ease_in_style,
            motion.ease_in_style
        );
        set_i32!(
            get_motion_ease_in_out_style,
            set_motion_ease_in_out_style,
            motion.ease_in_out_style
        );
        set_i32!(
            get_motion_overlay_open_style,
            set_motion_overlay_open_style,
            motion.overlay_open_style
        );
        set_i32!(
            get_motion_overlay_close_style,
            set_motion_overlay_close_style,
            motion.overlay_close_style
        );
        set_i32!(
            get_motion_fade_style,
            set_motion_fade_style,
            motion.fade_style
        );
        set_i32!(
            get_motion_page_style,
            set_motion_page_style,
            motion.page_style
        );

        changed
    }};
}

/// Load the configured wallpaper (`[appearance] wallpaper`) as a blurred Slint
/// image for the shell glass backdrop.
pub fn load_backdrop() -> Option<slint::Image> {
    let cfg = gnoblin_core::config::Config::load();
    let path = cfg.get("appearance", "wallpaper")?;
    let img = image::open(path).ok()?;
    let small = img.resize(640, 400, image::imageops::FilterType::Triangle);
    let blurred = image::imageops::blur(&small.to_rgba8(), 18.0);
    let (w, h) = blurred.dimensions();
    let buf =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&blurred.into_raw(), w, h);
    Some(slint::Image::from_rgba8(buf))
}

/// Local time/date for the topbar clock, without pulling a date crate.
pub fn clock_and_date() -> (String, String) {
    (
        datetime::format_local("%H:%M:%S").unwrap_or_else(|| "00:00:00".to_string()),
        datetime::format_local("%a").unwrap_or_default(),
    )
}
