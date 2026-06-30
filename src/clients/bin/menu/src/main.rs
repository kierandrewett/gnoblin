//! gnoblin-menu - the dock app context menu as a content-sized layer-shell
//! popup. The dock passes app state and an icon anchor; this client renders the
//! shared ContextMenu with compositor-owned chrome and exits after activation or
//! dismissal.

use gnoblin_core::RuntimeError;
use gnoblin_runtime::{app_context_menu, run, BarApp, BarConfig, BarMargins};
use slint::platform::Key;
use slint::{ComponentHandle, SharedString};
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::Cell;
use std::rc::Rc;

slint::include_modules!(); // GnoblinMenu, MenuItem

const MENU_W: u32 = 220;
const EDGE_INSET: i32 = 8;
const DEFAULT_BOTTOM_MARGIN: i32 = 102;
const DOCK_GAP: i32 = 6;

#[derive(Clone, Debug)]
struct MenuArgs {
    app_id: String,
    running: bool,
    pinned: bool,
    anchor_x: i32,
    screen_w: Option<i32>,
    bottom_margin: i32,
}

impl Default for MenuArgs {
    fn default() -> Self {
        Self {
            app_id: String::new(),
            running: false,
            pinned: false,
            anchor_x: MENU_W as i32 / 2,
            screen_w: None,
            bottom_margin: DEFAULT_BOTTOM_MARGIN,
        }
    }
}

impl MenuArgs {
    fn from_env() -> Self {
        let mut args = Self::default();
        let mut it = std::env::args().skip(1).peekable();

        while let Some(tok) = it.next() {
            let (key, inline) = match tok.split_once('=') {
                Some((k, v)) => (k.to_string(), Some(v.to_string())),
                None => (tok, None),
            };
            let mut value = || {
                if inline.is_some() {
                    return inline.clone();
                }
                match it.peek() {
                    Some(next) if !next.starts_with("--") => it.next(),
                    _ => None,
                }
            };

            match key.as_str() {
                "--app-id" => args.app_id = value().unwrap_or_default(),
                "--running" => args.running = value().as_deref().is_some_and(parse_bool),
                "--pinned" => args.pinned = value().as_deref().is_some_and(parse_bool),
                "--anchor-x" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.anchor_x = v;
                    }
                }
                "--screen-width" => args.screen_w = value().and_then(|v| v.parse().ok()),
                "--bottom-margin" => {
                    if let Some(v) = value().and_then(|v| v.parse().ok()) {
                        args.bottom_margin = v;
                    }
                }
                "--band-height" => {
                    if let Some(v) = value().and_then(|v: String| v.parse::<i32>().ok()) {
                        args.bottom_margin = v + DOCK_GAP;
                    }
                }
                _ => {
                    let _ = value();
                }
            }
        }

        args
    }

    fn left_margin(&self) -> i32 {
        let raw = self.anchor_x - MENU_W as i32 / 2;
        let max_left = self
            .screen_w
            .map(|w| (w - MENU_W as i32 - EDGE_INSET).max(EDGE_INSET))
            .unwrap_or(raw.max(EDGE_INSET));
        raw.clamp(EDGE_INSET, max_left)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn menu_height(items: &[MenuItem]) -> u32 {
    let mut total = 12;
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            total += 2;
        }
        total += if item.separator { 9 } else { 30 };
    }
    total.max(1)
}

fn menu_items(app_id: &str, running: bool, pinned: bool) -> Vec<MenuItem> {
    app_context_menu::build(app_id, running, pinned)
        .into_iter()
        .map(|it| MenuItem {
            id: it.id,
            label: it.label.into(),
            accelerator: Default::default(),
            separator: it.separator,
            submenu: it.submenu,
            enabled: it.enabled,
        })
        .collect()
}

fn apply_theme(menu: &GnoblinMenu) {
    gnoblin_runtime::apply_shell_theme!(menu);
}

struct MenuApp {
    menu: Option<GnoblinMenu>,
    app_id: String,
    items: Vec<MenuItem>,
    exit: Rc<Cell<bool>>,
}

impl BarApp for MenuApp {
    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        let menu = GnoblinMenu::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("GnoblinMenu::new: {e}")))?;
        apply_theme(&menu);
        gnoblin_runtime::apply_shell_motion_to_theme!(
            menu.global::<Theme>(),
            gnoblin_runtime::prefs::shell_motion()
        );
        menu.set_items(Rc::new(slint::VecModel::from(self.items.clone())).into());

        {
            let app_id = self.app_id.clone();
            let exit = self.exit.clone();
            menu.on_item_activated(move |id| {
                let _ = app_context_menu::activate(&app_id, id);
                exit.set(true);
            });
        }
        {
            let exit = self.exit.clone();
            menu.on_dismiss(move || exit.set(true));
        }

        menu.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("menu show: {e}")))?;
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

    fn key_pressed(&mut self, text: &SharedString) {
        if text.chars().next() == Some(char::from(Key::Escape)) {
            self.exit.set(true);
        }
    }
}

fn main() {
    let args = MenuArgs::from_env();
    if args.app_id.is_empty() {
        eprintln!("gnoblin-menu: missing --app-id");
        return;
    }

    let items = menu_items(&args.app_id, args.running, args.pinned);
    let height = menu_height(&items);
    run(
        BarConfig {
            namespace: "gnoblin-menu",
            anchor: Anchor::BOTTOM.union(Anchor::LEFT),
            layer: Layer::Overlay,
            width: MENU_W,
            height,
            margins: BarMargins {
                left: args.left_margin(),
                bottom: args.bottom_margin.max(0),
                ..BarMargins::default()
            },
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: false,
            keyboard: true,
        },
        Box::new(MenuApp {
            menu: None,
            app_id: args.app_id,
            items,
            exit: Rc::new(Cell::new(false)),
        }),
    );
}
