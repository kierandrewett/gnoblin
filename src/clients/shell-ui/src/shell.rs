//! Polls `dev.gnoblin.Shell` for window state on a background zbus thread, so the
//! dock can show running/focused indicators and the topbar can show the focused
//! app. Channels the state into the bar's render tick (same pattern as the tray).

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::appmenu::{self, BarEntry, MenuAddr};

/// Snapshot of window state from the compositor.
#[derive(Clone, PartialEq, Default)]
pub struct WindowState {
    /// app-ids of all current windows.
    pub running: Vec<String>,
    /// Per-app open window counts, in the same first-seen order as `running`.
    pub window_counts: Vec<(String, u32)>,
    /// app-id of the focused window (empty if none).
    pub focused: String,
    /// The focused window's GTK menu export (empty if none).
    pub menu_addr: MenuAddr,
    /// Top-level entries of the focused window's global menu (File / Edit / …).
    pub menu_bar: Vec<BarEntry>,
    /// Active workspace index (0-based) and the total workspace count.
    pub active_workspace: u32,
    pub n_workspaces: u32,
}

/// One row of `dev.gnoblin.Shell.ListWindows`: (id, title, app-id, focused, minimized).
type WindowRow = (u64, String, String, bool, bool);

const WINDOW_STATE_POLL_INTERVAL: Duration = Duration::from_millis(120);
const MENU_BAR_REFRESH_INTERVAL: Duration = Duration::from_millis(1200);

#[zbus::proxy(
    interface = "dev.gnoblin.Shell",
    default_service = "dev.gnoblin.Shell",
    default_path = "/dev/gnoblin/Shell"
)]
trait Shell {
    fn list_windows(&self) -> zbus::Result<Vec<WindowRow>>;
    /// (kind, bus_name, app_object_path, window_object_path, menubar_object_path,
    /// app_menu_object_path) of the focused window's menu export.
    fn get_active_window_menu(
        &self,
    ) -> zbus::Result<(String, String, String, String, String, String)>;
    fn activate_window(&self, id: u64) -> zbus::Result<()>;
    fn dispatch(&self, action: &str, arg: &str) -> zbus::Result<()>;
    /// (active index 0-based, workspace count).
    fn workspace_state(&self) -> zbus::Result<(u32, u32)>;
}

/// Run a gnoblin action against a specific window: focus it (so the action's
/// implicit target is right), then dispatch the verb. Used by the window menu.
/// Synchronous one-shot — fine for a client that exits right after.
pub fn dispatch_window_action(window: u64, action: &str, arg: &str) {
    let action = action.to_string();
    let arg = arg.to_string();
    let _ = zbus::block_on(async move {
        let conn = zbus::Connection::session().await?;
        let proxy = ShellProxy::new(&conn).await?;
        if window != 0 {
            let _ = proxy.activate_window(window).await;
        }
        proxy.dispatch(&action, &arg).await
    });
}

/// Close every window belonging to `app_id` (fuzzy-matched). Used by the dock's
/// right-click "Close" — activate each match, then dispatch the close verb.
/// Focus the first window belonging to `app_id` (so a dock click on a running
/// app raises it instead of launching a duplicate). No-op if none match.
pub fn activate_app(app_id: &str) {
    let app_id = app_id.to_string();
    let _ = zbus::block_on(async move {
        let conn = zbus::Connection::session().await?;
        let proxy = ShellProxy::new(&conn).await?;
        let wins = proxy.list_windows().await.unwrap_or_default();
        if let Some(w) = wins.iter().find(|(_, _, app, _, _)| matches(&app_id, app)) {
            let _ = proxy.activate_window(w.0).await;
        }
        Ok::<(), zbus::Error>(())
    });
}

pub fn close_app_windows(app_id: &str) {
    let app_id = app_id.to_string();
    let _ = zbus::block_on(async move {
        let conn = zbus::Connection::session().await?;
        let proxy = ShellProxy::new(&conn).await?;
        let wins = proxy.list_windows().await.unwrap_or_default();
        for (id, _title, app, _focused, _min) in wins {
            if matches(&app_id, &app) {
                let _ = proxy.activate_window(id).await;
                let _ = proxy.dispatch("close", "").await;
            }
        }
        Ok::<(), zbus::Error>(())
    });
}

/// A fixed menu address from `GNOBLIN_APPMENU_TEST`, used to validate the
/// appmenu pipeline without a real focused window. Accepted forms:
/// `bus;app_path;menubar_path[;win_path]`, `gtk;bus;app_path;menubar_path[;win_path]`,
/// or `dbusmenu;bus;object_path`.
fn test_menu_addr() -> Option<MenuAddr> {
    let spec = std::env::var("GNOBLIN_APPMENU_TEST").ok()?;
    let parts: Vec<&str> = spec.split(';').collect();
    if parts.first() == Some(&"dbusmenu") || parts.first() == Some(&"kde") {
        if parts.len() < 3 {
            return None;
        }
        return Some(MenuAddr {
            kind: "dbusmenu".to_string(),
            bus: parts[1].to_string(),
            app_path: String::new(),
            win_path: String::new(),
            menubar_path: parts[2].to_string(),
        });
    }
    let offset = if parts.first() == Some(&"gtk") { 1 } else { 0 };
    if parts.len() < offset + 3 {
        return None;
    }
    Some(MenuAddr {
        kind: "gtk".to_string(),
        bus: parts[offset].to_string(),
        app_path: parts[offset + 1].to_string(),
        win_path: parts
            .get(offset + 3)
            .map(|s| s.to_string())
            .unwrap_or_default(),
        menubar_path: parts[offset + 2].to_string(),
    })
}

/// True if a dock favourite id and a window app-id refer to the same app
/// (case-insensitive; tolerates `org.gnome.Foo` vs `foo` style differences).
pub fn matches(fav: &str, win_app: &str) -> bool {
    if fav.is_empty() || win_app.is_empty() {
        return false;
    }
    let a = fav.to_lowercase();
    let b = win_app.to_lowercase();
    if a == b {
        return true;
    }
    let tail = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    tail(&a) == tail(&b) || a.contains(&b) || b.contains(&a)
}

pub fn spawn(updates: Sender<WindowState>) {
    std::thread::spawn(move || {
        if let Err(e) = zbus::block_on(run(updates)) {
            eprintln!("gnoblin-topbar: shell poll stopped: {e}");
        }
    });
}

async fn run(updates: Sender<WindowState>) -> zbus::Result<()> {
    let conn = zbus::Connection::session().await?;
    let proxy = ShellProxy::new(&conn).await?;
    let forced_menu = test_menu_addr();
    let mut last = WindowState::default();
    let mut last_menu_bar_refresh = Instant::now() - MENU_BAR_REFRESH_INTERVAL;
    loop {
        if let Ok(wins) = proxy.list_windows().await {
            let mut state = WindowState::default();
            for (_id, _title, app, focused, _min) in &wins {
                if !app.is_empty() {
                    if !state.running.iter().any(|a| a == app) {
                        state.running.push(app.clone());
                        state.window_counts.push((app.clone(), 1));
                    } else if let Some((_, count)) =
                        state.window_counts.iter_mut().find(|(id, _)| id == app)
                    {
                        *count += 1;
                    }
                }
                if *focused {
                    state.focused = app.clone();
                }
            }

            // The focused window's GTK global menu, if any.
            let addr = match &forced_menu {
                Some(a) => a.clone(),
                None => match proxy.get_active_window_menu().await {
                    Ok((kind, bus, app_path, win_path, menubar_path, _app_menu)) => MenuAddr {
                        kind,
                        bus,
                        app_path,
                        win_path,
                        menubar_path,
                    },
                    Err(_) => MenuAddr::default(),
                },
            };
            if !addr.is_empty() {
                let menu_due = last_menu_bar_refresh.elapsed() >= MENU_BAR_REFRESH_INTERVAL;
                let menu_addr_changed = addr != last.menu_addr;
                state.menu_bar = if menu_addr_changed || menu_due {
                    last_menu_bar_refresh = Instant::now();
                    appmenu::fetch_bar(&conn, &addr).await
                } else {
                    last.menu_bar.clone()
                };
                state.menu_addr = addr;
            }

            if let Ok((active, count)) = proxy.workspace_state().await {
                state.active_workspace = active;
                state.n_workspaces = count;
            }

            if state != last {
                let _ = updates.send(state.clone());
                last = state;
            }
        }
        async_io::Timer::after(WINDOW_STATE_POLL_INTERVAL).await;
    }
}
