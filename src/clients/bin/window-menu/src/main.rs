//! gnoblin-window-menu — the WM window menu as an on-demand Slint layer-shell
//! client. The compositor spawns it (the `window-menu` role) with the target
//! window + anchor point on the command line; it renders the [menu] entries as
//! a content-sized compositor-chromed popup and dispatches the chosen gnoblin
//! action at that window.

use gnoblin_core::config::Config;
use gnoblin_core::{ClientArgs, RuntimeError};
use gnoblin_runtime::{run, shell, BarApp, BarConfig, BarMargins};
slint::include_modules!(); // WindowMenu, MenuItem
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::Cell;
use std::rc::Rc;

const MENU_W: u32 = 220;

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

fn menu_height(entries: &[Entry]) -> u32 {
    let mut total = 12;
    for (idx, entry) in entries.iter().enumerate() {
        if idx > 0 {
            total += 2;
        }
        total += if entry.separator { 9 } else { 30 };
    }
    total.max(1)
}

fn apply_theme(menu: &WindowMenu) {
    gnoblin_runtime::apply_shell_theme!(menu);
}

struct WindowMenuApp {
    menu: Option<WindowMenu>,
    args: ClientArgs,
    entries: Vec<Entry>,
    exit: Rc<Cell<bool>>,
}

impl BarApp for WindowMenuApp {
    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        let menu = WindowMenu::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("WindowMenu::new: {e}")))?;
        apply_theme(&menu);
        gnoblin_runtime::apply_shell_motion_to_theme!(
            menu.global::<Theme>(),
            gnoblin_runtime::prefs::shell_motion()
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
                submenu: false,
                enabled: !e.separator,
            })
            .collect();
        menu.set_items(Rc::new(slint::VecModel::from(model)).into());

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
            .map_err(|e| gnoblin_core::runtime_error(format!("window menu show: {e}")))?;
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
    let args = ClientArgs::from_env();
    let entries = load_entries();
    let x = args.x.unwrap_or(0).max(0);
    let y = args.y.unwrap_or(0).max(0);
    run(
        BarConfig {
            namespace: "gnoblin-window-menu",
            anchor: Anchor::TOP.union(Anchor::LEFT),
            layer: Layer::Overlay,
            width: MENU_W,
            height: menu_height(&entries),
            margins: BarMargins {
                top: y,
                left: x,
                ..BarMargins::default()
            },
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: false,
            keyboard: false,
            ..BarConfig::default()
        },
        Box::new(WindowMenuApp {
            menu: None,
            args,
            entries,
            exit: Rc::new(Cell::new(false)),
        }),
    );
}
