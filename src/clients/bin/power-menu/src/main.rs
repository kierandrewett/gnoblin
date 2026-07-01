//! gnoblin-power-menu — a Slint wlr-layer-shell modal offering Lock / Log Out /
//! Suspend / Restart / Shut Down. Spawned on demand (e.g. the control-centre
//! power button). Picks run the matching system command (logind/systemd) and the
//! surface exits. The compositor owns the centered popup chrome and modal grab.

use gnoblin_core::RuntimeError;
use gnoblin_runtime::{run, shell, BarApp, BarConfig};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::Cell;
use std::process::{Command, Stdio};
use std::rc::Rc;

slint::include_modules!(); // PowerMenu, MenuItem

const MENU_W: u32 = 220;

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

fn menu_height() -> u32 {
    12 + ENTRIES.len() as u32 * 30 + ENTRIES.len().saturating_sub(1) as u32 * 2
}

struct PowerMenuApp {
    menu: Option<PowerMenu>,
    exit: Rc<Cell<bool>>,
}

fn apply_theme(menu: &PowerMenu) {
    gnoblin_runtime::apply_shell_theme!(menu);
}

impl BarApp for PowerMenuApp {
    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
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

    fn tick(&mut self) -> bool {
        false
    }

    fn window(&self) -> Option<&slint::Window> {
        self.menu.as_ref().map(|m| m.window())
    }

    fn should_exit(&self) -> bool {
        self.exit.get()
    }
}

fn main() {
    run(
        BarConfig {
            namespace: "gnoblin-power-menu",
            anchor: Anchor::empty(),
            layer: Layer::Overlay,
            width: MENU_W,
            height: menu_height(),
            exclusive_zone: 0,
            full_height: false,
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
