use std::path::PathBuf;
use std::process::{Command, Stdio};

use gnoblin_desktop::{desktop_actions, launch_desktop_action, launch_desktop_app, resolve_desktop_id, DesktopAction};

pub const ITEM_ALL_WINDOWS: i32 = 1;
pub const ITEM_OPEN: i32 = 2;
pub const ITEM_PIN: i32 = 3;
pub const ITEM_APP_DETAILS: i32 = 4;
pub const ITEM_QUIT: i32 = 5;
pub const ITEM_DESKTOP_ACTION_BASE: i32 = 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppMenuEntry {
    pub id: i32,
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
}

pub fn pins_path() -> Option<PathBuf> {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(dir.join("gnoblin").join("dock-favorites"))
}

pub fn favorite_ids_from_config() -> Vec<String> {
    const DEFAULT: &[&str] = &[
        "firefox",
        "org.gnome.Nautilus",
        "org.gnome.TextEditor",
        "org.gnome.Calculator",
        "foot",
        "org.gnome.Settings",
    ];

    if let Some(text) = pins_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        let ids: Vec<String> = text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        return ids;
    }

    match gnoblin_core::config::Config::load().get("dock", "favorites") {
        Some(list) => list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        None => DEFAULT.iter().map(|s| s.to_string()).collect(),
    }
}

pub fn is_pinned(app_id: &str) -> bool {
    favorite_ids_from_config()
        .iter()
        .any(|id| crate::shell::matches(id, app_id))
}

pub fn toggle_pin(app_id: &str) {
    let mut ids = favorite_ids_from_config();
    if let Some(pos) = ids.iter().position(|id| crate::shell::matches(id, app_id)) {
        ids.remove(pos);
    } else {
        ids.push(app_id.to_string());
    }
    save_favorite_ids(&ids);
}

fn save_favorite_ids(ids: &[String]) {
    let Some(path) = pins_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, ids.join("\n") + "\n");
}

fn row(id: i32, label: impl Into<String>) -> AppMenuEntry {
    AppMenuEntry {
        id,
        label: label.into(),
        separator: false,
        enabled: true,
    }
}

fn sep() -> AppMenuEntry {
    AppMenuEntry {
        id: -1,
        label: String::new(),
        separator: true,
        enabled: false,
    }
}

fn has_new_window_action(actions: &[DesktopAction]) -> bool {
    actions.iter().any(|a| {
        let id = a.id.to_ascii_lowercase();
        let name = a.name.to_ascii_lowercase();
        id.contains("newwindow")
            || id.contains("new-window")
            || name == "new window"
            || name == "new-window"
    })
}

pub fn build(app_id: &str, running: bool, pinned: bool) -> Vec<AppMenuEntry> {
    let mut items = Vec::new();
    let actions = desktop_actions(app_id);

    if running {
        items.push(row(ITEM_ALL_WINDOWS, "All Windows >"));
        items.push(sep());
    }

    if actions.is_empty() || !has_new_window_action(&actions) {
        items.push(row(ITEM_OPEN, if running { "New Window" } else { "Open" }));
    }
    for (idx, action) in actions.iter().enumerate() {
        items.push(row(
            ITEM_DESKTOP_ACTION_BASE + idx as i32,
            action.name.clone(),
        ));
    }

    items.push(sep());
    items.push(row(
        ITEM_PIN,
        if pinned {
            "Unpin from Dock"
        } else {
            "Pin to Dock"
        },
    ));
    items.push(sep());
    items.push(row(ITEM_APP_DETAILS, "App Details"));
    if running {
        items.push(sep());
        items.push(row(ITEM_QUIT, "Quit"));
    }
    items
}

pub fn activate(app_id: &str, id: i32) -> bool {
    match id {
        ITEM_ALL_WINDOWS => crate::shell::activate_app(app_id),
        ITEM_OPEN => launch_desktop_app(app_id),
        ITEM_PIN => {
            toggle_pin(app_id);
            return true;
        }
        ITEM_APP_DETAILS => open_app_details(app_id),
        ITEM_QUIT => crate::shell::close_app_windows(app_id),
        id if id >= ITEM_DESKTOP_ACTION_BASE => {
            let idx = (id - ITEM_DESKTOP_ACTION_BASE) as usize;
            if let Some(action) = desktop_actions(app_id).get(idx) {
                let _ = launch_desktop_action(app_id, &action.id);
            }
        }
        _ => {}
    }
    false
}

fn open_app_details(app_id: &str) {
    let desktop_id = resolve_desktop_id(app_id).unwrap_or_else(|| app_id.to_string());
    let arg = format!("{desktop_id}.desktop");
    match Command::new("gnome-software")
        .args(["--details", &arg])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("gnoblin: failed to open app details for '{app_id}': {e}"),
    }
}
