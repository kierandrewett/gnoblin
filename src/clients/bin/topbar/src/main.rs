//! gnoblin-topbar — de's Panel as a top wlr-layer-shell client.
mod notifications;
mod qsplugin;
mod quick_settings;
mod quicksettings;
mod settings;

use gnoblin_core::config::Config;
use gnoblin_core::{file_mtime, prettify_app, RuntimeError};
use gnoblin_desktop::appmenu::{self, BarEntry, MenuAddr, MenuCommand, MenuReply};
use gnoblin_desktop::find_icon;
use gnoblin_desktop::tray::{self, TrayCommand, TrayItem};
use gnoblin_runtime::app_context_menu;
use gnoblin_runtime::shell::{self, WindowState};
use gnoblin_runtime::{
    datetime, run, BarApp, BarConfig, BarMargins, PopoutConfig, PopoutHandle, RuntimeControl,
};
use settings::{
    topbar_settings, TopbarCommands, TopbarGeometry, TopbarLayout, WidgetSpec, DEFAULT_CLOCK_FORMAT,
};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

// This client's own Slint UI (TopBar + the structs it uses + Theme/TokenMode).
slint::include_modules!();

/// The focused window's menu address + its bar entries, shared with the
/// global-menu click handler so a click can resolve to a (group, menu).
type MenuState = Rc<RefCell<(MenuAddr, Vec<BarEntry>)>>;

const DATETIME_POPOUT_W: u32 = 350;
const DATETIME_POPOUT_H: u32 = 590;
const CONTROL_CENTRE_POPOUT_W: u32 = 360;
const CONTROL_CENTRE_POPOUT_H: u32 = 632;
const POPOUT_GAP: i32 = 8;

/// Open state + displayed calendar month for the topbar popouts.
#[derive(Default)]
struct Popouts {
    dt_open: Cell<bool>,
    cc_open: Cell<bool>,
    cal: Cell<(i32, u32)>, // displayed (year, month)
}

/// Rebuild the calendar/date strings for the currently displayed month.
fn refresh_datetime(p: &DatetimePopoutWindow, pop: &Popouts) {
    let now = datetime::now();
    let (y, m) = pop.cal.get();
    let today = (y == now.year && m == now.month).then_some(now.day);
    let cells: Vec<CalendarDay> = datetime::calendar(y, m, today)
        .iter()
        .map(|c| CalendarDay {
            day_num: c.day_num,
            is_today: c.is_today,
            is_other_month: c.is_other_month,
            is_weekend: c.is_weekend,
        })
        .collect();
    p.set_calendar_days(Rc::new(slint::VecModel::from(cells)).into());
    p.set_calendar_week_numbers(
        Rc::new(slint::VecModel::from(datetime::week_numbers(y, m))).into(),
    );
    let weekdays: Vec<CalendarWeekday> = datetime::weekday_labels_monday_first()
        .into_iter()
        .map(|label| CalendarWeekday {
            label: label.into(),
        })
        .collect();
    p.set_weekday_labels(Rc::new(slint::VecModel::from(weekdays)).into());
    p.set_calendar_month_text(
        datetime::format_date(y, m, 1, "%B")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{y:04}-{m:02}"))
            .into(),
    );
    p.set_date_text(
        datetime::format_local("%d %B %Y")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{:04}-{:02}-{:02}", now.year, now.month, now.day))
            .into(),
    );
    p.set_day_text(
        (if now.day_name.is_empty() {
            format!("{:04}-{:02}-{:02}", now.year, now.month, now.day)
        } else {
            now.day_name.clone()
        })
        .into(),
    );
}

fn topbar_clock_text(format: &str) -> String {
    datetime::format_local(format)
        .or_else(|| datetime::format_local(DEFAULT_CLOCK_FORMAT))
        .unwrap_or_else(|| "00:00:00".to_string())
}

/// Apply the current light/dark preference to the Slint theme global.
fn apply_theme(p: &TopBar) {
    gnoblin_runtime::apply_shell_theme!(p);
}

fn apply_theme_datetime(p: &DatetimePopoutWindow) {
    gnoblin_runtime::apply_shell_theme!(p);
}

fn apply_theme_control(p: &ControlCentrePopoutWindow) {
    gnoblin_runtime::apply_shell_theme!(p);
}

fn apply_shell_chrome(p: &TopBar) {
    apply_shell_chrome_with(p, gnoblin_runtime::theme::is_dark());
}

fn apply_shell_chrome_datetime(p: &DatetimePopoutWindow) {
    apply_shell_chrome_with_datetime(p, gnoblin_runtime::theme::is_dark());
}

fn apply_shell_chrome_control(p: &ControlCentrePopoutWindow) {
    apply_shell_chrome_with_control(p, gnoblin_runtime::theme::is_dark());
}

fn apply_shell_chrome_with(p: &TopBar, dark: bool) {
    let chrome = gnoblin_runtime::theme::shell_chrome(dark);
    let theme = p.global::<Theme>();
    gnoblin_runtime::apply_shell_chrome_to_theme!(theme, chrome);
}

fn apply_shell_chrome_with_datetime(p: &DatetimePopoutWindow, dark: bool) {
    let chrome = gnoblin_runtime::theme::shell_chrome(dark);
    let theme = p.global::<Theme>();
    gnoblin_runtime::apply_shell_chrome_to_theme!(theme, chrome);
}

fn apply_shell_chrome_with_control(p: &ControlCentrePopoutWindow, dark: bool) {
    let chrome = gnoblin_runtime::theme::shell_chrome(dark);
    let theme = p.global::<Theme>();
    gnoblin_runtime::apply_shell_chrome_to_theme!(theme, chrome);
}

fn apply_shell_motion(p: &TopBar) -> bool {
    gnoblin_runtime::apply_shell_motion!(p)
}

fn apply_shell_motion_datetime(p: &DatetimePopoutWindow) -> bool {
    gnoblin_runtime::apply_shell_motion!(p)
}

fn apply_shell_motion_control(p: &ControlCentrePopoutWindow) -> bool {
    gnoblin_runtime::apply_shell_motion!(p)
}

fn apply_backdrop(p: &TopBar, screen_w: u32, screen_h: u32) {
    p.set_backdrop_screen_w(screen_w as f32);
    p.set_backdrop_screen_h(screen_h as f32);
    p.set_backdrop(gnoblin_runtime::load_backdrop().unwrap_or_default());
}

fn topbar_rect(screen_w: u32, geometry: &TopbarGeometry) -> (i32, i32) {
    let screen_w = screen_w as i32;
    let width = if geometry.width > 0 {
        geometry.width.min(screen_w).max(1)
    } else {
        screen_w
    };
    let base_x = match geometry.align {
        0 => 0,
        2 => screen_w - width,
        _ => (screen_w - width) / 2,
    };
    (
        (base_x + geometry.offset_x).clamp(0, screen_w - width),
        width,
    )
}

fn clamp_popout_left(left: i32, width: u32, screen_w: u32, right_pad: i32) -> i32 {
    let max_left = (screen_w as i32 - width as i32 - right_pad).max(POPOUT_GAP);
    left.clamp(POPOUT_GAP, max_left)
}

fn datetime_popout_margins(anchor_x: f32, screen_w: u32, bar_height: i32) -> BarMargins {
    let left = if anchor_x <= 0.0 {
        (screen_w as i32 - DATETIME_POPOUT_W as i32) / 2
    } else {
        anchor_x.round() as i32 - DATETIME_POPOUT_W as i32 / 2
    };
    BarMargins {
        top: bar_height + POPOUT_GAP,
        left: clamp_popout_left(left, DATETIME_POPOUT_W, screen_w, POPOUT_GAP),
        ..BarMargins::default()
    }
}

fn control_centre_popout_margins(
    anchor_x: f32,
    screen_w: u32,
    bar_height: i32,
    offset_x: i32,
    offset_y: i32,
) -> BarMargins {
    let left = if anchor_x <= 0.0 {
        screen_w as i32 - CONTROL_CENTRE_POPOUT_W as i32 - POPOUT_GAP + offset_x
    } else {
        anchor_x.round() as i32 - CONTROL_CENTRE_POPOUT_W as i32 + offset_x
    };
    BarMargins {
        top: (bar_height + POPOUT_GAP + offset_y).max(0),
        left: clamp_popout_left(left, CONTROL_CENTRE_POPOUT_W, screen_w, 4),
        ..BarMargins::default()
    }
}

fn popout_config(
    namespace: &'static str,
    width: u32,
    height: u32,
    margins: BarMargins,
) -> PopoutConfig {
    PopoutConfig {
        namespace,
        anchor: Anchor::TOP.union(Anchor::LEFT),
        layer: Layer::Overlay,
        width,
        height,
        margins,
        keyboard: false,
    }
}

fn apply_topbar_geometry(p: &TopBar, screen_w: u32, geometry: &TopbarGeometry, height: i32) {
    let (x, width) = topbar_rect(screen_w, geometry);
    p.set_bar_x(x as f32);
    p.set_bar_w(width as f32);
    p.set_bar_h(height as f32);
    p.set_panel_padding_left(geometry.padding_left as f32);
    p.set_panel_padding_right(geometry.padding_right as f32);
    p.set_clock_padding(geometry.clock_padding as f32);
    p.set_status_padding(geometry.status_padding as f32);
    p.set_status_icon_gap(geometry.status_icon_gap as f32);
    p.set_cc_offset_x(geometry.cc_offset_x as f32);
    p.set_cc_offset_y(geometry.cc_offset_y as f32);
}

fn apply_topbar_layout(p: &TopBar, layout: &TopbarLayout) {
    fn model(specs: &[WidgetSpec]) -> slint::ModelRc<TopbarWidget> {
        let items: Vec<TopbarWidget> = specs
            .iter()
            .map(|w| TopbarWidget {
                kind: w.kind,
                flex: w.flex,
                size: w.size,
            })
            .collect();
        Rc::new(slint::VecModel::from(items)).into()
    }

    p.set_left_widgets(model(&layout.left));
    p.set_center_widgets(model(&layout.center));
    p.set_right_widgets(model(&layout.right));
}

fn app_menu_model(app_id: &str, running: bool) -> slint::ModelRc<MenuItem> {
    let items: Vec<MenuItem> =
        app_context_menu::build(app_id, running, app_context_menu::is_pinned(app_id))
            .into_iter()
            .map(|it| MenuItem {
                id: it.id,
                label: it.label.into(),
                accelerator: "".into(),
                separator: it.separator,
                submenu: it.submenu,
                enabled: it.enabled,
            })
            .collect();
    Rc::new(slint::VecModel::from(items)).into()
}

struct TopBarApp {
    panel: Option<TopBar>,
    runtime: Option<RuntimeControl>,
    datetime_popout: Option<DatetimePopoutWindow>,
    control_popout: Option<ControlCentrePopoutWindow>,
    datetime_handle: Rc<Cell<Option<PopoutHandle>>>,
    control_handle: Rc<Cell<Option<PopoutHandle>>>,
    last_clock: String,
    tray_rx: Receiver<Vec<TrayItem>>,
    tray_tx: Sender<TrayCommand>,
    // id -> (service, object_path), so a tray-click can fire the right RPC.
    endpoints: Rc<RefCell<HashMap<i32, (String, String)>>>,
    win_rx: Receiver<WindowState>,
    qs_rx: Receiver<quicksettings::QuickState>,
    // Command/process-driven QS plugin host (spawns + drives declared plugins).
    qs_host: Rc<RefCell<qsplugin::Host>>,
    // Latest built-in quick-settings snapshot + plugin tiles. Both feed the one
    // unified CC tile grid, so both are cached to rebuild it on either change.
    qs_state: quicksettings::QuickState,
    qs_plugins: Rc<RefCell<Vec<qsplugin::PluginState>>>,
    last_focused: String,
    last_workspaces: (u32, u32), // (active, count) last pushed to the panel
    last_pending: bool,          // notification-history unread dot
    last_notif_summary: gnoblin_runtime::notifcenter::Summary,
    // Global menu (appmenu).
    menu_state: MenuState,
    menu_bar: Vec<BarEntry>,
    menu_tx: Sender<MenuCommand>,
    menu_rx: Receiver<MenuReply>,
    // Current dropdown rows (for mapping a row click → its action), shared with
    // the activation closure.
    menu_rows: Rc<RefCell<Vec<appmenu::MenuRow>>>,
    // Whether a dropdown is open (drives the surface input region) + the screen-x
    // the dropdown drops from.
    menu_open: Rc<Cell<bool>>,
    menu_x: Rc<Cell<f32>>,
    // Focused-app context menu.
    app_menu_open: Rc<Cell<bool>>,
    app_menu_target: Rc<RefCell<String>>,
    focused_app_id: Rc<RefCell<String>>,
    running_apps: Rc<RefCell<Vec<String>>>,
    // Test hook: GNOBLIN_APPMENU_AUTOCLICK=<index> opens that top-level entry
    // once the bar is populated (headless mutter can't inject pointer clicks).
    autoclick: Option<i32>,
    // Test hook: GNOBLIN_APPMENU_AUTOACTIVATE=<row> activates a leaf dropdown row.
    autoactivate: Option<usize>,
    // Datetime + control-centre popout state.
    popouts: Rc<Popouts>,
    // Cached dark/light state, to detect external theme changes.
    theme_dark: Rc<Cell<bool>>,
    commands: Rc<RefCell<TopbarCommands>>,
    layout: Rc<RefCell<TopbarLayout>>,
    geometry: TopbarGeometry,
    clock_format: String,
    config_path: Option<PathBuf>,
    config_mtime: Option<SystemTime>,
    screen_w: u32,
    screen_h: u32,
    screen_w_live: Rc<Cell<u32>>,
    screen_h_live: Rc<Cell<u32>>,
    bar_height: i32,
    bar_height_live: Rc<Cell<i32>>,
    bar_x: Cell<i32>,
    bar_w: Cell<i32>,
    cc_offset_x: Rc<Cell<i32>>,
    cc_offset_y: Rc<Cell<i32>>,
}

impl TopBarApp {
    /// Populate + open the dropdown with rows fetched for a clicked entry.
    fn on_submenu(&mut self, _group: u32, _menu: u32, rows: Vec<appmenu::MenuRow>) {
        let model: Vec<MenuItem> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| MenuItem {
                id: i as i32,
                label: r.label.clone().into(),
                accelerator: Default::default(),
                separator: r.separator,
                submenu: r.has_submenu,
                enabled: r.enabled && !r.separator,
            })
            .collect();
        if let Some(p) = &self.panel {
            p.set_global_menu_items(Rc::new(slint::VecModel::from(model)).into());
            p.set_global_menu_x(self.menu_x.get());
            p.set_global_menu_open(true);
        }
        *self.menu_rows.borrow_mut() = rows;
        self.menu_open.set(true);

        // Test hook: GNOBLIN_APPMENU_AUTOACTIVATE=<row> activates a leaf row once
        // its dropdown opens (headless mutter can't inject the click).
        if let Some(idx) = self.autoactivate.take() {
            let addr = self.menu_state.borrow().0.clone();
            if let Some(row) = self.menu_rows.borrow().get(idx) {
                if !row.action.is_empty() {
                    let _ = self.menu_tx.send(MenuCommand::Activate {
                        addr,
                        action: row.action.clone(),
                    });
                }
            }
        }
    }
}

impl BarApp for TopBarApp {
    fn set_runtime(&mut self, runtime: RuntimeControl) {
        self.runtime = Some(runtime);
    }

    fn show(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        let panel =
            TopBar::new().map_err(|e| gnoblin_core::runtime_error(format!("TopBar::new: {e}")))?;
        apply_theme(&panel);
        apply_shell_motion(&panel);
        self.theme_dark.set(gnoblin_runtime::theme::is_dark());
        let clock = topbar_clock_text(&self.clock_format);
        panel.set_clock_text(clock.clone().into());
        panel.set_date_text("".into());
        self.screen_w = screen_w;
        self.screen_h = screen_h;
        let (bar_x, bar_w) = topbar_rect(screen_w, &self.geometry);
        self.bar_x.set(bar_x);
        self.bar_w.set(bar_w);
        apply_topbar_geometry(&panel, screen_w, &self.geometry, self.bar_height);
        apply_topbar_layout(&panel, &self.layout.borrow());
        apply_backdrop(&panel, screen_w, screen_h);
        self.screen_w_live.set(screen_w);
        self.screen_h_live.set(screen_h);
        self.bar_height_live.set(self.bar_height);
        self.cc_offset_x.set(self.geometry.cc_offset_x);
        self.cc_offset_y.set(self.geometry.cc_offset_y);
        let runtime = self
            .runtime
            .as_ref()
            .cloned()
            .ok_or_else(|| gnoblin_core::runtime_error("topbar runtime handle missing"))?;

        // Tray clicks -> Activate / ContextMenu on the item's D-Bus endpoint.
        for (cb, ctx) in [(false, "activate"), (true, "context")] {
            let _ = ctx;
            let tx = self.tray_tx.clone();
            let eps = self.endpoints.clone();
            let handler = move |id: i32| {
                if let Some((service, path)) = eps.borrow().get(&id).cloned() {
                    let _ = tx.send(if cb {
                        TrayCommand::ContextMenu { service, path }
                    } else {
                        TrayCommand::Activate { service, path }
                    });
                }
            };
            if cb {
                panel.on_tray_right_clicked(handler);
            } else {
                panel.on_tray_clicked(handler);
            }
        }

        // Clock → calendar popout; status cluster → control-centre popout.
        {
            let pop = self.popouts.clone();
            let weak = panel.as_weak();
            let app_open = self.app_menu_open.clone();
            let runtime = runtime.clone();
            let dt_handle = self.datetime_handle.clone();
            let cc_handle = self.control_handle.clone();
            let screen_w = self.screen_w_live.clone();
            let bar_height = self.bar_height_live.clone();
            let host = self.qs_host.clone();
            panel.on_toggle_datetime_popout(move |anchor_x| {
                let open = !pop.dt_open.get();
                if open {
                    gnoblin_runtime::notifcenter::clear_legacy_flag();
                    if let Some(h) = cc_handle.take() {
                        runtime.close_popout(h);
                    }
                    pop.cc_open.set(false);
                    host.borrow().broadcast_open(false);
                    let now = datetime::now();
                    pop.cal.set((now.year, now.month));
                    let handle = runtime.open_popout(popout_config(
                        "gnoblin-datetime",
                        DATETIME_POPOUT_W,
                        DATETIME_POPOUT_H,
                        datetime_popout_margins(anchor_x, screen_w.get(), bar_height.get()),
                    ));
                    dt_handle.set(Some(handle));
                } else if let Some(h) = dt_handle.take() {
                    runtime.close_popout(h);
                }
                pop.dt_open.set(open);
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_open(false);
                    app_open.set(false);
                }
            });
        }
        {
            let pop = self.popouts.clone();
            let weak = panel.as_weak();
            let app_open = self.app_menu_open.clone();
            let host = self.qs_host.clone();
            let runtime = runtime.clone();
            let dt_handle = self.datetime_handle.clone();
            let cc_handle = self.control_handle.clone();
            let screen_w = self.screen_w_live.clone();
            let offset_x = self.cc_offset_x.clone();
            let offset_y = self.cc_offset_y.clone();
            let bar_height = self.bar_height_live.clone();
            panel.on_toggle_control_centre(move |anchor_x| {
                let open = !pop.cc_open.get();
                if open {
                    gnoblin_runtime::notifcenter::clear_legacy_flag();
                    if let Some(h) = dt_handle.take() {
                        runtime.close_popout(h);
                    }
                    pop.dt_open.set(false);
                    let handle = runtime.open_popout(popout_config(
                        "gnoblin-controlcentre",
                        CONTROL_CENTRE_POPOUT_W,
                        CONTROL_CENTRE_POPOUT_H,
                        control_centre_popout_margins(
                            anchor_x,
                            screen_w.get(),
                            bar_height.get(),
                            offset_x.get(),
                            offset_y.get(),
                        ),
                    ));
                    cc_handle.set(Some(handle));
                } else if let Some(h) = cc_handle.take() {
                    runtime.close_popout(h);
                }
                pop.cc_open.set(open);
                host.borrow().broadcast_open(open);
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_open(false);
                    app_open.set(false);
                }
            });
        }
        // Control-centre actions live on the auxiliary ControlCentrePopout.
        {
            let commands = self.commands.clone();
            panel.on_launcher_clicked(move || {
                let cmd = commands.borrow().launcher.clone();
                spawn_cmd(&cmd);
            });
        }
        {
            panel.on_workspace_clicked(move |i| {
                // The `workspace` action is 1-based; the indicator is 0-based.
                shell::dispatch_window_action(0, "workspace", &(i + 1).to_string());
            });
        }
        {
            let pop = self.popouts.clone();
            let weak = panel.as_weak();
            let app_open = self.app_menu_open.clone();
            let runtime = runtime.clone();
            let dt_handle = self.datetime_handle.clone();
            let cc_handle = self.control_handle.clone();
            let screen_w = self.screen_w_live.clone();
            let offset_x = self.cc_offset_x.clone();
            let offset_y = self.cc_offset_y.clone();
            let bar_height = self.bar_height_live.clone();
            let host = self.qs_host.clone();
            panel.on_bell_clicked(move || {
                gnoblin_runtime::notifcenter::clear_legacy_flag();
                if let Some(h) = dt_handle.take() {
                    runtime.close_popout(h);
                }
                if let Some(h) = cc_handle.take() {
                    runtime.close_popout(h);
                }
                pop.dt_open.set(false);
                pop.cc_open.set(true);
                host.borrow().broadcast_open(true);
                let handle = runtime.open_popout(popout_config(
                    "gnoblin-controlcentre",
                    CONTROL_CENTRE_POPOUT_W,
                    CONTROL_CENTRE_POPOUT_H,
                    control_centre_popout_margins(
                        0.0,
                        screen_w.get(),
                        bar_height.get(),
                        offset_x.get(),
                        offset_y.get(),
                    ),
                ));
                cc_handle.set(Some(handle));
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_open(false);
                    app_open.set(false);
                }
            });
        }

        // Focused app label -> same app menu model as the dock.
        {
            let weak = panel.as_weak();
            let focused = self.focused_app_id.clone();
            let running = self.running_apps.clone();
            let target = self.app_menu_target.clone();
            let app_open = self.app_menu_open.clone();
            let global_open = self.menu_open.clone();
            panel.on_focused_app_clicked(move |anchor_x| {
                let app = focused.borrow().clone();
                if app.is_empty() {
                    return;
                }
                let is_running = running
                    .borrow()
                    .iter()
                    .any(|running_id| shell::matches(&app, running_id));
                *target.borrow_mut() = app.clone();
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_items(app_menu_model(&app, is_running));
                    p.set_app_menu_x(anchor_x);
                    p.set_app_menu_open(true);
                    p.set_global_menu_open(false);
                    p.set_global_menu_open_index(-1);
                }
                app_open.set(true);
                global_open.set(false);
            });
        }
        {
            let weak = panel.as_weak();
            let target = self.app_menu_target.clone();
            let app_open = self.app_menu_open.clone();
            panel.on_app_menu_item_activated(move |id| {
                let app = target.borrow().clone();
                if !app.is_empty() {
                    let _ = app_context_menu::activate(&app, id);
                }
                app_open.set(false);
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_open(false);
                }
            });
        }
        {
            let weak = panel.as_weak();
            let app_open = self.app_menu_open.clone();
            panel.on_app_menu_dismiss(move || {
                app_open.set(false);
                if let Some(p) = weak.upgrade() {
                    p.set_app_menu_open(false);
                }
            });
        }

        // Global menu: a click on a top-level entry opens its submenu. Resolve
        // the bar id → (group, menu) and ask the worker to fetch the dropdown.
        {
            let state = self.menu_state.clone();
            let tx = self.menu_tx.clone();
            let menu_x = self.menu_x.clone();
            let app_open = self.app_menu_open.clone();
            let weak = panel.as_weak();
            panel.on_global_menu_clicked(move |id, x| {
                menu_x.set(x);
                let (addr, bar) = {
                    let s = state.borrow();
                    (s.0.clone(), s.1.clone())
                };
                if let Some(entry) = bar.get(id as usize) {
                    if let Some(p) = weak.upgrade() {
                        p.set_global_menu_open_index(id);
                        p.set_app_menu_open(false);
                    }
                    app_open.set(false);
                    let _ = tx.send(MenuCommand::OpenSubmenu {
                        addr,
                        group: entry.group,
                        menu: entry.menu,
                    });
                }
            });
        }

        // A dropdown row was picked: a submenu row opens its nested menu in
        // place; a leaf row activates its GTK action and closes the menu.
        {
            let state = self.menu_state.clone();
            let rows = self.menu_rows.clone();
            let tx = self.menu_tx.clone();
            let open = self.menu_open.clone();
            let weak = panel.as_weak();
            panel.on_global_menu_item_activated(move |id| {
                let addr = state.borrow().0.clone();
                let row = rows.borrow().get(id as usize).cloned();
                let Some(row) = row else { return };
                if row.has_submenu {
                    let _ = tx.send(MenuCommand::OpenSubmenu {
                        addr,
                        group: row.group,
                        menu: row.menu,
                    });
                } else {
                    if !row.action.is_empty() {
                        let _ = tx.send(MenuCommand::Activate {
                            addr,
                            action: row.action,
                        });
                    }
                    open.set(false);
                    if let Some(p) = weak.upgrade() {
                        p.set_global_menu_open(false);
                        p.set_global_menu_open_index(-1);
                    }
                }
            });
        }

        // Outside-click / scrim: close the dropdown.
        {
            let open = self.menu_open.clone();
            let app_open = self.app_menu_open.clone();
            let weak = panel.as_weak();
            panel.on_global_menu_dismiss(move || {
                open.set(false);
                app_open.set(false);
                if let Some(p) = weak.upgrade() {
                    p.set_global_menu_open(false);
                    p.set_global_menu_open_index(-1);
                    p.set_app_menu_open(false);
                }
            });
        }

        // Test hook: open a popout on start (headless can't click).
        match std::env::var("GNOBLIN_POPOUT").as_deref() {
            Ok("datetime") => {
                let now = datetime::now();
                self.popouts.dt_open.set(true);
                self.popouts.cal.set((now.year, now.month));
                let handle = runtime.open_popout(popout_config(
                    "gnoblin-datetime",
                    DATETIME_POPOUT_W,
                    DATETIME_POPOUT_H,
                    datetime_popout_margins(0.0, self.screen_w_live.get(), self.bar_height),
                ));
                self.datetime_handle.set(Some(handle));
            }
            Ok("cc") => {
                self.popouts.cc_open.set(true);
                self.qs_host.borrow().broadcast_open(true);
                let handle = runtime.open_popout(popout_config(
                    "gnoblin-controlcentre",
                    CONTROL_CENTRE_POPOUT_W,
                    CONTROL_CENTRE_POPOUT_H,
                    control_centre_popout_margins(
                        0.0,
                        self.screen_w_live.get(),
                        self.bar_height,
                        self.cc_offset_x.get(),
                        self.cc_offset_y.get(),
                    ),
                ));
                self.control_handle.set(Some(handle));
            }
            _ => {}
        }

        // Reflect real network/audio state + plugin tiles in the unified grid
        // from launch (the popout-open handler refreshes it live thereafter).
        if let Some(plugins) = self.qs_host.borrow().poll() {
            *self.qs_plugins.borrow_mut() = plugins;
        }
        self.qs_state = quicksettings::read();
        quick_settings::push(
            &panel,
            self.control_popout.as_ref(),
            &self.qs_state,
            &self.qs_plugins.borrow(),
        );
        self.last_notif_summary = notifications::apply(self.control_popout.as_ref());

        panel
            .show()
            .map_err(|e| gnoblin_core::runtime_error(format!("panel.show: {e}")))?;
        self.last_clock = clock;
        self.panel = Some(panel);
        Ok(())
    }

    fn show_popout(
        &mut self,
        handle: PopoutHandle,
        namespace: &'static str,
        width: u32,
        _height: u32,
        _screen_w: u32,
        screen_h: u32,
    ) -> Result<(), RuntimeError> {
        match namespace {
            "gnoblin-datetime" => {
                let popout = DatetimePopoutWindow::new().map_err(|e| {
                    gnoblin_core::runtime_error(format!("DatetimePopoutWindow::new: {e}"))
                })?;
                apply_theme_datetime(&popout);
                apply_shell_motion_datetime(&popout);
                popout.set_chrome_by_compositor(true);
                popout.set_open(true);
                popout.set_origin_x(width as f32 / 2.0);
                popout.set_origin_y(0.0);
                refresh_datetime(&popout, &self.popouts);

                for next in [false, true] {
                    let pop = self.popouts.clone();
                    let weak = popout.as_weak();
                    let handler = move || {
                        let (y, m) = pop.cal.get();
                        let stepped = if next {
                            if m == 12 {
                                (y + 1, 1)
                            } else {
                                (y, m + 1)
                            }
                        } else if m == 1 {
                            (y - 1, 12)
                        } else {
                            (y, m - 1)
                        };
                        pop.cal.set(stepped);
                        if let Some(p) = weak.upgrade() {
                            refresh_datetime(&p, &pop);
                        }
                    };
                    if next {
                        popout.on_next_month(handler);
                    } else {
                        popout.on_prev_month(handler);
                    }
                }
                popout.on_day_clicked(|_day| {});

                popout
                    .show()
                    .map_err(|e| gnoblin_core::runtime_error(format!("datetime.show: {e}")))?;
                self.datetime_handle.set(Some(handle));
                self.datetime_popout = Some(popout);
            }
            "gnoblin-controlcentre" => {
                let popout = ControlCentrePopoutWindow::new().map_err(|e| {
                    gnoblin_core::runtime_error(format!("ControlCentrePopoutWindow::new: {e}"))
                })?;
                apply_theme_control(&popout);
                apply_shell_motion_control(&popout);
                popout.set_chrome_by_compositor(true);
                popout.set_open(true);
                popout.set_origin_x(width as f32);
                popout.set_origin_y(0.0);
                popout.set_backdrop_screen_h(screen_h as f32);
                popout.set_popout_y(
                    (self.bar_height + POPOUT_GAP + self.cc_offset_y.get()).max(0) as f32,
                );
                quick_settings::push_popout(&popout, &self.qs_plugins.borrow());
                self.last_notif_summary = notifications::apply(Some(&popout));

                {
                    let commands = self.commands.clone();
                    popout.on_account_clicked(move || {
                        let cmd = commands.borrow().account.clone();
                        spawn_cmd(&cmd);
                    });
                }
                {
                    let commands = self.commands.clone();
                    popout.on_settings_clicked(move || {
                        let cmd = commands.borrow().settings.clone();
                        spawn_cmd(&cmd);
                    });
                }
                popout.on_lock_clicked(|| {
                    gnoblin_runtime::shell::dispatch_window_action(0, "lock", "")
                });
                {
                    let commands = self.commands.clone();
                    popout.on_power_clicked(move || {
                        let cmd = commands.borrow().power.clone();
                        spawn_cmd(&cmd);
                    });
                }
                {
                    let host = self.qs_host.clone();
                    popout.on_tile_clicked(move |id| {
                        host.borrow()
                            .send_event(qsplugin::PluginEvent::TileClicked { id: id.to_string() });
                    });
                }
                {
                    let host = self.qs_host.clone();
                    popout.on_tile_slider(move |id, v| {
                        host.borrow().send_event(qsplugin::PluginEvent::Slider {
                            id: id.to_string(),
                            row_id: String::new(),
                            value: v,
                        });
                    });
                }
                popout.on_tile_chevron(|_id| {});
                {
                    let host = self.qs_host.clone();
                    popout.on_plugin_row(move |id, row| {
                        host.borrow().send_event(qsplugin::PluginEvent::Row {
                            id: id.to_string(),
                            row_id: row.to_string(),
                        });
                    });
                }
                {
                    let host = self.qs_host.clone();
                    popout.on_plugin_toggle(move |id, row, v| {
                        host.borrow().send_event(qsplugin::PluginEvent::Toggle {
                            id: id.to_string(),
                            row_id: row.to_string(),
                            value: v,
                        });
                    });
                }
                {
                    let host = self.qs_host.clone();
                    popout.on_plugin_slider(move |id, row, v| {
                        host.borrow().send_event(qsplugin::PluginEvent::Slider {
                            id: id.to_string(),
                            row_id: row.to_string(),
                            value: v,
                        });
                    });
                }
                {
                    let weak = popout.as_weak();
                    popout.on_notification_dismissed(move |index| {
                        if index >= 0 {
                            gnoblin_runtime::notifcenter::dismiss_history_index(index as usize);
                        }
                        if let Some(p) = weak.upgrade() {
                            notifications::apply(Some(&p));
                        }
                    });
                }

                popout
                    .show()
                    .map_err(|e| gnoblin_core::runtime_error(format!("controlcentre.show: {e}")))?;
                self.control_handle.set(Some(handle));
                self.control_popout = Some(popout);
            }
            other => {
                return Err(gnoblin_core::runtime_error(format!(
                    "unknown topbar popout namespace: {other}"
                )));
            }
        }
        Ok(())
    }

    fn popout_window(&self, handle: PopoutHandle) -> Option<&slint::Window> {
        if self.datetime_handle.get() == Some(handle) {
            return self.datetime_popout.as_ref().map(|p| p.window());
        }
        if self.control_handle.get() == Some(handle) {
            return self.control_popout.as_ref().map(|p| p.window());
        }
        None
    }

    fn popout_closed(&mut self, handle: PopoutHandle, namespace: &'static str) {
        if self.datetime_handle.get() == Some(handle) || namespace == "gnoblin-datetime" {
            self.datetime_handle.set(None);
            self.datetime_popout = None;
            self.popouts.dt_open.set(false);
        }
        if self.control_handle.get() == Some(handle) || namespace == "gnoblin-controlcentre" {
            self.control_handle.set(None);
            self.control_popout = None;
            self.popouts.cc_open.set(false);
            self.qs_host.borrow().broadcast_open(false);
        }
    }

    fn resized(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        self.screen_w = screen_w;
        self.screen_h = screen_h;
        self.screen_w_live.set(screen_w);
        self.screen_h_live.set(screen_h);
        self.bar_height_live.set(self.bar_height);
        let (bar_x, bar_w) = topbar_rect(screen_w, &self.geometry);
        self.bar_x.set(bar_x);
        self.bar_w.set(bar_w);
        if let Some(panel) = &self.panel {
            apply_topbar_geometry(panel, screen_w, &self.geometry, self.bar_height);
            apply_backdrop(panel, screen_w, screen_h);
        }
    }

    fn tick(&mut self) -> bool {
        let mut changed = false;

        // Notification-center unread dot (notifyd maintains the flag).
        let pending = gnoblin_runtime::notifcenter::has_pending();
        if pending != self.last_pending {
            self.last_pending = pending;
            if let Some(p) = &self.panel {
                p.set_notif_pending(pending);
            }
            changed = true;
        }
        let notif_summary = gnoblin_runtime::notifcenter::summary();
        if notif_summary != self.last_notif_summary {
            self.last_notif_summary = notif_summary;
            notifications::apply(self.control_popout.as_ref());
            changed = true;
        }

        // Follow external light/dark changes (e.g. another client's toggle).
        let dark = gnoblin_runtime::theme::is_dark();
        if dark != self.theme_dark.get() {
            self.theme_dark.set(dark);
            if let Some(p) = &self.panel {
                apply_theme(p);
            }
            if let Some(p) = &self.datetime_popout {
                apply_theme_datetime(p);
            }
            if let Some(p) = &self.control_popout {
                apply_theme_control(p);
            }
            changed = true;
        }

        let clock = topbar_clock_text(&self.clock_format);
        if clock != self.last_clock {
            if let Some(p) = &self.panel {
                p.set_clock_text(clock.clone().into());
                p.set_date_text("".into());
            }
            if let Some(p) = &self.datetime_popout {
                refresh_datetime(p, &self.popouts);
            }
            notifications::apply(self.control_popout.as_ref());
            self.last_clock = clock;
            changed = true;
        }

        // Drain to the latest tray snapshot.
        let mut latest = None;
        while let Ok(items) = self.tray_rx.try_recv() {
            latest = Some(items);
        }
        // Drain to the latest quick-settings snapshot (network glyph, mute, cc).
        let mut latest_qs = None;
        while let Ok(st) = self.qs_rx.try_recv() {
            latest_qs = Some(st);
        }
        if let Some(st) = latest_qs {
            self.qs_state = st;
            if let Some(p) = &self.panel {
                quick_settings::push(
                    p,
                    self.control_popout.as_ref(),
                    &self.qs_state,
                    &self.qs_plugins.borrow(),
                );
            }
            changed = true;
        }

        // Drain to the latest QS plugin snapshot (process-driven tiles/menus).
        // Rebuild the grid from the cached built-in state so the (possibly
        // high-frequency) plugin tick doesn't re-read wpctl/D-Bus each time.
        if let Some(plugins) = self.qs_host.borrow().poll() {
            *self.qs_plugins.borrow_mut() = plugins;
            if let Some(p) = &self.panel {
                quick_settings::push(
                    p,
                    self.control_popout.as_ref(),
                    &self.qs_state,
                    &self.qs_plugins.borrow(),
                );
            }
            changed = true;
        }

        let config_mtime = file_mtime(self.config_path.as_deref());
        if config_mtime != self.config_mtime {
            self.config_mtime = config_mtime;
            // Re-spawn QS plugins to match the (possibly changed) declarations.
            let plugin_cfgs = qsplugin::load_configs(&Config::load());
            if self.qs_host.borrow().configs() != plugin_cfgs.as_slice() {
                self.qs_host.borrow_mut().apply(plugin_cfgs);
                changed = true;
            }
            let settings = topbar_settings();
            if *self.commands.borrow() != settings.commands {
                *self.commands.borrow_mut() = settings.commands;
            }
            if *self.layout.borrow() != settings.layout {
                *self.layout.borrow_mut() = settings.layout;
                if let Some(p) = &self.panel {
                    apply_topbar_layout(p, &self.layout.borrow());
                }
            }
            if self.geometry != settings.geometry {
                self.geometry = settings.geometry;
                self.cc_offset_x.set(self.geometry.cc_offset_x);
                self.cc_offset_y.set(self.geometry.cc_offset_y);
                let (bar_x, bar_w) = topbar_rect(self.screen_w, &self.geometry);
                self.bar_x.set(bar_x);
                self.bar_w.set(bar_w);
                if let Some(p) = &self.panel {
                    apply_topbar_geometry(p, self.screen_w, &self.geometry, self.bar_height);
                }
                changed = true;
            }
            if self.bar_height != settings.height {
                self.bar_height = settings.height;
                self.bar_height_live.set(self.bar_height);
                if let Some(p) = &self.panel {
                    apply_topbar_geometry(p, self.screen_w, &self.geometry, self.bar_height);
                }
                changed = true;
            }
            if self.clock_format != settings.clock_format {
                self.clock_format = settings.clock_format;
                self.last_clock.clear();
            }
            if let Some(p) = &self.panel {
                let _ = apply_shell_motion(p);
                apply_shell_chrome(p);
                apply_backdrop(p, self.screen_w, self.screen_h);
                changed = true;
            }
            if let Some(p) = &self.datetime_popout {
                let _ = apply_shell_motion_datetime(p);
                apply_shell_chrome_datetime(p);
            }
            if let Some(p) = &self.control_popout {
                let _ = apply_shell_motion_control(p);
                apply_shell_chrome_control(p);
            }
        }

        // Drain to the latest window-state snapshot; update the focused-app label.
        let mut latest_win = None;
        while let Ok(state) = self.win_rx.try_recv() {
            latest_win = Some(state);
        }
        if let Some(state) = latest_win {
            if state.focused != self.last_focused {
                if let Some(p) = &self.panel {
                    p.set_focused_app(prettify_app(&state.focused).into());
                    if state.focused.is_empty() {
                        p.set_app_menu_open(false);
                        self.app_menu_open.set(false);
                    }
                }
                self.last_focused = state.focused.clone();
                changed = true;
            }
            *self.focused_app_id.borrow_mut() = state.focused.clone();
            *self.running_apps.borrow_mut() = state.running.clone();

            // Workspace indicator.
            let ws = (state.active_workspace, state.n_workspaces);
            if ws != self.last_workspaces {
                if let Some(p) = &self.panel {
                    p.set_workspace_active(state.active_workspace as i32);
                    p.set_workspace_count(state.n_workspaces as i32);
                }
                self.last_workspaces = ws;
                changed = true;
            }

            // Global menu bar (File / Edit / …) for the focused window.
            if state.menu_bar != self.menu_bar {
                if let Some(p) = &self.panel {
                    let model: Vec<GlobalMenuTopItem> = state
                        .menu_bar
                        .iter()
                        .enumerate()
                        .map(|(i, e)| GlobalMenuTopItem {
                            id: i as i32,
                            label: e.label.clone().into(),
                        })
                        .collect();
                    p.set_global_menu_bar(Rc::new(slint::VecModel::from(model)).into());
                    if state.menu_bar.is_empty() {
                        p.set_global_menu_open_index(-1);
                    }
                }
                self.menu_bar = state.menu_bar.clone();
                *self.menu_state.borrow_mut() = (state.menu_addr.clone(), state.menu_bar.clone());
                changed = true;

                // Test hook: programmatically open one entry.
                if let Some(idx) = self.autoclick.take() {
                    if let Some(entry) = self.menu_bar.get(idx as usize) {
                        self.menu_x.set(8.0);
                        if let Some(p) = &self.panel {
                            p.set_global_menu_open_index(idx);
                        }
                        let _ = self.menu_tx.send(MenuCommand::OpenSubmenu {
                            addr: state.menu_addr.clone(),
                            group: entry.group,
                            menu: entry.menu,
                        });
                    }
                }
            }
        }

        // Drain appmenu worker replies (dropdown rows — rendered in show()'s
        // popup surface; see the dropdown handling below).
        while let Ok(reply) = self.menu_rx.try_recv() {
            match reply {
                MenuReply::Submenu { group, menu, rows } => {
                    self.on_submenu(group, menu, rows);
                    changed = true;
                }
            }
        }

        if let (Some(items), Some(panel)) = (latest, &self.panel) {
            let mut eps = self.endpoints.borrow_mut();
            eps.clear();
            let model: Vec<TrayIconItem> = items
                .iter()
                .map(|it| {
                    eps.insert(it.id, (it.service.clone(), it.object_path.clone()));
                    TrayIconItem {
                        id: it.id,
                        title: it.title.clone().into(),
                        icon_name: it.icon_name.clone().into(),
                        icon: find_icon(&it.icon_name, &it.icon_theme_path).unwrap_or_default(),
                        service: it.service.clone().into(),
                        object_path: it.object_path.clone().into(),
                    }
                })
                .collect();
            panel.set_tray_items(Rc::new(slint::VecModel::from(model)).into());
            changed = true;
        }

        changed
    }

    fn window(&self) -> Option<&slint::Window> {
        self.panel.as_ref().map(|p| p.window())
    }

    fn input_full(&self) -> bool {
        // In-surface dropdowns still need the full-height topbar surface to catch
        // outside clicks. Popouts are separate layer surfaces dismissed by the
        // compositor popup grab.
        self.menu_open.get() || self.app_menu_open.get()
    }

    fn input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        if self.input_full() {
            return Some(vec![(0, 0, self.screen_w as i32, self.screen_h as i32)]);
        }
        Some(vec![(
            self.bar_x.get(),
            0,
            self.bar_w.get().max(1),
            self.bar_height.max(1),
        )])
    }
}

/// Run a shell command detached (for the configurable topbar buttons).
fn spawn_cmd(cmd: &str) {
    use std::process::{Command, Stdio};
    if cmd.trim().is_empty() {
        return;
    }
    let _ = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn main() {
    let (utx, urx) = std::sync::mpsc::channel();
    let (ctx, crx) = std::sync::mpsc::channel();
    tray::spawn(utx, crx);

    let (wtx, wrx) = std::sync::mpsc::channel();
    shell::spawn(wtx);

    let (qtx, qrx) = std::sync::mpsc::channel();
    quicksettings::spawn(qtx);

    let (menu_tx, menu_cmd_rx) = std::sync::mpsc::channel();
    let (menu_reply_tx, menu_rx) = std::sync::mpsc::channel();
    appmenu::spawn(menu_cmd_rx, menu_reply_tx);
    let config_path = Config::path();
    let config_mtime = file_mtime(config_path.as_deref());
    let settings = topbar_settings();
    let initial_cc_offset_x = settings.geometry.cc_offset_x;
    let initial_cc_offset_y = settings.geometry.cc_offset_y;

    // Spawn the command/process-driven QS plugin host from the declared
    // [qs-plugin.*] / [providers] config.
    let qs_host = Rc::new(RefCell::new(qsplugin::Host::spawn(qsplugin::load_configs(
        &Config::load(),
    ))));

    run(
        BarConfig {
            namespace: "gnoblin-topbar",
            anchor: Anchor::TOP.union(Anchor::LEFT).union(Anchor::RIGHT),
            layer: Layer::Top,
            height: settings.height as u32,
            exclusive_zone: settings.exclusive_zone,
            full_height: true,
            input_passthrough: false,
            keyboard: false,
            ..BarConfig::default()
        },
        Box::new(TopBarApp {
            panel: None,
            runtime: None,
            datetime_popout: None,
            control_popout: None,
            datetime_handle: Rc::new(Cell::new(None)),
            control_handle: Rc::new(Cell::new(None)),
            last_clock: String::new(),
            tray_rx: urx,
            tray_tx: ctx,
            endpoints: Rc::new(RefCell::new(HashMap::new())),
            win_rx: wrx,
            qs_rx: qrx,
            qs_host,
            qs_state: quicksettings::QuickState::default(),
            qs_plugins: Rc::new(RefCell::new(Vec::new())),
            last_focused: String::new(),
            last_workspaces: (0, 0),
            last_pending: false,
            last_notif_summary: gnoblin_runtime::notifcenter::Summary::default(),
            menu_state: Rc::new(RefCell::new((MenuAddr::default(), Vec::new()))),
            menu_bar: Vec::new(),
            menu_tx,
            menu_rx,
            menu_rows: Rc::new(RefCell::new(Vec::new())),
            menu_open: Rc::new(Cell::new(false)),
            menu_x: Rc::new(Cell::new(0.0)),
            app_menu_open: Rc::new(Cell::new(false)),
            app_menu_target: Rc::new(RefCell::new(String::new())),
            focused_app_id: Rc::new(RefCell::new(String::new())),
            running_apps: Rc::new(RefCell::new(Vec::new())),
            autoclick: std::env::var("GNOBLIN_APPMENU_AUTOCLICK")
                .ok()
                .and_then(|s| s.parse().ok()),
            autoactivate: std::env::var("GNOBLIN_APPMENU_AUTOACTIVATE")
                .ok()
                .and_then(|s| s.parse().ok()),
            popouts: Rc::new(Popouts::default()),
            theme_dark: Rc::new(Cell::new(true)),
            commands: Rc::new(RefCell::new(settings.commands)),
            layout: Rc::new(RefCell::new(settings.layout)),
            geometry: settings.geometry,
            clock_format: settings.clock_format,
            config_path,
            config_mtime,
            screen_w: 1280,
            screen_h: 800,
            screen_w_live: Rc::new(Cell::new(1280)),
            screen_h_live: Rc::new(Cell::new(800)),
            bar_height: settings.height,
            bar_height_live: Rc::new(Cell::new(settings.height)),
            bar_x: Cell::new(0),
            bar_w: Cell::new(1280),
            cc_offset_x: Rc::new(Cell::new(initial_cc_offset_x)),
            cc_offset_y: Rc::new(Cell::new(initial_cc_offset_y)),
        }),
    );
}
