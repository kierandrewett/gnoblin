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

// These MUST track the Spotlight geometry in launcher.slint (row-h 52, search-h
// 60, panel-h 432 list / 560 grid) so keyboard nav scrolls the selected row into
// view. List viewport = panel-h(432) - search-h(60) - hairline(1) - top pad(4).
const ROW_H: i32 = 52;
const VISIBLE_H: i32 = 367;
const COLUMNS: usize = 5; // app-grid columns (must match the .slint default)
const CELL_H: i32 = 112; // app-grid tile height (must match the .slint cell-h)
const GRID_VISIBLE_H: i32 = 499; // grid viewport (panel 560 - search 60 - hairline 1)

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

/// Put `text` on the Wayland clipboard (used by the calculator's "Enter to
/// copy"). Best-effort: if `wl-copy` isn't present, nothing happens.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}

/// What activating a result does. Apps launch; computed results (calculator)
/// copy their payload; provider results (file search, web, custom) run a shell
/// command via `Run`.
enum Action {
    Launch(String),
    Copy(String),
    /// Run a shell command (a provider result's action), then close.
    Run(String),
}

/// One result row — an app, a calculator answer, or (later) a provider hit.
struct Row {
    name: String,
    subtitle: String,
    /// Icon name for `kind == "app"` (resolved via find_icon); empty otherwise.
    icon: String,
    /// "app" | "calc" — lets the view pick a built-in glyph + styling.
    kind: String,
    /// Right-aligned accessory text, e.g. a calculator result like "= 4".
    accessory: String,
    action: Action,
}

struct LauncherApp {
    win: Option<Launcher>,
    all: Vec<App>,
    filtered: Rc<RefCell<Vec<Row>>>,
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

/// Minimal URL query encoder (RFC 3986 unreserved chars pass through).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Single-quote a string for safe use inside `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Activate the row at `index`: launch an app (recording usage) or copy a
/// computed answer. Shared by the click closure and the Enter key path.
fn activate_row(
    rows: &Rc<RefCell<Vec<Row>>>,
    index: usize,
    usage: &Rc<RefCell<HashMap<String, u32>>>,
) {
    if let Some(row) = rows.borrow().get(index) {
        match &row.action {
            Action::Launch(id) => launch_app(id, usage),
            Action::Copy(text) => copy_to_clipboard(text),
            Action::Run(cmd) => {
                use std::process::{Command, Stdio};
                let _ = Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }
}

fn apply_theme(win: &Launcher) {
    gnoblin_shell_ui::apply_shell_theme!(win);
}

impl LauncherApp {
    fn rebuild(&self) {
        if let Some(win) = &self.win {
            let model: Vec<AppEntry> = self
                .filtered
                .borrow()
                .iter()
                .map(|r| {
                    // Apps + provider rows carry a (theme) icon name; calc uses a
                    // built-in glyph resolved in the view.
                    let icon = if r.icon.is_empty() {
                        None
                    } else {
                        find_icon(&r.icon, "")
                    };
                    AppEntry {
                        name: r.name.clone().into(),
                        subtitle: r.subtitle.clone().into(),
                        has_icon: icon.is_some(),
                        icon: icon.unwrap_or_default(),
                        kind: r.kind.clone().into(),
                        accessory: r.accessory.clone().into(),
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
        let mut rows: Vec<Row> = Vec::new();

        // Calculator: a computed answer on top when the query is arithmetic
        // (search mode only — the app grid is just apps). Enter copies it.
        if !self.grid {
            if let Some(ans) = calc::try_eval(&self.query) {
                rows.push(Row {
                    name: ans.clone(),
                    subtitle: "Calculator — press ⏎ to copy".into(),
                    icon: String::new(),
                    kind: "calc".into(),
                    accessory: String::new(),
                    action: Action::Copy(ans),
                });
            }
        }

        // Process/command providers — file search, web handoff, convert, etc.
        // Prefix-gated ones only run when their keyword is typed, so app
        // searches never spawn a process. Provider hits sit above the app list.
        if !self.grid {
            for p in &self.providers {
                for r in provider::run(p, &self.query) {
                    let action = if r.action.is_empty() {
                        Action::Copy(r.title.clone())
                    } else {
                        Action::Run(r.action)
                    };
                    rows.push(Row {
                        name: r.title,
                        subtitle: r.subtitle,
                        icon: r.icon,
                        kind: "provider".into(),
                        accessory: String::new(),
                        action,
                    });
                }
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
            rows.push(Row {
                name: a.name.clone(),
                subtitle: "Application".into(),
                icon: a.icon.clone(),
                kind: "app".into(),
                accessory: String::new(),
                action: Action::Launch(a.id.clone()),
            });
        }

        // Web-search fallback (opt-in via `[launcher] web-search = <url %s>`):
        // when nothing else matches, offer to search the web — like Spotlight.
        if !self.grid && rows.is_empty() {
            let q = self.query.trim();
            if !q.is_empty() {
                if let Some(tmpl) = &self.web_search {
                    let enc = urlencode(q);
                    rows.push(Row {
                        name: format!("Search the web for “{q}”"),
                        subtitle: "Open in your browser".into(),
                        icon: "web-browser".into(),
                        kind: "web".into(),
                        accessory: String::new(),
                        action: Action::Run(format!(
                            "xdg-open {}",
                            shell_quote(&tmpl.replace("%s", &enc))
                        )),
                    });
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
        activate_row(&self.filtered, index, &self.usage);
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
            activate_row(&filtered, i as usize, &usage);
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
            // The app grid has no search box, so skip provider spawning there.
            providers: if grid { Vec::new() } else { provider::load() },
            web_search: if grid {
                None
            } else {
                gnoblin_shell_ui::config::Config::load()
                    .get("launcher", "web-search")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            },
        }),
    );
}
