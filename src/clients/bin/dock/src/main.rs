//! gnoblin-dock — de's Dock as a bottom wlr-layer-shell client.
//!
//! The surface is taller than the visible dock band (BAND_H) by HEADROOM, so a
//! right-click context menu renders above an icon inside our own surface. The
//! band is bottom-anchored; the headroom is click-through — `input_rects()`
//! restricts pointer input to the band (+ the open menu).
use gnoblin_core::config::Config;
use gnoblin_core::{file_mtime, RuntimeError};
use gnoblin_runtime::app_context_menu;
use gnoblin_runtime::shell::{self, WindowState};
use gnoblin_runtime::{run, BarApp, BarConfig};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

// This client's own Slint UI (DockBar, DockItem, MenuItem, Theme, TokenMode).
slint::include_modules!();

/// Visible dock band height (pill + outer padding) — matches Tokens.dock-height.
const BAND_H: f32 = 96.0;
/// Logical app icon size in Dock.slint/Tokens.slint.
const DOCK_ICON_SIZE: u32 = 48;
/// Click-through headroom above the band, room for the right-click menu.
const HEADROOM: i32 = 360;
/// Total surface height = band + headroom.
const SURFACE_H: f32 = BAND_H + HEADROOM as f32;

type MenuRect = (i32, i32, i32, i32);
type SharedMenuGeom = Rc<Cell<Option<MenuRect>>>;

struct DockApp {
    dock: Option<DockBar>,
    // Pinned apps with their (cached) icons, in order.
    favs: Vec<(String, slint::Image)>,
    win_rx: Receiver<WindowState>,
    state: WindowState,
    theme_dark: bool,
    test_clicked: bool, // GNOBLIN_DOCK_CLICK one-shot guard
    // Shared with the Slint callbacks (which are 'static closures):
    surface_w: Rc<Cell<u32>>,
    running: Rc<RefCell<Vec<String>>>,
    menu_geom: SharedMenuGeom,
    menu_target: Rc<RefCell<String>>,
    // Current favourite ids (for the menu's pinned check) + a flag the menu sets
    // to ask tick() to reload favourites after a pin/unpin.
    fav_ids: Rc<RefCell<Vec<String>>>,
    needs_reload: Rc<Cell<bool>>,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
    pins_path: Option<PathBuf>,
    pins_mtime: Option<SystemTime>,
    screen_w: u32,
    screen_h: u32,
    test_pinned: bool,   // GNOBLIN_DOCK_PIN one-shot guard
    test_launched: bool, // GNOBLIN_DOCK_LAUNCH one-shot guard
}

/// Apply the current light/dark preference to the Slint theme global.
fn apply_theme(d: &DockBar) {
    gnoblin_runtime::apply_shell_theme!(d);
}

fn apply_shell_chrome(d: &DockBar) {
    apply_shell_chrome_with(d, gnoblin_runtime::theme::is_dark());
}

fn apply_shell_chrome_with(d: &DockBar, dark: bool) {
    let chrome = gnoblin_runtime::theme::shell_chrome(dark);
    let theme = d.global::<Theme>();
    gnoblin_runtime::apply_shell_chrome_to_theme!(theme, chrome);
}

fn apply_shell_motion(d: &DockBar) -> bool {
    gnoblin_runtime::apply_shell_motion!(d)
}

fn apply_backdrop(d: &DockBar, screen_w: u32, screen_h: u32) {
    d.set_backdrop(gnoblin_runtime::load_backdrop().unwrap_or_default());
    d.set_backdrop_screen_w(screen_w as f32);
    d.set_backdrop_screen_h(screen_h as f32);
    d.set_backdrop_offset_y(-((screen_h as f32) - BAND_H));
}

/// Pixel height the ContextMenu will self-size to: 34 px comfortable rows,
/// ~12 px separators, plus the 16 px container inset (mirrors ContextMenu.slint).
fn menu_height(items: &[MenuItem]) -> f32 {
    let normal = items.iter().filter(|i| !i.separator).count() as f32;
    let seps = items.iter().filter(|i| i.separator).count() as f32;
    normal * 34.0 + seps * 12.0 + 16.0
}

/// Open the menu for `app_id` anchored under `anchor_x` (window-space slot
/// centre). Sets the Slint props and records the menu rect for `input_rects()`.
#[allow(clippy::too_many_arguments)]
fn open_menu(
    d: &DockBar,
    surface_w: u32,
    menu_geom: &Cell<Option<MenuRect>>,
    menu_target: &RefCell<String>,
    app_id: &str,
    is_running: bool,
    is_pinned: bool,
    anchor_x: f32,
) {
    let items: Vec<MenuItem> = app_context_menu::build(app_id, is_running, is_pinned)
        .into_iter()
        .map(|it| MenuItem {
            id: it.id,
            label: it.label.into(),
            accelerator: "".into(),
            separator: it.separator,
            enabled: it.enabled,
        })
        .collect();
    let mh = menu_height(&items);
    let w = 220.0_f32;
    let x = (anchor_x - w / 2.0).clamp(8.0, (surface_w as f32 - w - 8.0).max(8.0));
    let y = (SURFACE_H - BAND_H - mh - 6.0).max(8.0);
    menu_geom.set(Some((x as i32, y as i32, w as i32, mh.ceil() as i32)));
    *menu_target.borrow_mut() = app_id.to_string();
    d.set_menu_items(Rc::new(slint::VecModel::from(items)).into());
    d.set_menu_anchor_x(anchor_x);
    d.set_menu_open(true);
}

impl DockApp {
    /// Reload favourites after a pin/unpin (the menu closure sets `needs_reload`).
    fn reload_favs(&mut self) {
        self.favs = default_favourites();
        *self.fav_ids.borrow_mut() = app_context_menu::favorite_ids_from_config();
        self.config_mtime = file_mtime(self.config_path.as_deref());
        self.pins_mtime = file_mtime(self.pins_path.as_deref());
        self.rebuild();
    }

    fn rebuild(&self) {
        // Snapshot running apps for the right-click closure.
        *self.running.borrow_mut() = self.state.running.clone();
        if let Some(dock) = &self.dock {
            // Pinned favourites first.
            let mut items: Vec<DockItem> = self
                .favs
                .iter()
                .map(|(id, icon)| {
                    let window_count = self.window_count_for(id);
                    let running = window_count > 0;
                    let focused = shell::matches(id, &self.state.focused);
                    DockItem {
                        icon: icon.clone(),
                        app_id: id.clone().into(),
                        name: gnoblin_core::prettify_app(id).into(),
                        running,
                        focused,
                        window_count: window_count as i32,
                        pinned: true,
                    }
                })
                .collect();

            // Then running apps that aren't pinned — a separator divides them
            // (Dock.slint draws it between the last pinned and first unpinned).
            for app in &self.state.running {
                if app.is_empty() || self.favs.iter().any(|(id, _)| shell::matches(id, app)) {
                    continue;
                }
                let Some(icon) = gnoblin_desktop::find_icon_at_size(app, "", DOCK_ICON_SIZE) else {
                    continue;
                };
                let focused = shell::matches(app, &self.state.focused);
                let window_count = self.window_count_for(app).max(1);
                items.push(DockItem {
                    icon,
                    app_id: app.clone().into(),
                    name: gnoblin_core::prettify_app(app).into(),
                    running: true,
                    focused,
                    window_count: window_count as i32,
                    pinned: false,
                });
            }

            dock.set_items(Rc::new(slint::VecModel::from(items)).into());
        }
    }

    fn window_count_for(&self, app_id: &str) -> u32 {
        self.state
            .window_counts
            .iter()
            .filter(|(id, _)| shell::matches(app_id, id))
            .map(|(_, count)| *count)
            .sum()
    }
}

impl BarApp for DockApp {
    fn show(&mut self, w: u32, h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        self.surface_w.set(if w > 0 { w } else { screen_w });
        let dock = DockBar::new()
            .map_err(|e| gnoblin_core::runtime_error(format!("DockBar::new: {e}")))?;

        // Left-click: focus the app if it's already running, else launch it.
        // Also dismiss any open menu.
        {
            let weak = dock.as_weak();
            let geom = self.menu_geom.clone();
            let running = self.running.clone();
            dock.on_icon_clicked(move |app_id| {
                if let Some(d) = weak.upgrade() {
                    d.set_menu_open(false);
                }
                geom.set(None);
                let is_running = running
                    .borrow()
                    .iter()
                    .any(|w| shell::matches(app_id.as_str(), w));
                if is_running {
                    shell::activate_app(app_id.as_str());
                } else {
                    launch(app_id.as_str());
                }
            });
        }
        // Right-click opens the Open/Close menu under the icon.
        {
            let weak = dock.as_weak();
            let running = self.running.clone();
            let sw = self.surface_w.clone();
            let geom = self.menu_geom.clone();
            let target = self.menu_target.clone();
            let fav_ids = self.fav_ids.clone();
            dock.on_icon_right_clicked(move |app_id, anchor_x| {
                let Some(d) = weak.upgrade() else { return };
                let is_running = running
                    .borrow()
                    .iter()
                    .any(|wname| shell::matches(app_id.as_str(), wname));
                let is_pinned = fav_ids
                    .borrow()
                    .iter()
                    .any(|f| shell::matches(f, app_id.as_str()));
                open_menu(
                    &d,
                    sw.get(),
                    &geom,
                    &target,
                    app_id.as_str(),
                    is_running,
                    is_pinned,
                    anchor_x,
                );
            });
        }
        // Menu item chosen → act, then close.
        {
            let weak = dock.as_weak();
            let geom = self.menu_geom.clone();
            let target = self.menu_target.clone();
            let needs_reload = self.needs_reload.clone();
            dock.on_menu_item_clicked(move |id| {
                let app = target.borrow().clone();
                if app_context_menu::activate(&app, id) {
                    needs_reload.set(true); // tick() reloads favourites
                }
                if let Some(d) = weak.upgrade() {
                    d.set_menu_open(false);
                }
                geom.set(None);
            });
        }
        // Outside-click dismiss.
        {
            let weak = dock.as_weak();
            let geom = self.menu_geom.clone();
            dock.on_menu_dismiss(move || {
                if let Some(d) = weak.upgrade() {
                    d.set_menu_open(false);
                }
                geom.set(None);
            });
        }

        self.screen_w = screen_w;
        self.screen_h = screen_h;
        apply_backdrop(&dock, screen_w, screen_h);
        apply_theme(&dock);
        apply_shell_motion(&dock);
        self.theme_dark = gnoblin_runtime::theme::is_dark();
        dock.show()
            .map_err(|e| gnoblin_core::runtime_error(format!("dock.show: {e}")))?;

        // Headless validation hook: GNOBLIN_DOCK_MENU=<app-id> auto-opens the
        // menu (GNOBLIN_DOCK_MENU_RUNNING=1 forces the running variant).
        if let Ok(app) = std::env::var("GNOBLIN_DOCK_MENU") {
            let is_running = std::env::var("GNOBLIN_DOCK_MENU_RUNNING").is_ok();
            let anchor = self.surface_w.get() as f32 / 2.0;
            let is_pinned = app_context_menu::is_pinned(&app);
            open_menu(
                &dock,
                self.surface_w.get(),
                &self.menu_geom,
                &self.menu_target,
                &app,
                is_running,
                is_pinned,
                anchor,
            );
        }

        self.dock = Some(dock);
        self.favs = default_favourites();
        *self.fav_ids.borrow_mut() = app_context_menu::favorite_ids_from_config();
        self.rebuild();
        let _ = h;
        Ok(())
    }

    fn resized(&mut self, w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        self.surface_w.set(if w > 0 { w } else { screen_w });
        self.screen_w = screen_w;
        self.screen_h = screen_h;
        if let Some(dock) = &self.dock {
            apply_backdrop(dock, screen_w, screen_h);
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;

        // Follow external light/dark changes (e.g. the topbar's toggle).
        let dark = gnoblin_runtime::theme::is_dark();
        if dark != self.theme_dark {
            self.theme_dark = dark;
            if let Some(d) = &self.dock {
                apply_theme(d);
            }
            changed = true;
        }

        let mut latest = None;
        while let Ok(state) = self.win_rx.try_recv() {
            latest = Some(state);
        }
        if let Some(state) = latest {
            self.state = state;
            self.rebuild();
            changed = true;
        }

        // A pin/unpin from the menu wrote the pins file — reload favourites.
        if self.needs_reload.replace(false) {
            self.reload_favs();
            changed = true;
        }

        let config_mtime = file_mtime(self.config_path.as_deref());
        let pins_mtime = file_mtime(self.pins_path.as_deref());
        if config_mtime != self.config_mtime || pins_mtime != self.pins_mtime {
            self.config_mtime = config_mtime;
            self.pins_mtime = pins_mtime;
            if let Some(d) = &self.dock {
                let _ = apply_shell_motion(d);
                apply_shell_chrome(d);
                apply_backdrop(d, self.screen_w, self.screen_h);
                changed = true;
            }
            let ids = app_context_menu::favorite_ids_from_config();
            if *self.fav_ids.borrow() != ids {
                self.reload_favs();
                changed = true;
            }
        }

        // Headless validation: GNOBLIN_DOCK_PIN=<app-id> pins that app once it's
        // running (proves the running-unpinned → pinned flow + persistence).
        if !self.test_pinned {
            if let Ok(app) = std::env::var("GNOBLIN_DOCK_PIN") {
                if self.state.running.iter().any(|w| shell::matches(&app, w)) {
                    app_context_menu::toggle_pin(&app);
                    self.reload_favs();
                    self.test_pinned = true;
                    changed = true;
                }
            }
        }

        // Headless validation: GNOBLIN_DOCK_LAUNCH=<app-id> exercises the same
        // idle-app launch branch as a left click on a non-running dock icon.
        if !self.test_launched {
            if let Ok(app) = std::env::var("GNOBLIN_DOCK_LAUNCH") {
                launch(&app);
                self.test_launched = true;
            }
        }

        // Headless validation: GNOBLIN_DOCK_CLICK=<app-id> simulates one left
        // click on that icon once it's known-running, to prove a running app is
        // focused (not relaunched).
        if !self.test_clicked {
            if let Ok(app) = std::env::var("GNOBLIN_DOCK_CLICK") {
                let running = self.state.running.iter().any(|w| shell::matches(&app, w));
                if running {
                    if let Some(d) = &self.dock {
                        d.invoke_icon_clicked(app.into());
                    }
                    self.test_clicked = true;
                }
            }
        }
        changed
    }

    fn window(&self) -> Option<&slint::Window> {
        self.dock.as_ref().map(|d| d.window())
    }

    fn input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        let w = self.surface_w.get() as i32;
        // While the menu is open, grab the whole surface so the full-surface
        // dismiss catcher (behind the menu) receives outside-clicks.
        if self.menu_geom.get().is_some() {
            return Some(vec![(0, 0, w, SURFACE_H as i32)]);
        }
        // Otherwise only the band (bottom strip) is interactive; the headroom
        // above it is click-through.
        Some(vec![(0, HEADROOM, w, BAND_H as i32)])
    }
}

/// Pinned apps for the dock. Read from `[dock] favorites` in gnoblin.conf
/// (comma-separated .desktop ids); falls back to a sensible default set when
/// unset. Icons are resolved from the system icon theme; the app-id is used as
/// both the icon name and the tooltip.
fn default_favourites() -> Vec<(String, slint::Image)> {
    app_context_menu::favorite_ids_from_config()
        .iter()
        .filter_map(|id| {
            let resolved = gnoblin_desktop::resolve_desktop_id(id);
            gnoblin_desktop::find_icon_at_size(id, "", DOCK_ICON_SIZE)
                .or_else(|| {
                    resolved
                        .as_deref()
                        .and_then(|r| gnoblin_desktop::find_icon_at_size(r, "", DOCK_ICON_SIZE))
                })
                .map(|img| (id.clone(), img))
        })
        .collect()
}

/// Launch an app by its .desktop id (the dock item's app-id), accepting legacy
/// short ids such as `firefox` when the installed id is reverse-DNS.
fn launch(app_id: &str) {
    gnoblin_desktop::launch_desktop_app(app_id);
}

fn main() {
    let (wtx, wrx) = std::sync::mpsc::channel();
    shell::spawn(wtx);
    let config_path = Config::path();
    let pins_path = app_context_menu::pins_path();
    let config_mtime = file_mtime(config_path.as_deref());
    let pins_mtime = file_mtime(pins_path.as_deref());

    run(
        BarConfig {
            namespace: "gnoblin-dock",
            anchor: Anchor::BOTTOM.union(Anchor::LEFT).union(Anchor::RIGHT),
            layer: Layer::Top,
            height: SURFACE_H as u32,
            exclusive_zone: BAND_H as i32,
            full_height: false,
            input_passthrough: false,
            keyboard: false,
        },
        Box::new(DockApp {
            dock: None,
            favs: Vec::new(),
            win_rx: wrx,
            state: WindowState::default(),
            theme_dark: true,
            test_clicked: false,
            surface_w: Rc::new(Cell::new(1280)),
            running: Rc::new(RefCell::new(Vec::new())),
            menu_geom: Rc::new(Cell::new(None)),
            menu_target: Rc::new(RefCell::new(String::new())),
            fav_ids: Rc::new(RefCell::new(Vec::new())),
            needs_reload: Rc::new(Cell::new(false)),
            config_path,
            config_mtime,
            pins_path,
            pins_mtime,
            screen_w: 1280,
            screen_h: 800,
            test_pinned: false,
            test_launched: false,
        }),
    );
}
