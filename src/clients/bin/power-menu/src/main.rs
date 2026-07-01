//! gnoblin-power-menu — a Slint wlr-layer-shell modal offering Lock / Log Out /
//! Suspend / Restart / Shut Down. Spawned on demand (e.g. the control-centre
//! power button). Picks run the matching system command (logind/systemd) and the
//! surface exits. A centred ContextMenu over a dismiss scrim, like the window menu.

use gnoblin_core::RuntimeError;
use gnoblin_runtime::{run, shell, BarApp, BarConfig};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::Cell;
use std::process::{Command, Stdio};
use std::rc::Rc;

slint::include_modules!(); // PowerMenu, MenuItem

/// (label, kind) for each row. `kind` selects the action in `run_power`.
const ENTRIES: &[(&str, &str)] = &[
    ("Lock", "lock"),
    ("Log Out", "logout"),
    ("Suspend", "suspend"),
    ("Restart", "reboot"),
    ("Shut Down", "poweroff"),
];

fn spawn(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Perform a power action. `GNOBLIN_POWER_DRYRUN` prints instead of acting, so
/// the menu can be exercised in tests without suspending/rebooting the machine.
fn run_power(kind: &str) {
    if std::env::var("GNOBLIN_POWER_DRYRUN").is_ok() {
        println!("POWER:{kind}");
        return;
    }
    match kind {
        "lock" => shell::dispatch_window_action(0, "lock", ""),
        "logout" => {
            if let Ok(sid) = std::env::var("XDG_SESSION_ID") {
                spawn("loginctl", &["terminate-session", &sid]);
            }
        }
        "suspend" => spawn("systemctl", &["suspend"]),
        "reboot" => spawn("systemctl", &["reboot"]),
        "poweroff" => spawn("systemctl", &["poweroff"]),
        _ => {}
    }
}

struct PowerMenuApp {
    menu: Option<PowerMenu>,
    exit: Rc<Cell<bool>>,
}

fn apply_theme(menu: &PowerMenu) {
    gnoblin_runtime::apply_shell_theme!(menu);
}

fn apply_geometry(menu: &PowerMenu, screen_w: u32, screen_h: u32) {
    // Centre the menu (220px wide; height grows with the row count).
    let menu_w = 220i32;
    let menu_h = (ENTRIES.len() as i32) * 34 + 16;
    let x = ((screen_w as i32 - menu_w) / 2).max(0);
    let y = ((screen_h as i32 - menu_h) / 2).max(0);
    menu.set_menu_x(x as f32);
    menu.set_menu_y(y as f32);
    menu.set_backdrop_screen_w(screen_w as f32);
    menu.set_backdrop_screen_h(screen_h as f32);
}

impl BarApp for PowerMenuApp {
    fn show(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        let menu = PowerMenu::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("PowerMenu::new: {e}")))?;
        apply_theme(&menu);
        gnoblin_runtime::apply_shell_motion_to_theme!(
            menu.global::<Theme>(),
            gnoblin_runtime::prefs::shell_motion()
        );

        let model: Vec<MenuItem> = ENTRIES
            .iter()
            .enumerate()
            .map(|(i, (label, _))| MenuItem {
                id: i as i32,
                label: (*label).into(),
                accelerator: Default::default(),
                separator: false,
                submenu: false,
                enabled: true,
            })
            .collect();
        menu.set_items(Rc::new(slint::VecModel::from(model)).into());

        apply_geometry(&menu, screen_w, screen_h);

        if let Some(bg) = gnoblin_runtime::load_backdrop() {
            menu.set_backdrop(bg);
        }

        {
            let exit = self.exit.clone();
            menu.on_item_activated(move |id| {
                if let Some((_, kind)) = ENTRIES.get(id as usize) {
                    run_power(kind);
                }
                exit.set(true);
            });
        }
        {
            let exit = self.exit.clone();
            menu.on_dismiss(move || exit.set(true));
        }

        menu.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("power menu show: {e}")))?;

        // Headless validation: GNOBLIN_POWER_AUTO=<id> activates that row.
        if let Ok(Ok(id)) = std::env::var("GNOBLIN_POWER_AUTO").map(|s| s.parse::<i32>()) {
            menu.invoke_item_activated(id);
        }

        self.menu = Some(menu);
        Ok(())
    }

    fn resized(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        if let Some(menu) = &self.menu {
            apply_geometry(menu, screen_w, screen_h);
        }
    }

    fn tick(&mut self) -> bool {
        false
    }

    fn window(&self) -> Option<&slint::Window> {
        self.menu.as_ref().map(|m| m.window())
    }

    // Modal: the whole surface catches input so an outside click dismisses.
    fn input_full(&self) -> bool {
        true
    }

    fn should_exit(&self) -> bool {
        self.exit.get()
    }
}

fn main() {
    run(
        BarConfig {
            namespace: "gnoblin-power-menu",
            anchor: Anchor::TOP
                .union(Anchor::BOTTOM)
                .union(Anchor::LEFT)
                .union(Anchor::RIGHT),
            layer: Layer::Overlay,
            height: 1,
            exclusive_zone: 0,
            full_height: true,
            input_passthrough: false,
            keyboard: false,
            ..BarConfig::default()
        },
        Box::new(PowerMenuApp {
            menu: None,
            exit: Rc::new(Cell::new(false)),
        }),
    );
}
