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
use slint::{ComponentHandle, Model, SharedString};
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
// Icon loading is the launcher's dominant cold-start cost: resolving+decoding
// +resizing one themed icon per row is ~0.7ms, and the empty query lists every
// installed app (~250), so a naive rebuild spends ~175ms on icons ALONE before
// the first frame. Only ~7 rows (list) / ~20 tiles (grid) are ever visible, so
// we resolve the first `EAGER_ICONS` synchronously (they're on-screen) and
// STREAM the rest in on idle `tick()`s — off-screen rows get their icons before
// the user can scroll to them, so there's no visible pop-in.
const EAGER_ICONS: usize = 12; // ~covers the visible list rows; grid streams fast
const ICON_BATCH: usize = 48; // themed icons resolved per idle tick
const COLUMNS: usize = 5; // app-grid columns (must match the .slint default)
const CELL_H: i32 = 112; // app-grid tile height (must match the .slint cell-h)
const GRID_VISIBLE_H: i32 = 499; // grid viewport (panel 560 - search 60 - hairline 1)

struct LauncherApp {
    win: Option<Launcher>,
    all: Vec<App>,
    filtered: Rc<RefCell<Vec<results::Row>>>,
    query: String,
    selected: usize,
    /// Next `filtered` index whose icon still needs streaming in (see
    /// `stream_icons`); starts past the eagerly-loaded visible rows.
    icon_cursor: Cell<usize>,
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
            // Resolve icons only for the rows that are actually on-screen; the
            // rest are built icon-less and streamed in by `stream_icons` on idle
            // ticks. This keeps the (expensive) icon work off the open path.
            let model: Vec<AppEntry> = self
                .filtered
                .borrow()
                .iter()
                .enumerate()
                .map(|(i, r)| r.entry_with_icon(i < EAGER_ICONS))
                .collect();
            self.icon_cursor.set(EAGER_ICONS.min(model.len()));
            win.set_apps(Rc::new(slint::VecModel::from(model)).into());
            win.set_query(self.query.clone().into());
            self.sync_selection();
        }
    }

    /// Resolve the next batch of deferred icons and patch them into the live
    /// model in place (no full rebuild). Returns true if any row changed so the
    /// caller can request a redraw. Cheap once the cursor reaches the end.
    fn stream_icons(&self) -> bool {
        let start = self.icon_cursor.get();
        let win = match &self.win {
            Some(w) => w,
            None => return false,
        };
        let model = win.get_apps();
        let vec = match model.as_any().downcast_ref::<slint::VecModel<AppEntry>>() {
            Some(v) => v,
            None => return false,
        };
        let rows = self.filtered.borrow();
        let end = (start + ICON_BATCH).min(rows.len());
        let mut changed = false;
        for i in start..end {
            let row = &rows[i];
            if !row.has_theme_icon() {
                continue; // built-in glyph row (calc/web) — nothing to resolve
            }
            if let Some(icon) = row.resolve_icon() {
                if let Some(mut entry) = vec.row_data(i) {
                    entry.has_icon = true;
                    entry.icon = icon;
                    vec.set_row_data(i, entry);
                    changed = true;
                }
            }
        }
        self.icon_cursor.set(end);
        changed
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
        gnoblin_runtime::rt_tick("    show(): before Launcher::new()");
        let win = Launcher::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("Launcher::new: {e}")))?;
        gnoblin_runtime::rt_tick("    show(): after Launcher::new() (component instantiated)");
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
        gnoblin_runtime::rt_tick("    show(): after win.show()");
        self.win = Some(win);
        self.refilter();
        gnoblin_runtime::rt_tick("    show(): after refilter()");
        Ok(())
    }

    fn tick(&mut self) -> bool {
        // Stream deferred (off-screen) icons in a few batches per second until
        // the whole list is resolved. Runs after the launcher is already up.
        self.stream_icons()
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
    let _t0 = std::time::Instant::now();
    macro_rules! tick {
        ($label:expr) => {
            if std::env::var("GNOBLIN_TIMING").is_ok() {
                eprintln!("[timing] {:>6.1}ms  {}", _t0.elapsed().as_secs_f64() * 1000.0, $label);
            }
        };
    }
    tick!("main entry (post exec+dynlink)");
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
    let all = desktop::scan();
    tick!("desktop::scan()");
    let usage_v = usage::load();
    tick!("usage::load()");
    let providers_v = if grid { Vec::new() } else { provider::load() };
    tick!("provider::load()");
    let web_search = if grid {
        None
    } else {
        gnoblin_core::config::Config::load()
            .get("launcher", "web-search")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    tick!("config web-search (all sync startup work done)");
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
            all,
            filtered: Rc::new(RefCell::new(Vec::new())),
            // Test hook: pre-fill the query (headless validation can't type).
            query: std::env::var("GNOBLIN_LAUNCHER_QUERY").unwrap_or_default(),
            selected: 0,
            icon_cursor: Cell::new(0),
            grid,
            exit: Rc::new(Cell::new(false)),
            usage: Rc::new(RefCell::new(usage_v)),
            // The app grid has no search box, so skip provider spawning there.
            providers: providers_v,
            web_search,
        }),
    );
}
