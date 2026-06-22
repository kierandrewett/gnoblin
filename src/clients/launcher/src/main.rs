//! gnoblin-launcher — a Slint wlr-layer-shell application launcher.
//!
//! A centred overlay with an exclusive keyboard grab: type to filter installed
//! apps, ↑/↓ to move, Enter or click to launch, Escape to dismiss. Invoked by
//! the dock's app-grid button and a Super+Space keybind.
//!
//! `--grid` (or GNOBLIN_LAUNCHER_MODE=grid) switches to the app grid: the same
//! apps + usage data shown as a scrollable icon grid (GNOME "show apps"),
//! navigated in 2-D with the arrow keys. Bound to Super+A.

mod desktop;
mod usage;

use desktop::App;
use gnoblin_shell_ui::{find_icon, run, BarApp, BarConfig, ClientArgs, RuntimeError};
slint::include_modules!(); // Launcher, AppEntry
use slint::platform::Key;
use slint::{ComponentHandle, SharedString};
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const ROW_H: i32 = 44;
const VISIBLE_H: i32 = 356; // list viewport height (panel 440 - pad 16 - search 40 - spacing 12 - pad 16)
const COLUMNS: usize = 5; // app-grid columns (must match the .slint default)
const CELL_H: i32 = 112; // app-grid tile height (must match the .slint cell-h)
const GRID_VISIBLE_H: i32 = 476; // grid viewport (panel 560 - pad 16 - search 40 - spacing 12 - pad 16)

/// Bump `id`'s launch count, persist, then run it. Shared by the click + Enter
/// paths so usage (which drives the most-used-first sort) is always recorded.
fn launch_app(id: &str, usage: &Rc<RefCell<HashMap<String, u32>>>) {
    {
        let mut u = usage.borrow_mut();
        *u.entry(id.to_string()).or_insert(0) += 1;
        usage::save(&u);
    }
    gnoblin_shell_ui::launch_desktop_app(id);
}

struct LauncherApp {
    win: Option<Launcher>,
    all: Vec<App>,
    filtered: Rc<RefCell<Vec<App>>>,
    query: String,
    selected: usize,
    grid: bool,
    exit: Rc<Cell<bool>>,
    usage: Rc<RefCell<HashMap<String, u32>>>,
}

fn apply_theme(win: &Launcher) {
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

impl LauncherApp {
    fn rebuild(&self) {
        if let Some(win) = &self.win {
            let model: Vec<AppEntry> = self
                .filtered
                .borrow()
                .iter()
                .map(|a| {
                    let icon = find_icon(&a.icon, "");
                    AppEntry {
                        name: a.name.clone().into(),
                        has_icon: icon.is_some(),
                        icon: icon.unwrap_or_default(),
                    }
                })
                .collect();
            win.set_apps(Rc::new(slint::VecModel::from(model)).into());
            win.set_query(self.query.clone().into());
            self.sync_selection();
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let mut matched: Vec<App> = self
            .all
            .iter()
            .filter(|a| q.is_empty() || a.search.contains(&q))
            .cloned()
            .collect();
        // Most-used first (so the empty query surfaces frequent apps), then A→Z.
        let usage = self.usage.borrow();
        matched.sort_by(|a, b| {
            let ua = usage.get(&a.id).copied().unwrap_or(0);
            let ub = usage.get(&b.id).copied().unwrap_or(0);
            ub.cmp(&ua)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        drop(usage);
        matched.truncate(300);
        *self.filtered.borrow_mut() = matched;
        self.selected = 0;
        self.rebuild();
    }

    fn sync_selection(&self) {
        if let Some(win) = &self.win {
            win.set_selected(self.selected as i32);
            // Scroll so the selected row stays in view; grid rows pack COLUMNS
            // tiles each, the list one app per row.
            let (sel_top, step, visible) = if self.grid {
                (
                    (self.selected / COLUMNS) as i32 * CELL_H,
                    CELL_H,
                    GRID_VISIBLE_H,
                )
            } else {
                (self.selected as i32 * ROW_H, ROW_H, VISIBLE_H)
            };
            let scroll = -((sel_top + step - visible).max(0)) as f32;
            win.set_scroll_y(scroll);
        }
    }

    fn launch(&self, index: usize) {
        if let Some(app) = self.filtered.borrow().get(index) {
            launch_app(&app.id, &self.usage);
        }
    }
}

impl BarApp for LauncherApp {
    fn show(
        &mut self,
        _w: u32,
        _h: u32,
        _screen_w: u32,
        _screen_h: u32,
    ) -> Result<(), RuntimeError> {
        let win = Launcher::new()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("Launcher::new: {e}")))?;
        apply_theme(&win);
        gnoblin_shell_ui::apply_shell_motion_to_theme!(
            win.global::<Theme>(),
            gnoblin_shell_ui::prefs::shell_motion()
        );
        win.set_grid_mode(self.grid);
        win.set_columns(COLUMNS as i32);
        let exit = self.exit.clone();
        let filtered = self.filtered.clone();
        let usage = self.usage.clone();
        win.on_activated(move |i| {
            if let Some(app) = filtered.borrow().get(i as usize) {
                launch_app(&app.id, &usage);
            }
            exit.set(true);
        });
        win.show()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("launcher.show: {e}")))?;
        self.win = Some(win);
        self.refilter();
        Ok(())
    }

    fn tick(&mut self) -> bool {
        false
    }

    fn window(&self) -> Option<&slint::Window> {
        self.win.as_ref().map(|w| w.window())
    }

    fn input_full(&self) -> bool {
        true
    }

    fn should_exit(&self) -> bool {
        self.exit.get()
    }

    fn key_pressed(&mut self, text: &SharedString) {
        let c = text.chars().next();
        let count = self.filtered.borrow().len();
        if c == Some(char::from(Key::Escape)) {
            self.exit.set(true);
        } else if c == Some(char::from(Key::Return)) {
            if count > 0 {
                self.launch(self.selected);
            }
            self.exit.set(true);
        } else if c == Some(char::from(Key::Backspace)) {
            self.query.pop();
            self.refilter();
        } else if c == Some(char::from(Key::UpArrow)) {
            // In the grid, Up/Down step a whole row (COLUMNS apps); in the list
            // they step one. Left/Right step one only in the grid.
            let step = if self.grid { COLUMNS } else { 1 };
            self.selected = self.selected.saturating_sub(step);
            self.sync_selection();
        } else if c == Some(char::from(Key::DownArrow)) {
            let step = if self.grid { COLUMNS } else { 1 };
            if count > 0 {
                self.selected = (self.selected + step).min(count - 1);
            }
            self.sync_selection();
        } else if self.grid && c == Some(char::from(Key::LeftArrow)) {
            self.selected = self.selected.saturating_sub(1);
            self.sync_selection();
        } else if self.grid && c == Some(char::from(Key::RightArrow)) {
            if count > 0 {
                self.selected = (self.selected + 1).min(count - 1);
            }
            self.sync_selection();
        } else {
            // Printable text — skip control + Slint private-use special chars.
            let mut typed = false;
            for ch in text.chars() {
                if !ch.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&ch) {
                    self.query.push(ch);
                    typed = true;
                }
            }
            if typed {
                self.refilter();
            }
        }
    }
}

fn main() {
    let _ = ClientArgs::from_env();
    // App-grid mode: `gnoblin-launcher --grid` (or GNOBLIN_LAUNCHER_MODE=grid),
    // bound to Super+A / a dock button. Same scan + usage data, grid layout.
    let grid = std::env::args().any(|a| a == "--grid")
        || std::env::var("GNOBLIN_LAUNCHER_MODE").as_deref() == Ok("grid");
    run(
        BarConfig {
            namespace: "gnoblin-launcher",
            anchor: Anchor::TOP
                .union(Anchor::BOTTOM)
                .union(Anchor::LEFT)
                .union(Anchor::RIGHT),
            layer: Layer::Overlay,
            height: 1,
            exclusive_zone: 0,
            full_height: true,
            input_passthrough: false,
            keyboard: true,
        },
        Box::new(LauncherApp {
            win: None,
            all: desktop::scan(),
            filtered: Rc::new(RefCell::new(Vec::new())),
            // Test hook: pre-fill the query (headless validation can't type).
            query: std::env::var("GNOBLIN_LAUNCHER_QUERY").unwrap_or_default(),
            selected: 0,
            grid,
            exit: Rc::new(Cell::new(false)),
            usage: Rc::new(RefCell::new(usage::load())),
        }),
    );
}
