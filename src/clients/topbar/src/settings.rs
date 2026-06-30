use gnoblin_core::config::Config;

pub(crate) const DEFAULT_CLOCK_FORMAT: &str = "%a %d %b  %H:%M:%S";

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TopbarCommands {
    pub(crate) launcher: String,
    pub(crate) account: String,
    pub(crate) settings: String,
    pub(crate) power: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct WidgetSpec {
    pub(crate) kind: i32,
    pub(crate) flex: i32,
    pub(crate) size: i32,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TopbarLayout {
    pub(crate) left: Vec<WidgetSpec>,
    pub(crate) center: Vec<WidgetSpec>,
    pub(crate) right: Vec<WidgetSpec>,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TopbarGeometry {
    pub(crate) width: i32,
    pub(crate) align: i32,
    pub(crate) offset_x: i32,
    pub(crate) padding_left: i32,
    pub(crate) padding_right: i32,
    pub(crate) clock_padding: i32,
    pub(crate) status_padding: i32,
    pub(crate) status_icon_gap: i32,
    pub(crate) cc_offset_x: i32,
    pub(crate) cc_offset_y: i32,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TopbarSettings {
    pub(crate) commands: TopbarCommands,
    pub(crate) layout: TopbarLayout,
    pub(crate) geometry: TopbarGeometry,
    pub(crate) height: i32,
    pub(crate) exclusive_zone: i32,
    pub(crate) clock_format: String,
}

const WIDGET_SPACER: i32 = 0;
const WIDGET_SPRING: i32 = 1;
const WIDGET_SEPARATOR: i32 = 2;
const WIDGET_WORKSPACES: i32 = 3;
const WIDGET_FOCUSED_APP: i32 = 4;
const WIDGET_APPMENU: i32 = 5;
const WIDGET_CLOCK: i32 = 6;
const WIDGET_NOTIFICATIONS: i32 = 7;
const WIDGET_TRAY: i32 = 8;
const WIDGET_STATUS: i32 = 9;
const WIDGET_LAUNCHER: i32 = 10;

fn widget(kind: i32) -> WidgetSpec {
    WidgetSpec {
        kind,
        flex: if kind == WIDGET_SPRING { 1 } else { 0 },
        size: if kind == WIDGET_SPACER { 16 } else { 0 },
    }
}

fn parse_widget(raw: &str) -> Option<WidgetSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (name, arg) = raw
        .split_once(':')
        .map(|(name, arg)| (name.trim(), Some(arg.trim())))
        .unwrap_or((raw, None));
    let name = name.to_ascii_lowercase();
    let mut spec = match name.as_str() {
        "spacer" | "space" => widget(WIDGET_SPACER),
        "spring" | "flex" | "flexible-spacer" => widget(WIDGET_SPRING),
        "separator" | "sep" => widget(WIDGET_SEPARATOR),
        "workspaces" | "workspace" => widget(WIDGET_WORKSPACES),
        "focused-app" | "focused_app" | "app-title" | "app_title" => widget(WIDGET_FOCUSED_APP),
        "appmenu" | "global-menu" | "global_menu" => widget(WIDGET_APPMENU),
        "clock" | "datetime" => widget(WIDGET_CLOCK),
        "notifications" | "notification" | "bell" => widget(WIDGET_NOTIFICATIONS),
        "tray" | "status-notifier" | "status_notifier" => widget(WIDGET_TRAY),
        "status" | "quick-settings" | "quick_settings" | "control-centre" | "control_centre" => {
            widget(WIDGET_STATUS)
        }
        "launcher" | "search" | "spotlight" => widget(WIDGET_LAUNCHER),
        _ => return None,
    };
    if let Some(arg) = arg.and_then(|v| v.parse::<i32>().ok()) {
        if spec.kind == WIDGET_SPRING {
            spec.flex = arg.max(1);
        } else {
            spec.size = arg.max(0);
        }
    }
    Some(spec)
}

fn parse_widget_list(cfg: &Config, key: &str, fallback: &[WidgetSpec]) -> Vec<WidgetSpec> {
    let Some(raw) = cfg.get("topbar", key) else {
        return fallback.to_vec();
    };
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let parsed: Vec<WidgetSpec> = raw.split(',').filter_map(parse_widget).collect();
    if parsed.is_empty() {
        fallback.to_vec()
    } else {
        parsed
    }
}

fn topbar_layout(cfg: &Config) -> TopbarLayout {
    let default_left = vec![
        widget(WIDGET_WORKSPACES),
        widget(WIDGET_FOCUSED_APP),
        widget(WIDGET_APPMENU),
        widget(WIDGET_SPRING),
    ];
    let default_center = vec![widget(WIDGET_CLOCK)];
    let default_right = vec![
        widget(WIDGET_LAUNCHER),
        widget(WIDGET_TRAY),
        widget(WIDGET_STATUS),
    ];
    TopbarLayout {
        left: parse_widget_list(cfg, "left", &default_left),
        center: parse_widget_list(cfg, "center", &default_center),
        right: parse_widget_list(cfg, "right", &default_right),
    }
}

fn config_i32(cfg: &Config, section: &str, key: &str, fallback: i32) -> i32 {
    cfg.get(section, key)
        .and_then(parse_i32)
        .unwrap_or(fallback)
}

fn config_i32_any(cfg: &Config, section: &str, keys: &[&str], fallback: i32) -> i32 {
    keys.iter()
        .find_map(|key| cfg.get(section, key).and_then(parse_i32))
        .unwrap_or(fallback)
}

fn parse_i32(raw: &str) -> Option<i32> {
    raw.trim()
        .strip_suffix("px")
        .unwrap_or(raw.trim())
        .trim()
        .parse::<i32>()
        .ok()
}

fn parse_align(raw: Option<&str>) -> i32 {
    match raw.unwrap_or("full").trim().to_ascii_lowercase().as_str() {
        "left" | "start" => 0,
        "right" | "end" => 2,
        "center" | "centre" => 1,
        _ => 1,
    }
}

pub(crate) fn topbar_settings() -> TopbarSettings {
    let cfg = Config::load();
    let height = config_i32(&cfg, "topbar", "height", 34).max(1);
    let command = |key: &str, fallback: &str| {
        cfg.get("topbar", key)
            .map(str::to_string)
            .unwrap_or_else(|| fallback.to_string())
    };
    let commands = TopbarCommands {
        launcher: command("launcher", "gnoblin-launcher"),
        account: command("account", "gnome-control-center users"),
        settings: command("control_centre", "gnome-control-center"),
        power: command("power", "gnoblin-power-menu"),
    };
    TopbarSettings {
        commands,
        layout: topbar_layout(&cfg),
        geometry: TopbarGeometry {
            width: config_i32(&cfg, "topbar", "width", 0).max(0),
            align: parse_align(cfg.get("topbar", "align")),
            offset_x: config_i32_any(&cfg, "topbar", &["offset-x", "offset_x", "x"], 0),
            padding_left: config_i32_any(&cfg, "topbar", &["padding-left", "padding_left"], 12)
                .max(0),
            padding_right: config_i32_any(&cfg, "topbar", &["padding-right", "padding_right"], 12)
                .max(0),
            clock_padding: config_i32_any(&cfg, "topbar", &["clock-padding", "clock_padding"], 12)
                .max(0),
            status_padding: config_i32_any(
                &cfg,
                "topbar",
                &["status-padding", "status_padding"],
                10,
            )
            .max(0),
            status_icon_gap: config_i32_any(
                &cfg,
                "topbar",
                &["status-icon-gap", "status_icon_gap"],
                10,
            )
            .max(0),
            cc_offset_x: config_i32_any(
                &cfg,
                "topbar",
                &["quick-settings-offset-x", "quick_settings_offset_x"],
                6,
            ),
            cc_offset_y: config_i32_any(
                &cfg,
                "topbar",
                &["quick-settings-offset-y", "quick_settings_offset_y"],
                -4,
            ),
        },
        height,
        exclusive_zone: config_i32(&cfg, "topbar", "exclusive_zone", height).max(0),
        clock_format: command("clock-format", DEFAULT_CLOCK_FORMAT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_config(text: &str, f: impl FnOnce()) {
        let _guard = env_lock().lock().unwrap();
        let old = std::env::var("GNOBLIN_CONFIG").ok();
        let path = std::env::temp_dir().join(format!(
            "gnoblin-topbar-test-{}-{}.conf",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        std::fs::write(&path, text).unwrap();
        std::env::set_var("GNOBLIN_CONFIG", &path);

        f();

        if let Some(old) = old {
            std::env::set_var("GNOBLIN_CONFIG", old);
        } else {
            std::env::remove_var("GNOBLIN_CONFIG");
        }
        let _ = std::fs::remove_file(path);
    }

    fn kinds(widgets: &[WidgetSpec]) -> Vec<i32> {
        widgets.iter().map(|w| w.kind).collect()
    }

    #[test]
    fn missing_widget_zones_use_default_desktop_layout() {
        with_config("", || {
            let cfg = Config::load();
            let layout = topbar_layout(&cfg);

            assert_eq!(
                kinds(&layout.left),
                vec![
                    WIDGET_WORKSPACES,
                    WIDGET_FOCUSED_APP,
                    WIDGET_APPMENU,
                    WIDGET_SPRING,
                ]
            );
            assert_eq!(kinds(&layout.center), vec![WIDGET_CLOCK]);
            assert_eq!(
                kinds(&layout.right),
                vec![WIDGET_LAUNCHER, WIDGET_TRAY, WIDGET_STATUS]
            );
        });
    }

    #[test]
    fn invalid_nonempty_widget_zones_fall_back_to_defaults() {
        with_config(
            "[topbar]\n\
             left = clcok\n\
             center = misspelled\n\
             right = nope\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert_eq!(
                    kinds(&layout.left),
                    vec![
                        WIDGET_WORKSPACES,
                        WIDGET_FOCUSED_APP,
                        WIDGET_APPMENU,
                        WIDGET_SPRING,
                    ]
                );
                assert_eq!(kinds(&layout.center), vec![WIDGET_CLOCK]);
                assert_eq!(
                    kinds(&layout.right),
                    vec![WIDGET_LAUNCHER, WIDGET_TRAY, WIDGET_STATUS]
                );
            },
        );
    }

    #[test]
    fn topbar_settings_reads_clock_and_geometry_config() {
        with_config(
            "[topbar]\n\
             clock-format = %x %X\n\
             width = 900px\n\
             align = right\n\
             offset-x = -6px\n\
             padding-left = 10px\n\
             padding-right = 6px\n\
             clock-padding = 18px\n\
             status-padding = 3px\n\
             status-icon-gap = 4px\n\
             quick-settings-offset-x = 8px\n\
             quick-settings-offset-y = -6px\n",
            || {
                let settings = topbar_settings();
                assert_eq!(settings.clock_format, "%x %X");
                assert_eq!(settings.geometry.width, 900);
                assert_eq!(settings.geometry.align, 2);
                assert_eq!(settings.geometry.offset_x, -6);
                assert_eq!(settings.geometry.padding_left, 10);
                assert_eq!(settings.geometry.padding_right, 6);
                assert_eq!(settings.geometry.clock_padding, 18);
                assert_eq!(settings.geometry.status_padding, 3);
                assert_eq!(settings.geometry.status_icon_gap, 4);
                assert_eq!(settings.geometry.cc_offset_x, 8);
                assert_eq!(settings.geometry.cc_offset_y, -6);
            },
        );
    }

    #[test]
    fn explicit_empty_widget_zones_stay_empty() {
        with_config(
            "[topbar]\n\
             left =\n\
             center =\n\
             right =\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert!(layout.left.is_empty());
                assert!(layout.center.is_empty());
                assert!(layout.right.is_empty());
            },
        );
    }

    #[test]
    fn mixed_invalid_widget_lists_keep_valid_entries() {
        with_config(
            "[topbar]\n\
             left = nonsense, status, launcher, spring:3\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert_eq!(
                    kinds(&layout.left),
                    vec![WIDGET_STATUS, WIDGET_LAUNCHER, WIDGET_SPRING]
                );
                assert_eq!(layout.left[2].flex, 3);
            },
        );
    }
}
