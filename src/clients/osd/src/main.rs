//! gnoblin-osd — a Slint volume/brightness on-screen display.
//!
//! `gnoblin-osd volume` / `gnoblin-osd brightness` (typically from the media-key
//! binds, after the value changes). A full-screen, input-passthrough overlay
//! showing a glass pill near the bottom; tears itself down after a short timeout.

use gnoblin_shell_ui::{run, BarApp, BarConfig, RuntimeError};
slint::include_modules!(); // Osd
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::time::{Duration, Instant};

const SHOW: Duration = Duration::from_millis(1500);

struct OsdApp {
    win: Option<Osd>,
    is_brightness: bool,
    level: f32,
    muted: bool,
    deadline: Option<Instant>,
}

fn apply_theme(win: &Osd) {
    let dark = gnoblin_shell_ui::theme::is_dark();
    let mode = if dark {
        TokenMode::Dark
    } else {
        TokenMode::Light
    };
    let chrome = gnoblin_shell_ui::theme::shell_chrome(dark);
    let theme = win.global::<Theme>();
    theme.set_mode(mode);
    gnoblin_shell_ui::apply_shell_chrome_to_theme!(theme, chrome);
}

impl BarApp for OsdApp {
    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        let win =
            Osd::new().map_err(|e| gnoblin_shell_ui::runtime_error(format!("Osd::new: {e}")))?;
        apply_theme(&win);
        gnoblin_shell_ui::apply_shell_motion_to_theme!(
            win.global::<Theme>(),
            gnoblin_shell_ui::prefs::shell_motion()
        );
        win.set_level(self.level);
        win.set_is_brightness(self.is_brightness);
        win.set_muted(self.muted);
        win.show()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("osd.show: {e}")))?;
        self.win = Some(win);
        self.deadline = Some(Instant::now() + SHOW);
        Ok(())
    }

    fn tick(&mut self) -> bool {
        false
    }

    fn window(&self) -> Option<&slint::Window> {
        self.win.as_ref().map(|w| w.window())
    }

    fn should_exit(&self) -> bool {
        self.deadline.map(|d| Instant::now() >= d).unwrap_or(false)
    }
}

/// Volume from wpctl: (level 0..1, muted).
fn read_volume() -> Option<(f32, bool)> {
    let out = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let muted = text.contains("MUTED");
    let level = text.split_whitespace().nth(1)?.parse::<f32>().ok()?;
    Some((level, muted))
}

/// Backlight brightness percentage via brightnessctl.
fn brightness_percent() -> Option<f32> {
    let out = std::process::Command::new("brightnessctl")
        .args(["-c", "backlight", "-m"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let field = text.trim().split(',').nth(3)?;
    field
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .map(|p| p / 100.0)
}

fn main() {
    let kind = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "volume".to_string());
    let is_brightness = kind == "brightness";
    let (mut level, muted) = if is_brightness {
        (brightness_percent().unwrap_or(0.0), false)
    } else {
        read_volume().unwrap_or((0.0, false))
    };
    // Test override (no audio/backlight in headless validation).
    if let Some(l) = std::env::var("GNOBLIN_OSD_LEVEL")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        level = l;
    }
    let muted = muted || std::env::var("GNOBLIN_OSD_MUTED").is_ok();

    run(
        BarConfig {
            namespace: "gnoblin-osd",
            anchor: Anchor::TOP
                .union(Anchor::BOTTOM)
                .union(Anchor::LEFT)
                .union(Anchor::RIGHT),
            layer: Layer::Overlay,
            height: 1,
            exclusive_zone: 0,
            full_height: true,
            input_passthrough: true,
            keyboard: false,
        },
        Box::new(OsdApp {
            win: None,
            is_brightness,
            level: level.min(1.0),
            muted,
            deadline: None,
        }),
    );
}
