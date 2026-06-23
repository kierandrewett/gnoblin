//! gnoblin-topbar — de's Panel as a top wlr-layer-shell client.
use gnoblin_shell_ui::app_context_menu;
use gnoblin_shell_ui::appmenu::{self, BarEntry, MenuAddr, MenuCommand, MenuReply};
use gnoblin_shell_ui::config::Config;
use gnoblin_shell_ui::shell::{self, WindowState};
use gnoblin_shell_ui::tray::{self, TrayCommand, TrayItem};
use gnoblin_shell_ui::{datetime, find_icon, run, BarApp, BarConfig, RuntimeError};
use slint::ComponentHandle;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

// This client's own Slint UI (TopBar + the structs it uses + Theme/TokenMode).
slint::include_modules!();

const DEFAULT_CLOCK_FORMAT: &str = "%a %d %b  %H:%M:%S";

/// The focused window's menu address + its bar entries, shared with the
/// global-menu click handler so a click can resolve to a (group, menu).
type MenuState = Rc<RefCell<(MenuAddr, Vec<BarEntry>)>>;

#[derive(Clone, PartialEq, Eq)]
struct TopbarCommands {
    launcher: String,
    account: String,
    settings: String,
    power: String,
}

#[derive(Clone, PartialEq, Eq)]
struct WidgetSpec {
    kind: i32,
    flex: i32,
    size: i32,
}

#[derive(Clone, PartialEq, Eq)]
struct TopbarLayout {
    left: Vec<WidgetSpec>,
    center: Vec<WidgetSpec>,
    right: Vec<WidgetSpec>,
}

#[derive(Clone, PartialEq, Eq)]
struct TopbarGeometry {
    width: i32,
    align: i32,
    offset_x: i32,
    padding_left: i32,
    padding_right: i32,
    clock_padding: i32,
    status_padding: i32,
    status_icon_gap: i32,
    cc_offset_x: i32,
    cc_offset_y: i32,
}

#[derive(Clone, PartialEq, Eq)]
struct TopbarSettings {
    commands: TopbarCommands,
    layout: TopbarLayout,
    geometry: TopbarGeometry,
    height: i32,
    exclusive_zone: i32,
    clock_format: String,
}

/// Open state + displayed calendar month for the topbar popouts.
#[derive(Default)]
struct Popouts {
    dt_open: Cell<bool>,
    cc_open: Cell<bool>,
    cal: Cell<(i32, u32)>, // displayed (year, month)
}

/// Rebuild the calendar/date strings for the currently displayed month.
fn refresh_datetime(p: &TopBar, pop: &Popouts) {
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
    p.set_popout_calendar_days(Rc::new(slint::VecModel::from(cells)).into());
    p.set_popout_calendar_week_numbers(
        Rc::new(slint::VecModel::from(datetime::week_numbers(y, m))).into(),
    );
    let weekdays: Vec<CalendarWeekday> = datetime::weekday_labels_monday_first()
        .into_iter()
        .map(|label| CalendarWeekday {
            label: label.into(),
        })
        .collect();
    p.set_popout_weekday_labels(Rc::new(slint::VecModel::from(weekdays)).into());
    p.set_popout_calendar_month_text(
        datetime::format_date(y, m, 1, "%B")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{y:04}-{m:02}"))
            .into(),
    );
    p.set_popout_date_text(
        datetime::format_local("%d %B %Y")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{:04}-{:02}-{:02}", now.year, now.month, now.day))
            .into(),
    );
    p.set_popout_day_text(
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
    let dark = gnoblin_shell_ui::theme::is_dark();
    let mode = if dark {
        TokenMode::Dark
    } else {
        TokenMode::Light
    };
    p.global::<Theme>().set_mode(mode);
    apply_shell_chrome_with(p, dark);
}

fn apply_shell_chrome(p: &TopBar) {
    apply_shell_chrome_with(p, gnoblin_shell_ui::theme::is_dark());
}

fn apply_shell_chrome_with(p: &TopBar, dark: bool) {
    let chrome = gnoblin_shell_ui::theme::shell_chrome(dark);
    let theme = p.global::<Theme>();
    gnoblin_shell_ui::apply_shell_chrome_to_theme!(theme, chrome);
}

fn apply_shell_motion(p: &TopBar) -> bool {
    let motion = gnoblin_shell_ui::prefs::shell_motion();
    let theme = p.global::<Theme>();
    gnoblin_shell_ui::apply_shell_motion_to_theme!(theme, motion)
}

fn apply_backdrop(p: &TopBar, screen_w: u32, screen_h: u32) {
    p.set_backdrop_screen_w(screen_w as f32);
    p.set_backdrop_screen_h(screen_h as f32);
    p.set_backdrop(gnoblin_shell_ui::load_backdrop().unwrap_or_default());
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
                enabled: it.enabled,
            })
            .collect();
    Rc::new(slint::VecModel::from(items)).into()
}

/// Push a quick-settings snapshot into the control-centre popout + the status
/// cluster (network glyph, mute). Shared by the one-shot read and the poller.
/// Topbar status cluster (network glyph + mute icon) — independent of the CC
/// tile grid, so it's set separately from the tile model.
fn apply_cluster(p: &TopBar, st: &gnoblin_shell_ui::quicksettings::QuickState) {
    // Cluster network glyph: wired wins when both are up (the active route, as
    // GNOME does); else wifi; else disconnected. GNOBLIN_NET_MODE overrides for
    // headless validation of the wifi/disconnected variants.
    let net_mode = std::env::var("GNOBLIN_NET_MODE")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(if st.wired {
            1
        } else if st.wifi {
            2
        } else {
            0
        });
    p.set_network_mode(net_mode);
    // GNOBLIN_AUDIO_MUTED overrides for headless validation of the mute glyph.
    let muted = std::env::var("GNOBLIN_AUDIO_MUTED")
        .map(|v| v == "1")
        .unwrap_or(st.muted);
    p.set_audio_muted(muted);
}

/// One built-in tile. Its glyph is named (resolved to a bundled symbolic asset
/// in Slint via QsIcons.glyph), not a pre-loaded image.
#[allow(clippy::too_many_arguments)]
/// Greedily pack tiles into rows of a 4-wide span grid, wrapping when the next
/// tile would overflow the row's 4 columns.
fn pack_rows(tiles: Vec<QsTile>) -> Vec<QsTileRow> {
    let mut rows: Vec<Vec<QsTile>> = Vec::new();
    let mut cur: Vec<QsTile> = Vec::new();
    let mut used = 0;
    for t in tiles {
        let span = t.span.clamp(1, 4);
        if used + span > 4 && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
        used += span;
        cur.push(t);
        if used >= 4 {
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows.into_iter()
        .map(|tiles| QsTileRow {
            tiles: Rc::new(slint::VecModel::from(tiles)).into(),
        })
        .collect()
}

/// Build the unified tile list: the dogfooded built-ins (synthesised from live
/// state) followed by every ready plugin tile. One model, one render path, no
/// "Plugins" section — see the unified-grid design note.
fn build_qs_tiles(plugins: &[gnoblin_shell_ui::qsplugin::PluginState]) -> Vec<QsTile> {
    // EVERY tile is a config-declared async plugin — there are no hardcoded
    // built-ins. Render the host's snapshot in its order (the host preserves the
    // declared/`[quicksettings] order` order; see qsplugin::load_configs).
    let mut tiles = Vec::new();
    for pl in plugins {
        let spec = &pl.update.tile;
        let layout = if spec.layout.is_empty() {
            "toggle"
        } else {
            spec.layout.as_str()
        };
        let span = if spec.span == 0 {
            if layout == "toggle" {
                2
            } else {
                4
            }
        } else {
            spec.span
        };
        let rows: Vec<QsMenuRow> = pl
            .update
            .menu
            .as_ref()
            .map(|m| {
                m.rows
                    .iter()
                    .map(|r| QsMenuRow {
                        id: r.id.clone().into(),
                        kind: r.kind.clone().into(),
                        label: r.label.clone().into(),
                        sublabel: r.sublabel.clone().into(),
                        icon: find_icon(&r.icon, "").unwrap_or_default(),
                        value: r.value,
                        on: r.on,
                    })
                    .collect()
            })
            .unwrap_or_default();
        tiles.push(QsTile {
            id: pl.id.clone().into(),
            span,
            layout: layout.into(),
            icon_name: String::new().into(),
            icon: find_icon(&spec.icon, "").unwrap_or_default(),
            title: spec.title.clone().into(),
            subtitle: spec.subtitle.clone().into(),
            active: spec.active,
            chevron: spec.chevron || !rows.is_empty(),
            // Clamp to the track range: a slider plugin (e.g. gnoblin-qs-output)
            // can report >1.0 for an over-amplified PipeWire volume, which would
            // overdraw the fill past the track.
            value: spec.value.clamp(0.0, 1.0),
            rows: Rc::new(slint::VecModel::from(rows)).into(),
        });
    }

    tiles
}

/// Push the unified CC tile model + status cluster from a known state snapshot.
fn push_cc_tiles(
    p: &TopBar,
    st: &gnoblin_shell_ui::quicksettings::QuickState,
    plugins: &[gnoblin_shell_ui::qsplugin::PluginState],
) {
    apply_cluster(p, st);
    let tiles = build_qs_tiles(plugins);
    // If a slide-out submenu is open, refresh its rows from the freshly-built
    // model so a plugin update mid-open doesn't leave the page showing a stale
    // snapshot (the rows were captured when the chevron was tapped).
    let open = p.get_cc_open_submenu().to_string();
    if !open.is_empty() {
        if let Some(t) = tiles.iter().find(|t| t.id == open) {
            p.set_cc_submenu_rows(t.rows.clone());
        }
    }
    p.set_cc_tiles(Rc::new(slint::VecModel::from(pack_rows(tiles))).into());
}

/// Re-read live built-in state and rebuild the CC tile grid (used by toggles +
/// popout-open, where a fresh read is wanted and cheap enough).
fn refresh_cc_tiles(p: &TopBar, plugins: &[gnoblin_shell_ui::qsplugin::PluginState]) {
    let st = gnoblin_shell_ui::quicksettings::read();
    push_cc_tiles(p, &st, plugins);
}

/// One-shot synchronous read (startup + popout-open, for an immediate refresh).
fn notification_age_label(timestamp_secs: u64) -> String {
    if timestamp_secs == 0 {
        return String::new();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let elapsed = now.saturating_sub(timestamp_secs);
    let format = if elapsed < 86_400 { "%H:%M" } else { "%x" };
    datetime::format_unix(timestamp_secs, format).unwrap_or_default()
}

fn apply_notifications(p: &TopBar) -> gnoblin_shell_ui::notifcenter::Summary {
    let summary = gnoblin_shell_ui::notifcenter::summary();
    let entries = gnoblin_shell_ui::notifcenter::history();
    let items: Vec<NotificationItem> = entries
        .iter()
        .take(8)
        .map(|entry| {
            let icon = find_icon(&entry.icon_name, "");
            NotificationItem {
                app_name: if entry.app_name.is_empty() {
                    "Notification".into()
                } else {
                    entry.app_name.clone().into()
                },
                summary: entry.summary.clone().into(),
                body: entry.body.clone().into(),
                age: notification_age_label(entry.timestamp_secs).into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
            }
        })
        .collect();
    let count = summary.count.max(entries.len()).min(i32::MAX as usize) as i32;
    p.set_cc_notification_count(count);
    p.set_cc_notifications(Rc::new(slint::VecModel::from(items)).into());
    summary
}

struct TopBarApp {
    panel: Option<TopBar>,
    last_clock: String,
    tray_rx: Receiver<Vec<TrayItem>>,
    tray_tx: Sender<TrayCommand>,
    // id -> (service, object_path), so a tray-click can fire the right RPC.
    endpoints: Rc<RefCell<HashMap<i32, (String, String)>>>,
    win_rx: Receiver<WindowState>,
    qs_rx: Receiver<gnoblin_shell_ui::quicksettings::QuickState>,
    // Command/process-driven QS plugin host (spawns + drives declared plugins).
    qs_host: Rc<RefCell<gnoblin_shell_ui::qsplugin::Host>>,
    // Latest built-in quick-settings snapshot + plugin tiles. Both feed the one
    // unified CC tile grid, so both are cached to rebuild it on either change.
    qs_state: gnoblin_shell_ui::quicksettings::QuickState,
    qs_plugins: Rc<RefCell<Vec<gnoblin_shell_ui::qsplugin::PluginState>>>,
    last_focused: String,
    last_workspaces: (u32, u32), // (active, count) last pushed to the panel
    last_pending: bool,          // notification-history unread dot
    last_notif_summary: gnoblin_shell_ui::notifcenter::Summary,
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
    bar_height: i32,
    bar_x: Cell<i32>,
    bar_w: Cell<i32>,
}

/// Turn an app-id into a friendly label: drop the reverse-DNS prefix and the
/// `.desktop` suffix, then title-case the remainder. `org.gnome.TextEditor` →
/// `TextEditor`; `firefox` → `Firefox`.
use gnoblin_shell_ui::prettify_app;

impl TopBarApp {
    /// Populate + open the dropdown with rows fetched for a clicked entry.
    fn on_submenu(&mut self, _group: u32, _menu: u32, rows: Vec<appmenu::MenuRow>) {
        let model: Vec<MenuItem> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| MenuItem {
                id: i as i32,
                label: if r.has_submenu {
                    format!("{} ▸", r.label).into()
                } else {
                    r.label.clone().into()
                },
                accelerator: Default::default(),
                separator: r.separator,
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
    fn show(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) -> Result<(), RuntimeError> {
        let panel = TopBar::new()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("TopBar::new: {e}")))?;
        apply_theme(&panel);
        apply_shell_motion(&panel);
        self.theme_dark.set(gnoblin_shell_ui::theme::is_dark());
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
            panel.on_toggle_datetime_popout(move |anchor_x| {
                let open = !pop.dt_open.get();
                if open {
                    gnoblin_shell_ui::notifcenter::set(false);
                }
                pop.dt_open.set(open);
                pop.cc_open.set(false);
                if let Some(p) = weak.upgrade() {
                    if open {
                        let now = datetime::now();
                        pop.cal.set((now.year, now.month));
                        refresh_datetime(&p, &pop);
                        p.set_datetime_anchor_x(anchor_x);
                    }
                    p.set_datetime_open(open);
                    p.set_cc_open(false);
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
            let plugins = self.qs_plugins.clone();
            panel.on_toggle_control_centre(move |anchor_x| {
                let open = !pop.cc_open.get();
                if open {
                    gnoblin_shell_ui::notifcenter::set(false);
                }
                pop.cc_open.set(open);
                pop.dt_open.set(false);
                host.borrow().broadcast_open(open);
                if let Some(p) = weak.upgrade() {
                    if open {
                        refresh_cc_tiles(&p, &plugins.borrow());
                        apply_notifications(&p);
                        p.set_cc_anchor_x(anchor_x);
                    }
                    p.set_cc_open(open);
                    p.set_datetime_open(false);
                    p.set_app_menu_open(false);
                    app_open.set(false);
                }
            });
        }
        for next in [false, true] {
            let pop = self.popouts.clone();
            let weak = panel.as_weak();
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
                panel.on_popout_next_month(handler);
            } else {
                panel.on_popout_prev_month(handler);
            }
        }
        {
            let pop = self.popouts.clone();
            let weak = panel.as_weak();
            panel.on_popout_dismiss(move || {
                pop.dt_open.set(false);
                pop.cc_open.set(false);
                if let Some(p) = weak.upgrade() {
                    p.set_datetime_open(false);
                    p.set_cc_open(false);
                }
            });
        }
        // Control-centre actions.
        {
            let commands = self.commands.clone();
            panel.on_launcher_clicked(move || {
                let cmd = commands.borrow().launcher.clone();
                spawn_cmd(&cmd);
            });
        }
        // ── Header actions (account / settings / lock / power) ───────────────
        {
            let commands = self.commands.clone();
            panel.on_cc_account_clicked(move || {
                let cmd = commands.borrow().account.clone();
                spawn_cmd(&cmd);
            });
        }
        {
            let commands = self.commands.clone();
            panel.on_cc_settings_clicked(move || {
                let cmd = commands.borrow().settings.clone();
                spawn_cmd(&cmd);
            });
        }
        panel.on_cc_lock_clicked(|| gnoblin_shell_ui::shell::dispatch_window_action(0, "lock", ""));
        {
            let commands = self.commands.clone();
            panel.on_cc_power_clicked(move || {
                let cmd = commands.borrow().power.clone();
                spawn_cmd(&cmd);
            });
        }
        // ── Unified tile dispatch ────────────────────────────────────────────
        // Every tile is a config-declared plugin: tap and slide events go
        // straight to the qsplugin host, which forwards them to the owning plugin
        // process (gnoblin-qs-* etc.). The plugin performs the action (flip a
        // state file, run wpctl/nmcli, …) and its next poll reflects the result.
        {
            let host = self.qs_host.clone();
            panel.on_cc_tile_clicked(move |id| {
                host.borrow()
                    .send_event(gnoblin_shell_ui::qsplugin::PluginEvent::TileClicked {
                        id: id.to_string(),
                    });
            });
        }
        {
            let host = self.qs_host.clone();
            panel.on_cc_tile_slider(move |id, v| {
                host.borrow()
                    .send_event(gnoblin_shell_ui::qsplugin::PluginEvent::Slider {
                        id: id.to_string(),
                        row_id: String::new(),
                        value: v,
                    });
            });
        }
        // The chevron opens the slide-out submenu in Slint; nothing extra here.
        panel.on_cc_tile_chevron(|_id| {});
        // Submenu row interactions → the qsplugin host (keyed by the tile/plugin
        // id; built-in tiles have no host rows, so these only fire for plugins).
        {
            let host = self.qs_host.clone();
            panel.on_cc_plugin_row(move |id, row| {
                host.borrow()
                    .send_event(gnoblin_shell_ui::qsplugin::PluginEvent::Row {
                        id: id.to_string(),
                        row_id: row.to_string(),
                    });
            });
        }
        {
            let host = self.qs_host.clone();
            panel.on_cc_plugin_toggle(move |id, row, v| {
                host.borrow()
                    .send_event(gnoblin_shell_ui::qsplugin::PluginEvent::Toggle {
                        id: id.to_string(),
                        row_id: row.to_string(),
                        value: v,
                    });
            });
        }
        {
            let host = self.qs_host.clone();
            panel.on_cc_plugin_slider(move |id, row, v| {
                host.borrow()
                    .send_event(gnoblin_shell_ui::qsplugin::PluginEvent::Slider {
                        id: id.to_string(),
                        row_id: row.to_string(),
                        value: v,
                    });
            });
        }
        {
            let weak = panel.as_weak();
            panel.on_cc_notification_dismissed(move |index| {
                if index >= 0 {
                    gnoblin_shell_ui::notifcenter::dismiss_history_index(index as usize);
                }
                if let Some(p) = weak.upgrade() {
                    apply_notifications(&p);
                }
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
            let plugins = self.qs_plugins.clone();
            panel.on_bell_clicked(move || {
                gnoblin_shell_ui::notifcenter::set(false);
                pop.dt_open.set(false);
                pop.cc_open.set(true);
                if let Some(p) = weak.upgrade() {
                    refresh_cc_tiles(&p, &plugins.borrow());
                    apply_notifications(&p);
                    p.set_datetime_open(false);
                    p.set_cc_anchor_x(0.0);
                    p.set_cc_open(true);
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
                refresh_datetime(&panel, &self.popouts);
                panel.set_datetime_open(true);
            }
            Ok("cc") => {
                self.popouts.cc_open.set(true);
                panel.set_cc_open(true);
            }
            _ => {}
        }

        // Reflect real network/audio state + plugin tiles in the unified grid
        // from launch (the popout-open handler refreshes it live thereafter).
        if let Some(plugins) = self.qs_host.borrow().poll() {
            *self.qs_plugins.borrow_mut() = plugins;
        }
        self.qs_state = gnoblin_shell_ui::quicksettings::read();
        push_cc_tiles(&panel, &self.qs_state, &self.qs_plugins.borrow());
        self.last_notif_summary = apply_notifications(&panel);

        panel
            .show()
            .map_err(|e| gnoblin_shell_ui::runtime_error(format!("panel.show: {e}")))?;
        self.last_clock = clock;
        self.panel = Some(panel);
        Ok(())
    }

    fn resized(&mut self, _w: u32, _h: u32, screen_w: u32, screen_h: u32) {
        self.screen_w = screen_w;
        self.screen_h = screen_h;
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
        let pending = gnoblin_shell_ui::notifcenter::has_pending();
        if pending != self.last_pending {
            self.last_pending = pending;
            if let Some(p) = &self.panel {
                p.set_notif_pending(pending);
            }
            changed = true;
        }
        let notif_summary = gnoblin_shell_ui::notifcenter::summary();
        if notif_summary != self.last_notif_summary {
            self.last_notif_summary = notif_summary;
            if let Some(p) = &self.panel {
                apply_notifications(p);
            }
            changed = true;
        }

        // Follow external light/dark changes (e.g. another client's toggle).
        let dark = gnoblin_shell_ui::theme::is_dark();
        if dark != self.theme_dark.get() {
            self.theme_dark.set(dark);
            if let Some(p) = &self.panel {
                apply_theme(p);
            }
            changed = true;
        }

        let clock = topbar_clock_text(&self.clock_format);
        if clock != self.last_clock {
            if let Some(p) = &self.panel {
                p.set_clock_text(clock.clone().into());
                p.set_date_text("".into());
                apply_notifications(p);
            }
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
                push_cc_tiles(p, &self.qs_state, &self.qs_plugins.borrow());
            }
            changed = true;
        }

        // Drain to the latest QS plugin snapshot (process-driven tiles/menus).
        // Rebuild the grid from the cached built-in state so the (possibly
        // high-frequency) plugin tick doesn't re-read wpctl/D-Bus each time.
        if let Some(plugins) = self.qs_host.borrow().poll() {
            *self.qs_plugins.borrow_mut() = plugins;
            if let Some(p) = &self.panel {
                push_cc_tiles(p, &self.qs_state, &self.qs_plugins.borrow());
            }
            changed = true;
        }

        let config_mtime = file_mtime(self.config_path.as_deref());
        if config_mtime != self.config_mtime {
            self.config_mtime = config_mtime;
            // Re-spawn QS plugins to match the (possibly changed) declarations.
            let plugin_cfgs = gnoblin_shell_ui::qsplugin::load_configs(&Config::load());
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
        // When a dropdown or popout is open the whole surface must catch input so
        // an outside-click dismisses it; otherwise only the bar strip does.
        self.menu_open.get()
            || self.app_menu_open.get()
            || self.popouts.dt_open.get()
            || self.popouts.cc_open.get()
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

const WIDGET_SPACER: i32 = 0;
const WIDGET_SPRING: i32 = 1;
const WIDGET_SEPARATOR: i32 = 2;
const WIDGET_WORKSPACES: i32 = 3;
const WIDGET_FOCUSED_APP: i32 = 4;
const WIDGET_APPMENU: i32 = 5;
const WIDGET_CLOCK: i32 = 6;
const WIDGET_NOTIFICATIONS: i32 = 7;
const WIDGET_TRAY: i32 = 8;
const WIDGET_STATUS: i32 = 9;
const WIDGET_LAUNCHER: i32 = 10;

fn widget(kind: i32) -> WidgetSpec {
    WidgetSpec {
        kind,
        flex: if kind == WIDGET_SPRING { 1 } else { 0 },
        size: if kind == WIDGET_SPACER { 16 } else { 0 },
    }
}

fn parse_widget(raw: &str) -> Option<WidgetSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (name, arg) = raw
        .split_once(':')
        .map(|(name, arg)| (name.trim(), Some(arg.trim())))
        .unwrap_or((raw, None));
    let name = name.to_ascii_lowercase();
    let mut spec = match name.as_str() {
        "spacer" | "space" => widget(WIDGET_SPACER),
        "spring" | "flex" | "flexible-spacer" => widget(WIDGET_SPRING),
        "separator" | "sep" => widget(WIDGET_SEPARATOR),
        "workspaces" | "workspace" => widget(WIDGET_WORKSPACES),
        "focused-app" | "focused_app" | "app-title" | "app_title" => widget(WIDGET_FOCUSED_APP),
        "appmenu" | "global-menu" | "global_menu" => widget(WIDGET_APPMENU),
        "clock" | "datetime" => widget(WIDGET_CLOCK),
        "notifications" | "notification" | "bell" => widget(WIDGET_NOTIFICATIONS),
        "tray" | "status-notifier" | "status_notifier" => widget(WIDGET_TRAY),
        "status" | "quick-settings" | "quick_settings" | "control-centre" | "control_centre" => {
            widget(WIDGET_STATUS)
        }
        "launcher" | "search" | "spotlight" => widget(WIDGET_LAUNCHER),
        _ => return None,
    };
    if let Some(arg) = arg.and_then(|v| v.parse::<i32>().ok()) {
        if spec.kind == WIDGET_SPRING {
            spec.flex = arg.max(1);
        } else {
            spec.size = arg.max(0);
        }
    }
    Some(spec)
}

fn parse_widget_list(cfg: &Config, key: &str, fallback: &[WidgetSpec]) -> Vec<WidgetSpec> {
    let Some(raw) = cfg.get("topbar", key) else {
        return fallback.to_vec();
    };
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let parsed: Vec<WidgetSpec> = raw.split(',').filter_map(parse_widget).collect();
    if parsed.is_empty() {
        fallback.to_vec()
    } else {
        parsed
    }
}

fn topbar_layout(cfg: &Config) -> TopbarLayout {
    let default_left = vec![
        widget(WIDGET_WORKSPACES),
        widget(WIDGET_FOCUSED_APP),
        widget(WIDGET_APPMENU),
        widget(WIDGET_SPRING),
    ];
    let default_center = vec![widget(WIDGET_CLOCK)];
    let default_right = vec![
        widget(WIDGET_LAUNCHER),
        widget(WIDGET_TRAY),
        widget(WIDGET_STATUS),
    ];
    TopbarLayout {
        left: parse_widget_list(cfg, "left", &default_left),
        center: parse_widget_list(cfg, "center", &default_center),
        right: parse_widget_list(cfg, "right", &default_right),
    }
}

fn config_i32(cfg: &Config, section: &str, key: &str, fallback: i32) -> i32 {
    cfg.get(section, key)
        .and_then(parse_i32)
        .unwrap_or(fallback)
}

fn config_i32_any(cfg: &Config, section: &str, keys: &[&str], fallback: i32) -> i32 {
    keys.iter()
        .find_map(|key| cfg.get(section, key).and_then(parse_i32))
        .unwrap_or(fallback)
}

fn parse_i32(raw: &str) -> Option<i32> {
    raw.trim()
        .strip_suffix("px")
        .unwrap_or(raw.trim())
        .trim()
        .parse::<i32>()
        .ok()
}

fn parse_align(raw: Option<&str>) -> i32 {
    match raw.unwrap_or("full").trim().to_ascii_lowercase().as_str() {
        "left" | "start" => 0,
        "right" | "end" => 2,
        "center" | "centre" => 1,
        _ => 1,
    }
}

fn topbar_settings() -> TopbarSettings {
    let cfg = Config::load();
    let height = config_i32(&cfg, "topbar", "height", 34).max(1);
    let command = |key: &str, fallback: &str| {
        cfg.get("topbar", key)
            .map(str::to_string)
            .unwrap_or_else(|| fallback.to_string())
    };
    let commands = TopbarCommands {
        launcher: command("launcher", "gnoblin-launcher"),
        account: command("account", "gnome-control-center users"),
        settings: command("control_centre", "gnome-control-center"),
        power: command("power", "gnoblin-power-menu"),
    };
    TopbarSettings {
        commands,
        layout: topbar_layout(&cfg),
        geometry: TopbarGeometry {
            width: config_i32(&cfg, "topbar", "width", 0).max(0),
            align: parse_align(cfg.get("topbar", "align")),
            offset_x: config_i32_any(&cfg, "topbar", &["offset-x", "offset_x", "x"], 0),
            padding_left: config_i32_any(&cfg, "topbar", &["padding-left", "padding_left"], 12)
                .max(0),
            padding_right: config_i32_any(&cfg, "topbar", &["padding-right", "padding_right"], 12)
                .max(0),
            clock_padding: config_i32_any(&cfg, "topbar", &["clock-padding", "clock_padding"], 12)
                .max(0),
            status_padding: config_i32_any(
                &cfg,
                "topbar",
                &["status-padding", "status_padding"],
                10,
            )
            .max(0),
            status_icon_gap: config_i32_any(
                &cfg,
                "topbar",
                &["status-icon-gap", "status_icon_gap"],
                10,
            )
            .max(0),
            cc_offset_x: config_i32_any(
                &cfg,
                "topbar",
                &["quick-settings-offset-x", "quick_settings_offset_x"],
                6,
            ),
            cc_offset_y: config_i32_any(
                &cfg,
                "topbar",
                &["quick-settings-offset-y", "quick_settings_offset_y"],
                -4,
            ),
        },
        height,
        exclusive_zone: config_i32(&cfg, "topbar", "exclusive_zone", height).max(0),
        clock_format: command("clock-format", DEFAULT_CLOCK_FORMAT),
    }
}

fn file_mtime(path: Option<&Path>) -> Option<SystemTime> {
    path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
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
    gnoblin_shell_ui::quicksettings::spawn(qtx);

    let (menu_tx, menu_cmd_rx) = std::sync::mpsc::channel();
    let (menu_reply_tx, menu_rx) = std::sync::mpsc::channel();
    appmenu::spawn(menu_cmd_rx, menu_reply_tx);
    let config_path = Config::path();
    let config_mtime = file_mtime(config_path.as_deref());
    let settings = topbar_settings();

    // Spawn the command/process-driven QS plugin host from the declared
    // [qs-plugin.*] / [providers] config.
    let qs_host = Rc::new(RefCell::new(gnoblin_shell_ui::qsplugin::Host::spawn(
        gnoblin_shell_ui::qsplugin::load_configs(&Config::load()),
    )));

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
        },
        Box::new(TopBarApp {
            panel: None,
            last_clock: String::new(),
            tray_rx: urx,
            tray_tx: ctx,
            endpoints: Rc::new(RefCell::new(HashMap::new())),
            win_rx: wrx,
            qs_rx: qrx,
            qs_host,
            qs_state: gnoblin_shell_ui::quicksettings::QuickState::default(),
            qs_plugins: Rc::new(RefCell::new(Vec::new())),
            last_focused: String::new(),
            last_workspaces: (0, 0),
            last_pending: false,
            last_notif_summary: gnoblin_shell_ui::notifcenter::Summary::default(),
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
            bar_height: settings.height,
            bar_x: Cell::new(0),
            bar_w: Cell::new(1280),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_config(text: &str, f: impl FnOnce()) {
        let _guard = env_lock().lock().unwrap();
        let old = std::env::var("GNOBLIN_CONFIG").ok();
        let path = std::env::temp_dir().join(format!(
            "gnoblin-topbar-test-{}-{}.conf",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        std::fs::write(&path, text).unwrap();
        std::env::set_var("GNOBLIN_CONFIG", &path);

        f();

        if let Some(old) = old {
            std::env::set_var("GNOBLIN_CONFIG", old);
        } else {
            std::env::remove_var("GNOBLIN_CONFIG");
        }
        let _ = std::fs::remove_file(path);
    }

    fn kinds(widgets: &[WidgetSpec]) -> Vec<i32> {
        widgets.iter().map(|w| w.kind).collect()
    }

    #[test]
    fn missing_widget_zones_use_default_desktop_layout() {
        with_config("", || {
            let cfg = Config::load();
            let layout = topbar_layout(&cfg);

            assert_eq!(
                kinds(&layout.left),
                vec![
                    WIDGET_WORKSPACES,
                    WIDGET_FOCUSED_APP,
                    WIDGET_APPMENU,
                    WIDGET_SPRING,
                ]
            );
            assert_eq!(kinds(&layout.center), vec![WIDGET_CLOCK]);
            assert_eq!(
                kinds(&layout.right),
                vec![WIDGET_LAUNCHER, WIDGET_TRAY, WIDGET_STATUS]
            );
        });
    }

    #[test]
    fn invalid_nonempty_widget_zones_fall_back_to_defaults() {
        with_config(
            "[topbar]\n\
             left = clcok\n\
             center = misspelled\n\
             right = nope\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert_eq!(
                    kinds(&layout.left),
                    vec![
                        WIDGET_WORKSPACES,
                        WIDGET_FOCUSED_APP,
                        WIDGET_APPMENU,
                        WIDGET_SPRING,
                    ]
                );
                assert_eq!(kinds(&layout.center), vec![WIDGET_CLOCK]);
                assert_eq!(
                    kinds(&layout.right),
                    vec![WIDGET_LAUNCHER, WIDGET_TRAY, WIDGET_STATUS]
                );
            },
        );
    }

    #[test]
    fn topbar_settings_reads_clock_and_geometry_config() {
        with_config(
            "[topbar]\n\
             clock-format = %x %X\n\
             width = 900px\n\
             align = right\n\
             offset-x = -6px\n\
             padding-left = 10px\n\
             padding-right = 6px\n\
             clock-padding = 18px\n\
             status-padding = 3px\n\
             status-icon-gap = 4px\n\
             quick-settings-offset-x = 8px\n\
             quick-settings-offset-y = -6px\n",
            || {
                let settings = topbar_settings();
                assert_eq!(settings.clock_format, "%x %X");
                assert_eq!(settings.geometry.width, 900);
                assert_eq!(settings.geometry.align, 2);
                assert_eq!(settings.geometry.offset_x, -6);
                assert_eq!(settings.geometry.padding_left, 10);
                assert_eq!(settings.geometry.padding_right, 6);
                assert_eq!(settings.geometry.clock_padding, 18);
                assert_eq!(settings.geometry.status_padding, 3);
                assert_eq!(settings.geometry.status_icon_gap, 4);
                assert_eq!(settings.geometry.cc_offset_x, 8);
                assert_eq!(settings.geometry.cc_offset_y, -6);
            },
        );
    }

    #[test]
    fn notification_age_uses_locale_time_not_english_relative_text() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap();
        let recent = notification_age_label(now.saturating_sub(90));
        let older = notification_age_label(now.saturating_sub(172_800));

        for label in [recent, older] {
            let lower = label.to_ascii_lowercase();
            assert!(!lower.contains("ago"));
            assert!(!lower.contains("yesterday"));
            assert_ne!(lower, "now");
        }
    }

    #[test]
    fn explicit_empty_widget_zones_stay_empty() {
        with_config(
            "[topbar]\n\
             left =\n\
             center =\n\
             right =\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert!(layout.left.is_empty());
                assert!(layout.center.is_empty());
                assert!(layout.right.is_empty());
            },
        );
    }

    #[test]
    fn mixed_invalid_widget_lists_keep_valid_entries() {
        with_config(
            "[topbar]\n\
             left = nonsense, status, launcher, spring:3\n",
            || {
                let cfg = Config::load();
                let layout = topbar_layout(&cfg);

                assert_eq!(
                    kinds(&layout.left),
                    vec![WIDGET_STATUS, WIDGET_LAUNCHER, WIDGET_SPRING]
                );
                assert_eq!(layout.left[2].flex, 3);
            },
        );
    }
}
