//! gnoblin-launcher — a Slint wlr-layer-shell application launcher.
//!
//! A centred overlay with an exclusive keyboard grab: type to filter installed
//! apps, ↑/↓ to move, Enter or click to launch, Escape to dismiss. Invoked by
//! the dock's app-grid button and a Super+Space keybind.
//!
//! `--grid` (or GNOBLIN_LAUNCHER_MODE=grid) switches to the app grid: the same
//! apps + usage data shown as a scrollable icon grid (GNOME "show apps"),
//! navigated in 2-D with the arrow keys. Bound to Super+A.

mod calc;
mod desktop;
mod provider;
mod results;
mod usage;

use desktop::App;
use gnoblin_core::{ClientArgs, RuntimeError};
use gnoblin_runtime::{run, BarApp, BarConfig, BarMargins};
slint::include_modules!(); // Launcher, AppEntry
use slint::platform::Key;
use slint::{ComponentHandle, SharedString};
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

// These MUST track the launcher geometry in launcher.slint (row-h 52, search-h
// 60, panel-h 432 list / 560 grid) so keyboard nav scrolls the selected row into
// view. List viewport = panel-h(432) - search-h(60) - hairline(1) - top pad(4).
const ROW_H: i32 = 52;
const VISIBLE_H: i32 = 367;
const LIST_PANEL_W: u32 = 600;
const LIST_PANEL_H: u32 = 432;
const GRID_PANEL_W: u32 = 680;
const GRID_PANEL_H: u32 = 560;
const LIST_TOP_MARGIN: i32 = 128;
const COLUMNS: usize = 5; // app-grid columns (must match the .slint default)
const CELL_H: i32 = 112; // app-grid tile height (must match the .slint cell-h)
const GRID_VISIBLE_H: i32 = 499; // grid viewport (panel 560 - search 60 - hairline 1)

struct LauncherApp {
    win: Option<Launcher>,
    all: Vec<App>,
    filtered: Rc<RefCell<Vec<results::Row>>>,
    query: String,
    selected: usize,
    grid: bool,
    exit: Rc<Cell<bool>>,
    usage: Rc<RefCell<HashMap<String, u32>>>,
    /// Process/command search sources (file search, web, convert, …).
    providers: Vec<provider::Provider>,
    /// Optional `[launcher] web-search` URL template (with `%s`) — when set, a
    /// "Search the web" fallback appears if nothing else matches.
    web_search: Option<String>,
}

fn apply_theme(win: &Launcher) {
    gnoblin_runtime::apply_shell_theme!(win);
}

impl LauncherApp {
    fn rebuild(&self) {
        if let Some(win) = &self.win {
            let model: Vec<AppEntry> = self
                .filtered
                .borrow()
                .iter()
                .map(results::Row::entry)
                .collect();
            win.set_apps(Rc::new(slint::VecModel::from(model)).into());
            win.set_query(self.query.clone().into());
            self.sync_selection();
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let mut rows: Vec<results::Row> = Vec::new();

        // Calculator: a computed answer on top when the query is arithmetic
        // (search mode only — the app grid is just apps). Enter copies it.
        if !self.grid {
            if let Some(ans) = calc::try_eval(&self.query) {
                rows.push(results::Row::calculator(ans));
            }
        }

        // Process/command providers — file search, web handoff, convert, etc.
        // Prefix-gated ones only run when their keyword is typed, so app
        // searches never spawn a process. Provider hits sit above the app list.
        if !self.grid {
            for p in &self.providers {
                rows.extend(
                    provider::run(p, &self.query)
                        .into_iter()
                        .map(results::Row::provider),
                );
            }
        }

        // Apps — most-used first (so the empty query surfaces frequent apps),
        // then A→Z.
        let mut matched: Vec<&App> = self
            .all
            .iter()
            .filter(|a| q.is_empty() || a.search.contains(&q))
            .collect();
        let usage = self.usage.borrow();
        matched.sort_by(|a, b| {
            let ua = usage.get(&a.id).copied().unwrap_or(0);
            let ub = usage.get(&b.id).copied().unwrap_or(0);
            ub.cmp(&ua)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        drop(usage);
        for a in matched.into_iter().take(300) {
            rows.push(results::Row::app(a));
        }

        // Web-search fallback (opt-in via `[launcher] web-search = <url %s>`):
        // when nothing else matches, offer to search the web (search-engine fallback).
        if !self.grid && rows.is_empty() {
            let q = self.query.trim();
            if !q.is_empty() {
                if let Some(tmpl) = &self.web_search {
                    rows.push(results::Row::web_search(q, tmpl));
                }
            }
        }

        *self.filtered.borrow_mut() = rows;
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
        let rows = self.filtered.borrow();
        results::activate(rows.as_slice(), index, &self.usage);
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
            .map_err(|e| gnoblin_core::runtime_error(format!("Launcher::new: {e}")))?;
        apply_theme(&win);
        gnoblin_runtime::apply_shell_motion_to_theme!(
            win.global::<Theme>(),
            gnoblin_runtime::prefs::shell_motion()
        );
        win.set_grid_mode(self.grid);
        win.set_columns(COLUMNS as i32);
        let exit = self.exit.clone();
        let filtered = self.filtered.clone();
        let usage = self.usage.clone();
        win.on_activated(move |i| {
            let rows = filtered.borrow();
            results::activate(rows.as_slice(), i as usize, &usage);
            exit.set(true);
        });
        win.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("launcher.show: {e}")))?;
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

    fn should_exit(&self) -> bool {
        self.exit.get()
    }

    fn key_pressed(&mut self, text: &SharedString) {
        let c = text.chars().next();
        let count = self.filtered.borrow().len();
        // The pointer can move the selection (hover-to-select); read it back so
        // arrow keys + Enter continue from the row under the cursor, not a stale
        // index.
        if let Some(w) = &self.win {
            self.selected = (w.get_selected().max(0) as usize).min(count.saturating_sub(1));
        }
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
    let (width, height, anchor, margins) = if grid {
        (
            GRID_PANEL_W,
            GRID_PANEL_H,
            Anchor::empty(),
            BarMargins::default(),
        )
    } else {
        (
            LIST_PANEL_W,
            LIST_PANEL_H,
            Anchor::TOP,
            BarMargins {
                top: LIST_TOP_MARGIN,
                ..BarMargins::default()
            },
        )
    };
    run(
        BarConfig {
            namespace: "gnoblin-launcher",
            anchor,
            layer: Layer::Overlay,
            width,
            height,
            margins,
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: false,
            keyboard: true,
            ..BarConfig::default()
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
            // The app grid has no search box, so skip provider spawning there.
            providers: if grid { Vec::new() } else { provider::load() },
            web_search: if grid {
                None
            } else {
                gnoblin_core::config::Config::load()
                    .get("launcher", "web-search")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            },
        }),
    );
}
