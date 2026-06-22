//! gnoblin-window-menu — the WM window menu as an on-demand Slint layer-shell
//! client. The compositor spawns it (the `window-menu` role) with the target
//! window + anchor point on the command line; it renders the [menu] entries as
//! a modal overlay and dispatches the chosen gnoblin action at that window.

use gnoblin_shell_ui::config::Config;
use gnoblin_shell_ui::{run, shell, BarApp, BarConfig, ClientArgs, RuntimeError};
slint::include_modules!(); // WindowMenu, MenuItem
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::Cell;
use std::rc::Rc;

/// One parsed `[menu]` entry: `Label | action [arg]`, or a separator.
#[derive(Clone)]
struct Entry {
    label: String,
    action: String,
    arg: String,
    separator: bool,
}

/// Built-in menu when `[menu]` has no `item` lines (mirrors the verbs the action
/// API / keybindings use).
const DEFAULT_ITEMS: &[&str] = &[
    "Minimise | minimize",
    "Maximise | maximize",
    "Move | move",
    "Resize | resize",
    "-",
    "Snap Left | snap left",
    "Snap Right | snap right",
    "-",
    "Always on Top | always-on-top",
    "Always on Visible Workspace | always-on-visible-workspace",
    "-",
    "Close | close",
];

/// Parse one `Label | action [arg]` (or `-`) line.
fn parse_entry(line: &str) -> Entry {
    let line = line.trim();
    if line == "-" || line.starts_with("separator") {
        return Entry {
            label: String::new(),
            action: String::new(),
            arg: String::new(),
            separator: true,
        };
    }
    let (label, rest) = match line.split_once('|') {
        Some((l, r)) => (l.trim().to_string(), r.trim().to_string()),
        None => (line.to_string(), String::new()),
    };
    let (action, arg) = match rest.split_once(char::is_whitespace) {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (rest, String::new()),
    };
    Entry {
        label,
        action,
        arg,
        separator: false,
    }
}

fn load_entries() -> Vec<Entry> {
    let cfg = Config::load();
    let lines = cfg.get_list("menu", "item");
    let raw: Vec<String> = if lines.is_empty() {
        DEFAULT_ITEMS.iter().map(|s| s.to_string()).collect()
    } else {
        lines
    };
    let mut entries: Vec<Entry> = raw.iter().map(|l| parse_entry(l)).collect();
    // Drop leading/trailing separators (look broken).
    while entries.first().map(|e| e.separator).unwrap_or(false) {
        entries.remove(0);
    }
    while entries.last().map(|e| e.separator).unwrap_or(false) {
        entries.pop();
    }
    entries
}

fn apply_theme(menu: &WindowMenu) {
    let dark = gnoblin_shell_ui::theme::is_dark();
    let mode = if dark {
        TokenMode::Dark
    } else {
        TokenMode::Light
    };
    let theme = menu.global::<Theme>();
    theme.set_mode(mode);
    let chrome = gnoblin_shell_ui::theme::shell_chrome(dark);
    gnoblin_shell_ui::apply_shell_chrome_to_theme!(theme, chrome);
}

struct WindowMenuApp {
    menu: Option<WindowMenu>,
    args: ClientArgs,
    entries: Vec<Entry>,
    exit: Rc<Cell<bool>>,
}

impl BarApp for WindowMenuApp {
    fn show(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        let menu = WindowMenu::new()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("WindowMenu::new: {e}")))?;
        apply_theme(&menu);
        gnoblin_shell_ui::apply_shell_motion_to_theme!(
            menu.global::<Theme>(),
            gnoblin_shell_ui::prefs::shell_motion()
        );

        let model: Vec<MenuItem> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| MenuItem {
                id: i as i32,
                label: e.label.clone().into(),
                accelerator: Default::default(),
                separator: e.separator,
                enabled: !e.separator,
            })
            .collect();
        menu.set_items(Rc::new(slint::VecModel::from(model)).into());

        self.apply_geometry(&menu, screen_w, screen_h);

        if let Some(bg) = gnoblin_shell_ui::load_backdrop() {
            menu.set_backdrop(bg);
        }

        // Pick → dispatch the action at the target window, then exit.
        {
            let exit = self.exit.clone();
            let entries = self.entries.clone();
            let window = self.args.window.unwrap_or(0);
            menu.on_item_activated(move |id| {
                if let Some(e) = entries.get(id as usize) {
                    if !e.separator && !e.action.is_empty() {
                        shell::dispatch_window_action(window, &e.action, &e.arg);
                    }
                }
                exit.set(true);
            });
        }
        {
            let exit = self.exit.clone();
            menu.on_dismiss(move || exit.set(true));
        }

        menu.show()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("window menu show: {e}")))?;
        self.menu = Some(menu);
        Ok(())
    }

    fn resized(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        if let Some(menu) = &self.menu {
            self.apply_geometry(menu, screen_w, screen_h);
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

impl WindowMenuApp {
    fn apply_geometry(&self, menu: &WindowMenu, screen_w: u32, screen_h: u32) {
        // Anchor at the compositor-provided point, clamped so the menu stays
        // on screen (menu is ~220px wide; height grows with the row count).
        let menu_w = 220i32;
        let menu_h = (self.entries.len() as i32) * 30 + 16;
        let x = self
            .args
            .x
            .unwrap_or(0)
            .clamp(0, (screen_w as i32 - menu_w).max(0));
        let y = self
            .args
            .y
            .unwrap_or(0)
            .clamp(0, (screen_h as i32 - menu_h).max(0));
        menu.set_menu_x(x as f32);
        menu.set_menu_y(y as f32);
        menu.set_backdrop_screen_w(screen_w as f32);
        menu.set_backdrop_screen_h(screen_h as f32);
    }
}

fn main() {
    let args = ClientArgs::from_env();
    run(
        BarConfig {
            namespace: "gnoblin-window-menu",
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
        },
        Box::new(WindowMenuApp {
            menu: None,
            args,
            entries: load_entries(),
            exit: Rc::new(Cell::new(false)),
        }),
    );
}
