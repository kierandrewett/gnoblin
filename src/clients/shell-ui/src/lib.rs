//! Shared runner: render a Slint component as a wlr-layer-shell client.
//!
//! Slint normally renders through its own winit window. Here we render the Slint
//! scene with the FemtoVG OpenGL renderer onto an EGL surface created from a
//! smithay-client-toolkit layer surface — so de's Slint panels run as ordinary
//! layer-shell clients on gnoblin. GPU rendering keeps Slint's blur/drop-shadow.
//! Slint's timer/animation pump and Wayland frame callbacks drive redraw +
//! per-app refresh; pointer events are forwarded into Slint. The bar-specific bits
//! (which component, where it anchors, what data it shows) live in a `BarApp`
//! impl in each binary.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::{c_void, CStr};
use std::io::ErrorKind;
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use khronos_egl as egl;
use slint::platform::femtovg_renderer::{FemtoVGRenderer, OpenGLInterface};
use slint::platform::{Platform, PointerEventButton, Renderer, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, LogicalSize, PhysicalSize};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
};
use wayland_client::{
    backend::WaylandError,
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy, QueueHandle,
};

const CONFIGURE_RENDER_DELAY: Duration = Duration::from_millis(50);
const IDLE_DISPATCH_TIMEOUT: Duration = Duration::from_millis(200);
const APP_TICK_INTERVAL: Duration = Duration::from_millis(100);

pub type RuntimeError = Box<dyn Error + Send + Sync>;

pub fn runtime_error(message: impl Into<String>) -> RuntimeError {
    Box::new(std::io::Error::other(message.into()))
}

/// Last-modified time of a file, or `None` if the path is `None` or unreadable.
/// Layer-shell clients poll this to live-reload their config/pins on change.
pub fn file_mtime(path: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
    path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

pub mod app_context_menu;
pub mod appmenu;
pub mod args;
pub mod config;
pub mod datetime;
pub mod dnd;
pub mod nightlight;
pub mod notifcenter;
pub mod prefs;
pub mod qsplugin;
pub mod quicksettings;
pub mod shell;
pub mod theme;
pub mod tray;

pub use args::ClientArgs;

/// Apply the light/dark preference (mode + shell chrome) to a client's Slint
/// `Theme` global — replaces the per-client `apply_theme` copies. `$component` is
/// the client's top-level component; its generated `TokenMode`/`Theme` resolve at
/// the call site.
#[macro_export]
macro_rules! apply_shell_theme {
    ($component:expr) => {{
        let dark = $crate::theme::is_dark();
        let theme = $component.global::<Theme>();
        theme.set_mode(if dark {
            TokenMode::Dark
        } else {
            TokenMode::Light
        });
        $crate::apply_shell_chrome_to_theme!(theme, $crate::theme::shell_chrome(dark));
    }};
}

#[macro_export]
macro_rules! apply_shell_chrome_to_theme {
    ($theme:expr, $chrome:expr) => {{
        let theme = $theme;
        let chrome = $chrome;
        theme.set_panel_bg(chrome.panel_bg);
        theme.set_panel_fg(chrome.panel_fg);
        theme.set_panel_border_bottom(chrome.panel_border_bottom);
        theme.set_dock_bg(chrome.dock_bg);
        theme.set_dock_border(chrome.dock_border);
        theme.set_dock_highlight(chrome.dock_highlight);
        theme.set_surface_bg(chrome.surface_bg);
        theme.set_surface_raised_bg(chrome.surface_raised_bg);
        theme.set_surface_hover_bg(chrome.surface_hover_bg);
        theme.set_surface_active_bg(chrome.surface_active_bg);
        theme.set_critical_accent(chrome.critical_accent);
        theme.set_wallpaper_fallback_bg(chrome.wallpaper_fallback_bg);
        theme.set_surface_border(chrome.surface_border);
        theme.set_text_primary(chrome.text_primary);
        theme.set_text_secondary(chrome.text_secondary);
        theme.set_menu_bg(chrome.menu_bg);
        theme.set_menu_border(chrome.menu_border);
        theme.set_menu_highlight(chrome.menu_highlight);
        theme.set_chrome_shadow(chrome.chrome_shadow);
        theme.set_chrome_shadow_source(chrome.chrome_shadow_source);
        theme.set_dock_corner_radius(chrome.dock_corner_radius);
        theme.set_menu_corner_radius(chrome.menu_corner_radius);
        theme.set_popout_corner_radius(chrome.popout_corner_radius);
        theme.set_tooltip_corner_radius(chrome.tooltip_corner_radius);
        theme.set_control_corner_radius(chrome.control_corner_radius);
        theme.set_chrome_hairline_width(chrome.chrome_hairline_width);
        theme.set_chrome_highlight_height(chrome.chrome_highlight_height);
        theme.set_dock_shadow_blur(chrome.dock_shadow_blur);
        theme.set_dock_shadow_offset_y(chrome.dock_shadow_offset_y);
        theme.set_menu_shadow_blur(chrome.menu_shadow_blur);
        theme.set_menu_shadow_offset_y(chrome.menu_shadow_offset_y);
        theme.set_popout_shadow_blur(chrome.popout_shadow_blur);
        theme.set_popout_shadow_offset_y(chrome.popout_shadow_offset_y);
        theme.set_tooltip_shadow_blur(chrome.tooltip_shadow_blur);
        theme.set_tooltip_shadow_offset_y(chrome.tooltip_shadow_offset_y);
        theme.set_control_shadow_blur(chrome.control_shadow_blur);
        theme.set_control_shadow_offset_y(chrome.control_shadow_offset_y);
        theme.set_window_shadow_blur(chrome.window_shadow_blur);
        theme.set_window_shadow_offset_y(chrome.window_shadow_offset_y);
        theme.set_shell_font_family(chrome.font_family.clone().into());
    }};
}

#[macro_export]
macro_rules! apply_shell_motion_to_theme {
    ($theme:expr, $motion:expr) => {{
        let theme = $theme;
        let motion = $motion;
        let mut changed = false;
        macro_rules! set_f32 {
            ($get:ident, $set:ident, $value:expr) => {{
                let value = $value;
                if (theme.$get() - value).abs() > f32::EPSILON {
                    theme.$set(value);
                    changed = true;
                }
            }};
        }
        macro_rules! set_i32 {
            ($get:ident, $set:ident, $value:expr) => {{
                let value = $value;
                if theme.$get() != value {
                    theme.$set(value);
                    changed = true;
                }
            }};
        }

        set_f32!(get_motion_scale, set_motion_scale, motion.scale);
        set_f32!(get_motion_fast_ms, set_motion_fast_ms, motion.fast_ms);
        set_f32!(get_motion_medium_ms, set_motion_medium_ms, motion.medium_ms);
        set_f32!(
            get_motion_overlay_ms,
            set_motion_overlay_ms,
            motion.overlay_ms
        );
        set_f32!(
            get_motion_overlay_open_ms,
            set_motion_overlay_open_ms,
            motion.overlay_open_ms
        );
        set_f32!(
            get_motion_overlay_close_ms,
            set_motion_overlay_close_ms,
            motion.overlay_close_ms
        );
        set_f32!(get_motion_fade_ms, set_motion_fade_ms, motion.fade_ms);
        set_f32!(get_motion_page_ms, set_motion_page_ms, motion.page_ms);
        set_f32!(
            get_motion_overlay_slide_value,
            set_motion_overlay_slide_value,
            motion.overlay_slide
        );
        set_f32!(
            get_motion_overlay_scale_from_value,
            set_motion_overlay_scale_from_value,
            motion.overlay_scale_from
        );

        set_i32!(
            get_motion_fast_style,
            set_motion_fast_style,
            motion.fast_style
        );
        set_i32!(
            get_motion_medium_style,
            set_motion_medium_style,
            motion.medium_style
        );
        set_i32!(
            get_motion_overlay_style,
            set_motion_overlay_style,
            motion.overlay_style
        );
        set_i32!(
            get_motion_ease_out_style,
            set_motion_ease_out_style,
            motion.ease_out_style
        );
        set_i32!(
            get_motion_ease_in_style,
            set_motion_ease_in_style,
            motion.ease_in_style
        );
        set_i32!(
            get_motion_ease_in_out_style,
            set_motion_ease_in_out_style,
            motion.ease_in_out_style
        );
        set_i32!(
            get_motion_overlay_open_style,
            set_motion_overlay_open_style,
            motion.overlay_open_style
        );
        set_i32!(
            get_motion_overlay_close_style,
            set_motion_overlay_close_style,
            motion.overlay_close_style
        );
        set_i32!(
            get_motion_fade_style,
            set_motion_fade_style,
            motion.fade_style
        );
        set_i32!(
            get_motion_page_style,
            set_motion_page_style,
            motion.page_style
        );

        changed
    }};
}

fn applications_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let data_home = xdg_data_home();
    if let Some(d) = data_home {
        dirs.push(d.join("applications"));
    }
    for d in xdg_data_dirs() {
        dirs.push(d.join("applications"));
    }
    dirs
}

fn xdg_data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local/share"))
                .filter(|p| p.is_absolute())
        })
}

fn default_xdg_data_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}

fn xdg_data_dirs() -> Vec<PathBuf> {
    let Some(raw_dirs) = std::env::var_os("XDG_DATA_DIRS").filter(|s| !s.is_empty()) else {
        return default_xdg_data_dirs();
    };
    let dirs: Vec<PathBuf> = std::env::split_paths(&raw_dirs)
        .filter(|path| path.is_absolute())
        .collect();
    if dirs.is_empty() {
        default_xdg_data_dirs()
    } else {
        dirs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopEntryFile {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopExec {
    program: String,
    args: Vec<String>,
    terminal: bool,
    working_dir: Option<PathBuf>,
}

struct DesktopExecMetadata {
    name: Option<String>,
    icon: Option<String>,
    desktop_file: String,
}

#[derive(Clone, Debug, Default)]
struct DesktopActionDraft {
    name: Option<String>,
    icon: Option<String>,
    exec: Option<String>,
    terminal: bool,
    working_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DesktopActionExec {
    action: DesktopAction,
    exec: DesktopExec,
}

fn desktop_id_for_path(applications_dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(applications_dir).ok()?;
    let mut parts = Vec::new();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        parts.push(part.to_string_lossy().to_string());
    }
    let id = parts.join("-");
    id.strip_suffix(".desktop").map(|id| id.to_string())
}

fn collect_desktop_entries(
    applications_dir: &Path,
    dir: &Path,
    entries: &mut Vec<DesktopEntryFile>,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read_dir.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_desktop_entries(applications_dir, &path, entries);
        } else if path.extension().map(|e| e == "desktop").unwrap_or(false) {
            if let Some(id) = desktop_id_for_path(applications_dir, &path) {
                entries.push(DesktopEntryFile { id, path });
            }
        }
    }
}

/// Installed desktop entries in XDG data-dir precedence order. IDs are returned
/// without the `.desktop` suffix, matching the argument accepted by `gtk-launch`.
pub fn installed_desktop_entries() -> Vec<DesktopEntryFile> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for dir in applications_dirs() {
        let mut dir_entries = Vec::new();
        collect_desktop_entries(&dir, &dir, &mut dir_entries);
        for entry in dir_entries {
            if seen.insert(entry.id.clone()) {
                entries.push(entry);
            }
        }
    }
    entries
}

fn desktop_id_tail(id: &str) -> &str {
    let dot = id.rfind('.');
    let dash = id.rfind('-');
    match (dot, dash) {
        (Some(a), Some(b)) => &id[a.max(b) + 1..],
        (Some(i), None) | (None, Some(i)) => &id[i + 1..],
        (None, None) => id,
    }
}

fn desktop_id_match_rank(candidate: &str, want: &str) -> Option<usize> {
    if candidate.rsplit('.').next() == Some(want) {
        return Some(0);
    }
    if desktop_id_tail(candidate) == want {
        return Some(1);
    }
    if candidate.ends_with(&format!(".{want}")) {
        return Some(2);
    }
    if candidate.ends_with(&format!("-{want}")) {
        return Some(3);
    }
    None
}

/// Resolve a user-facing/dock favourite id to the installed `.desktop` id that
/// `gtk-launch` expects. Exact ids win; otherwise tolerate legacy short ids like
/// `firefox` for an installed `org.mozilla.firefox.desktop`.
pub fn resolve_desktop_id(id: &str) -> Option<String> {
    let id = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    if id.is_empty() {
        return None;
    }
    let ids: Vec<String> = installed_desktop_entries()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    if ids.iter().any(|candidate| candidate == id) {
        return Some(id.to_string());
    }
    let want = id.to_lowercase();
    ids.into_iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let c = candidate.to_lowercase();
            desktop_id_match_rank(&c, &want).map(|rank| (rank, index, candidate))
        })
        .min_by_key(|(rank, index, _)| (*rank, *index))
        .map(|(_, _, candidate)| candidate)
}

/// Launch an installed desktop application detached from the shell client.
/// `gtk-launch` exits quickly but still needs reaping; otherwise repeated dock
/// clicks leave zombies until the dock/launcher exits.
pub fn launch_desktop_app(id: &str) {
    let requested = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    if requested.is_empty() {
        return;
    }
    let launch_id = match resolve_desktop_id(requested) {
        Some(resolved) => {
            if resolved != requested {
                eprintln!("gnoblin: desktop app '{requested}' resolved to '{resolved}'");
            }
            resolved
        }
        None => {
            eprintln!("gnoblin: desktop app '{requested}' not found in XDG application dirs");
            requested.to_string()
        }
    };
    if let Some(entry) = installed_desktop_entry(&launch_id) {
        if desktop_entry_dbus_activatable(&entry.path) {
            if let Some(exec) = desktop_exec(&entry.path) {
                if spawn_desktop_exec_command(&launch_id, exec) {
                    return;
                }
            }
        }
    }
    let fallback_id = launch_id.clone();
    match Command::new("gtk-launch")
        .arg(&launch_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    eprintln!("gnoblin: gtk-launch '{launch_id}' exited with {status}");
                    spawn_desktop_exec_fallback(&fallback_id);
                }
                Err(e) => {
                    eprintln!("gnoblin: gtk-launch '{launch_id}' wait failed: {e}");
                    spawn_desktop_exec_fallback(&fallback_id);
                }
            });
        }
        Err(e) => {
            eprintln!("gnoblin: failed to spawn gtk-launch '{launch_id}': {e}");
            spawn_desktop_exec_fallback(&fallback_id);
        }
    }
}

pub fn desktop_actions(id: &str) -> Vec<DesktopAction> {
    desktop_action_execs(id)
        .into_iter()
        .map(|a| a.action)
        .collect()
}

pub fn launch_desktop_action(id: &str, action_id: &str) -> bool {
    let Some(action) = desktop_action_execs(id)
        .into_iter()
        .find(|a| a.action.id == action_id)
    else {
        eprintln!("gnoblin: desktop action '{action_id}' not found for '{id}'");
        return false;
    };
    spawn_desktop_exec_command(id, action.exec)
}

fn desktop_action_execs(id: &str) -> Vec<DesktopActionExec> {
    let requested = id.trim().strip_suffix(".desktop").unwrap_or(id.trim());
    let Some(launch_id) = resolve_desktop_id(requested) else {
        return Vec::new();
    };
    let Some(entry) = installed_desktop_entry(&launch_id) else {
        return Vec::new();
    };
    parse_desktop_actions(&entry.path)
}

fn parse_desktop_actions(path: &Path) -> Vec<DesktopActionExec> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut action_order: Vec<String> = Vec::new();
    let mut action_seen = HashSet::new();
    let mut drafts: HashMap<String, DesktopActionDraft> = HashMap::new();
    let mut current_action: Option<String> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_action = line
                .strip_prefix("[Desktop Action ")
                .and_then(|s| s.strip_suffix(']'))
                .map(str::to_string);
            if let Some(id) = &current_action {
                if action_seen.insert(id.clone()) {
                    action_order.push(id.clone());
                }
                drafts.entry(id.clone()).or_default();
            }
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if current_action.is_none() {
            if key == "Actions" {
                for id in value.split(';').map(str::trim).filter(|id| !id.is_empty()) {
                    if action_seen.insert(id.to_string()) {
                        action_order.push(id.to_string());
                    }
                    drafts.entry(id.to_string()).or_default();
                }
            }
            continue;
        }

        let Some(id) = current_action.as_ref().cloned() else {
            continue;
        };
        let draft = drafts.entry(id).or_default();
        match key {
            "Name" if draft.name.is_none() => {
                draft.name = Some(unescape_desktop_string(value));
            }
            "Icon" if draft.icon.is_none() => {
                draft.icon = Some(unescape_desktop_string(value));
            }
            "Exec" if draft.exec.is_none() => {
                draft.exec = Some(value.to_string());
            }
            "Terminal" => draft.terminal = value == "true",
            "Path" if draft.working_dir.is_none() => {
                let path = unescape_desktop_string(value);
                if !path.is_empty() {
                    draft.working_dir = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }

    action_order
        .into_iter()
        .filter_map(|id| {
            let draft = drafts.remove(&id)?;
            let name = draft.name?;
            let exec = parse_desktop_exec(
                draft.exec.as_deref()?,
                &DesktopExecMetadata {
                    name: Some(name.clone()),
                    icon: draft.icon,
                    desktop_file: path.display().to_string(),
                },
                draft.terminal,
                draft.working_dir,
            )?;
            Some(DesktopActionExec {
                action: DesktopAction { id, name },
                exec,
            })
        })
        .collect()
}

fn installed_desktop_entry(id: &str) -> Option<DesktopEntryFile> {
    installed_desktop_entries()
        .into_iter()
        .find(|entry| entry.id == id)
}

fn desktop_entry_dbus_activatable(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "DBusActivatable" {
            return value.trim() == "true";
        }
    }
    false
}

fn desktop_exec(path: &Path) -> Option<DesktopExec> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut exec = None;
    let mut name = None;
    let mut icon = None;
    let mut terminal = false;
    let mut working_dir = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Exec" if exec.is_none() => exec = Some(value.trim().to_string()),
            "Name" if name.is_none() => name = Some(unescape_desktop_string(value.trim())),
            "Icon" if icon.is_none() => icon = Some(unescape_desktop_string(value.trim())),
            "Terminal" => terminal = value.trim() == "true",
            "Path" if working_dir.is_none() => {
                let path = unescape_desktop_string(value.trim());
                if !path.is_empty() {
                    working_dir = Some(PathBuf::from(path));
                }
            }
            _ => {}
        }
    }
    parse_desktop_exec(
        &exec?,
        &DesktopExecMetadata {
            name,
            icon,
            desktop_file: path.display().to_string(),
        },
        terminal,
        working_dir,
    )
}

fn parse_desktop_exec(
    value: &str,
    metadata: &DesktopExecMetadata,
    terminal: bool,
    working_dir: Option<PathBuf>,
) -> Option<DesktopExec> {
    let value = unescape_desktop_string(value);
    let argv = split_desktop_exec_words(&value)?;
    let mut argv = argv
        .into_iter()
        .flat_map(|arg| expand_desktop_exec_field_codes(arg, metadata));
    let program = argv.next()?.trim().to_string();
    if program.is_empty() {
        return None;
    }
    Some(DesktopExec {
        program,
        args: argv.collect(),
        terminal,
        working_dir,
    })
}

fn split_desktop_exec_words(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut chars = value.chars();
    let mut in_quotes = false;
    let mut in_word = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                in_word = true;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    match next {
                        '"' | '`' | '$' | '\\' => word.push(next),
                        _ => {
                            word.push('\\');
                            word.push(next);
                        }
                    }
                } else {
                    word.push('\\');
                }
                in_word = true;
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            _ => {
                word.push(ch);
                in_word = true;
            }
        }
    }

    if in_quotes {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

fn unescape_desktop_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn expand_desktop_exec_field_codes(arg: String, metadata: &DesktopExecMetadata) -> Vec<String> {
    if arg == "%i" {
        return metadata
            .icon
            .as_ref()
            .map(|icon| vec!["--icon".to_string(), icon.clone()])
            .unwrap_or_default();
    }

    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    let mut removed_field_code = false;
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('i') => {
                if let Some(icon) = &metadata.icon {
                    out.push_str(icon);
                } else {
                    removed_field_code = true;
                }
            }
            Some('c') => {
                if let Some(name) = &metadata.name {
                    out.push_str(name);
                } else {
                    removed_field_code = true;
                }
            }
            Some('k') => out.push_str(&metadata.desktop_file),
            Some('f' | 'F' | 'u' | 'U' | 'v' | 'm') => {
                removed_field_code = true;
            }
            Some(other) if other.is_ascii_alphabetic() => {
                removed_field_code = true;
            }
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    if out.is_empty() && removed_field_code {
        Vec::new()
    } else {
        vec![out]
    }
}

fn spawn_desktop_exec_fallback(id: &str) -> bool {
    let Some(entry) = installed_desktop_entry(id) else {
        eprintln!("gnoblin: no desktop file found for Exec fallback '{id}'");
        return false;
    };
    let Some(exec) = desktop_exec(&entry.path) else {
        eprintln!("gnoblin: desktop file '{id}' has no Exec fallback");
        return false;
    };
    spawn_desktop_exec_command(id, exec)
}

fn spawn_desktop_exec_command(id: &str, exec: DesktopExec) -> bool {
    let exec = if exec.terminal {
        match terminal_command_for_exec(&exec) {
            Some(exec) => exec,
            None => {
                eprintln!("gnoblin: desktop file '{id}' requires a terminal, but no terminal emulator was found");
                return false;
            }
        }
    } else {
        exec
    };
    let mut command = Command::new(&exec.program);
    command
        .args(&exec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(working_dir) = &exec.working_dir {
        command.current_dir(working_dir);
    }
    match command.spawn() {
        Ok(mut child) => {
            let id = id.to_string();
            std::thread::spawn(move || match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("gnoblin: Exec fallback '{id}' exited with {status}"),
                Err(e) => eprintln!("gnoblin: Exec fallback '{id}' wait failed: {e}"),
            });
            true
        }
        Err(e) => {
            eprintln!("gnoblin: failed to spawn Exec fallback '{id}': {e}");
            false
        }
    }
}

fn terminal_command_for_exec(exec: &DesktopExec) -> Option<DesktopExec> {
    if let Some(custom) = std::env::var("GNOBLIN_TERMINAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        let mut words = split_desktop_exec_words(&custom)?;
        if !words.is_empty() {
            let program = words.remove(0);
            if command_available(&program) {
                words.push(exec.program.clone());
                words.extend(exec.args.clone());
                return Some(DesktopExec {
                    program,
                    args: words,
                    terminal: false,
                    working_dir: exec.working_dir.clone(),
                });
            }
        }
    }

    for (program, prefix) in [
        ("ptyxis", &["--new-window", "--"][..]),
        ("foot", &[][..]),
        ("alacritty", &["-e"][..]),
        ("kitty", &[][..]),
        ("gnome-terminal", &["--"][..]),
        ("kgx", &["--"][..]),
        ("konsole", &["-e"][..]),
        ("xterm", &["-e"][..]),
    ] {
        if !command_available(program) {
            continue;
        }
        let mut args: Vec<String> = prefix.iter().map(|arg| (*arg).to_string()).collect();
        args.push(exec.program.clone());
        args.extend(exec.args.clone());
        return Some(DesktopExec {
            program: program.to_string(),
            args,
            terminal: false,
            working_dir: exec.working_dir.clone(),
        });
    }

    None
}

fn command_available(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return is_executable_file(path);
    }
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| is_executable_file(&dir.join(program)))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod desktop_entry_tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVar {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvVar {
        fn set(key: &'static str, value: &Path) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }

        fn set_str(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old }
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                std::env::set_var(self.key, old);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "gnoblin-shell-ui-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_executable(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn expected_desktop_exec(program: String, args: Vec<String>) -> DesktopExec {
        DesktopExec {
            program,
            args,
            terminal: false,
            working_dir: None,
        }
    }

    #[test]
    fn applications_dirs_use_default_data_dirs_when_env_is_empty() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("xdg-data-dirs-empty");
        let data_home = root.join("data");
        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set_str("XDG_DATA_DIRS", "");

        let dirs = applications_dirs();

        assert_eq!(dirs[0], data_home.join("applications"));
        assert!(dirs.contains(&PathBuf::from("/usr/local/share/applications")));
        assert!(dirs.contains(&PathBuf::from("/usr/share/applications")));
    }

    #[test]
    fn installed_entries_use_xdg_ids_for_nested_desktop_files() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("nested");
        let data_home = root.join("data");
        let appdir = data_home.join("applications").join("RPCS3");
        std::fs::create_dir_all(&appdir).unwrap();
        let desktop_path = appdir.join("RPCS3.desktop");
        std::fs::write(
            &desktop_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=RPCS3\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let entries = installed_desktop_entries();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            entries,
            vec![DesktopEntryFile {
                id: "RPCS3-RPCS3".to_string(),
                path: desktop_path
            }]
        );
    }

    #[test]
    fn installed_entries_use_first_xdg_precedence_match_per_id() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("precedence");
        let data_home = root.join("home-data");
        let data_dir = root.join("system-data");
        let home_appdir = data_home.join("applications");
        let system_appdir = data_dir.join("applications");
        std::fs::create_dir_all(&home_appdir).unwrap();
        std::fs::create_dir_all(&system_appdir).unwrap();
        let home_path = home_appdir.join("same.desktop");
        let system_path = system_appdir.join("same.desktop");
        std::fs::write(
            &home_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Home Override\n\
             Hidden=true\n",
        )
        .unwrap();
        std::fs::write(
            &system_path,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=System Copy\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &data_dir);
        let entries = installed_desktop_entries();

        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            entries,
            vec![DesktopEntryFile {
                id: "same".to_string(),
                path: home_path
            }]
        );
    }

    #[test]
    fn desktop_actions_parse_ordered_action_sections() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("desktop-actions");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("firefox.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Firefox\n\
             Exec=firefox %u\n\
             Actions=new-window;private-window;profile-manager;\n\
             \n\
             [Desktop Action private-window]\n\
             Name=New Private Window\n\
             Exec=firefox --private-window\n\
             \n\
             [Desktop Action new-window]\n\
             Name=New Window\n\
             Exec=firefox --new-window %u\n\
             \n\
             [Desktop Action profile-manager]\n\
             Name=Open the Profile Manager\n\
             Exec=firefox --ProfileManager\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        let actions = desktop_actions("firefox");
        assert_eq!(
            actions,
            vec![
                DesktopAction {
                    id: "new-window".to_string(),
                    name: "New Window".to_string()
                },
                DesktopAction {
                    id: "private-window".to_string(),
                    name: "New Private Window".to_string()
                },
                DesktopAction {
                    id: "profile-manager".to_string(),
                    name: "Open the Profile Manager".to_string()
                },
            ]
        );

        let execs = desktop_action_execs("firefox");
        assert_eq!(
            execs[0].exec,
            expected_desktop_exec("firefox".to_string(), vec!["--new-window".to_string()])
        );
        assert_eq!(
            execs[2].exec,
            expected_desktop_exec("firefox".to_string(), vec!["--ProfileManager".to_string()])
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_accepts_nested_exact_and_tail_ids() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-nested");
        let data_home = root.join("data");
        let appdir = data_home.join("applications").join("RPCS3");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("RPCS3.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=RPCS3\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("RPCS3-RPCS3.desktop"),
            Some("RPCS3-RPCS3".to_string())
        );
        assert_eq!(resolve_desktop_id("RPCS3"), Some("RPCS3-RPCS3".to_string()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_accepts_reverse_dns_tail_ids() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-reverse-dns");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("org.mozilla.firefox.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Firefox\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("firefox"),
            Some("org.mozilla.firefox".to_string())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_desktop_id_prefers_exact_final_segment_over_longer_suffix() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("resolve-ambiguous-tail");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::write(
            appdir.join("com.example.More-Settings.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=More Settings\n\
             Exec=foot\n",
        )
        .unwrap();
        std::fs::write(
            appdir.join("org.gnome.Settings.desktop"),
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Settings\n\
             Exec=foot\n",
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert_eq!(
            resolve_desktop_id("settings"),
            Some("org.gnome.Settings".to_string())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_searches_xdg_data_home_hicolor_theme() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-xdg-data-home");
        let data_home = root.join("data");
        let icon_dir = data_home.join("icons/hicolor/48x48/apps");
        std::fs::create_dir_all(&icon_dir).unwrap();
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([0, 128, 255, 255]));
        img.save(icon_dir.join("local-only.png")).unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        assert!(find_icon("local-only", "").is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_at_size_downsamples_large_raster_icons() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-sized-raster");
        let data_home = root.join("data");
        let icon_dir = data_home.join("icons/hicolor/256x256/apps");
        std::fs::create_dir_all(&icon_dir).unwrap();
        let mut img = image::RgbaImage::new(256, 256);
        for y in 0..256 {
            for x in 0..256 {
                img.put_pixel(x, y, image::Rgba([x as u8, y as u8, 192, 255]));
            }
        }
        img.save(icon_dir.join("large-only.png")).unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));

        let icon = find_icon_at_size("large-only", "", 48).unwrap();
        let size = icon.size();
        assert_eq!((size.width, size.height), (48, 48));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_loads_absolute_extensionless_png() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-extensionless-absolute");
        std::fs::create_dir_all(&root).unwrap();
        let icon = root.join("app-icon");
        let mut img = image::RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([255, 64, 0, 255]));
        img.write_to(
            &mut std::io::BufWriter::new(std::fs::File::create(&icon).unwrap()),
            image::ImageOutputFormat::Png,
        )
        .unwrap();

        assert!(find_icon(icon.to_str().unwrap(), "").is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_icon_loads_xpm_icons() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("icon-xpm");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("local-xpm.xpm"),
            r##"/* XPM */
static const char * local_xpm[] = {
"2 2 3 1",
"  c None",
". c #ff0000",
"+ c #00ff00",
".+",
"+."
};
"##,
        )
        .unwrap();

        assert!(find_icon("local-xpm", root.to_str().unwrap()).is_some());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn desktop_exec_argv_parses_quoting_and_field_codes_without_shell() {
        let root = temp_root("desktop-exec-command");
        std::fs::create_dir_all(&root).unwrap();
        let app_path = root.join("Arg Recorder");
        let desktop_path = root.join("settings.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings\n\
                 Exec=\"{}\" \"\\$HOME\" \"two words\" --percent=%% --url=%U %F\n\
                 DBusActivatable=true\n",
                app_path.display()
            ),
        )
        .unwrap();

        assert_eq!(
            desktop_exec(&desktop_path),
            Some(expected_desktop_exec(
                app_path.to_string_lossy().to_string(),
                vec![
                    "$HOME".to_string(),
                    "two words".to_string(),
                    "--percent=%".to_string(),
                    "--url=".to_string(),
                ],
            ))
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn desktop_exec_expands_icon_name_and_desktop_file_field_codes() {
        let root = temp_root("desktop-exec-metadata");
        std::fs::create_dir_all(&root).unwrap();
        let app_path = root.join("Arg Recorder");
        let desktop_path = root.join("settings.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings App\n\
                 Icon=org.example.Settings\n\
                 Exec=\"{}\" %i --name=%c --desktop=%k\n",
                app_path.display()
            ),
        )
        .unwrap();

        assert_eq!(
            desktop_exec(&desktop_path),
            Some(expected_desktop_exec(
                app_path.to_string_lossy().to_string(),
                vec![
                    "--icon".to_string(),
                    "org.example.Settings".to_string(),
                    "--name=Settings App".to_string(),
                    format!("--desktop={}", desktop_path.display()),
                ],
            ))
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_falls_back_to_exec_when_gtk_launch_fails() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("gtk-launch-fallback");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let marker = root.join("launched");
        let recorder = bin_dir.join("record-launch");
        write_executable(&recorder, "#!/bin/sh\nprintf launched > \"$1\"\n");
        let desktop_path = appdir.join("fake.desktop");
        std::fs::write(
            &desktop_path,
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Fake App\n\
                 Exec=\"{}\" \"{}\"\n",
                recorder.display(),
                marker.display()
            ),
        )
        .unwrap();
        let gtk_launch = bin_dir.join("gtk-launch");
        write_executable(&gtk_launch, "#!/bin/sh\nexit 1\n");

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("fake");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !marker.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&marker).ok().as_deref(),
            Some("launched")
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_uses_exec_for_dbus_activatable_entries() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("dbus-activatable-exec");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let launched = root.join("launched");
        let gtk_launch_called = root.join("gtk-launch-called");
        let recorder = bin_dir.join("record-dbus-launch");
        write_executable(&recorder, "#!/bin/sh\nprintf launched > \"$1\"\n");
        std::fs::write(
            appdir.join("org.example.Settings.desktop"),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Settings\n\
                 Exec=\"{}\" \"{}\"\n\
                 DBusActivatable=true\n",
                recorder.display(),
                launched.display()
            ),
        )
        .unwrap();
        let gtk_launch = bin_dir.join("gtk-launch");
        write_executable(
            &gtk_launch,
            &format!(
                "#!/bin/sh\nprintf called > {}\nexit 0\n",
                gtk_launch_called.display()
            ),
        );

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("org.example.Settings");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !launched.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&launched).ok().as_deref(),
            Some("launched")
        );
        assert!(!gtk_launch_called.exists());

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn launch_desktop_app_wraps_terminal_exec_fallback_in_terminal() {
        let _lock = env_lock().lock().unwrap();
        let root = temp_root("terminal-exec-fallback");
        let data_home = root.join("data");
        let appdir = data_home.join("applications");
        let bin_dir = root.join("bin");
        let workdir = root.join("work dir");
        std::fs::create_dir_all(&appdir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let args_marker = root.join("terminal-args");
        let cwd_marker = root.join("terminal-cwd");
        let app_path = bin_dir.join("terminal-app");
        write_executable(&app_path, "#!/bin/sh\nexit 0\n");
        write_executable(
            &bin_dir.join("ptyxis"),
            &format!(
                "#!/bin/sh\npwd > '{}'\nprintf '%s\\n' \"$@\" > '{}'\n",
                cwd_marker.display(),
                args_marker.display()
            ),
        );
        write_executable(&bin_dir.join("gtk-launch"), "#!/bin/sh\nexit 1\n");
        std::fs::write(
            appdir.join("terminal.desktop"),
            format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Terminal App\n\
                 Exec=\"{}\" \"two words\"\n\
                 Path={}\n\
                 Terminal=true\n",
                app_path.display(),
                workdir.display()
            ),
        )
        .unwrap();

        let _xdg_data_home = EnvVar::set("XDG_DATA_HOME", &data_home);
        let _xdg_data_dirs = EnvVar::set("XDG_DATA_DIRS", &root.join("empty-data-dirs"));
        let _path = EnvVar::set("PATH", &bin_dir);

        launch_desktop_app("terminal");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline && !args_marker.exists() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert_eq!(
            std::fs::read_to_string(&cwd_marker).ok().as_deref(),
            Some(format!("{}\n", workdir.display()).as_str())
        );
        assert_eq!(
            std::fs::read_to_string(&args_marker).ok().as_deref(),
            Some(format!("--new-window\n--\n{}\ntwo words\n", app_path.display()).as_str())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }
}

fn load_icon_path_sized(path: &Path, target_size: Option<u32>) -> Option<slint::Image> {
    if !path.exists() {
        return None;
    }
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    if path.extension().is_none() {
        if let Some(img) = load_raster_icon_path(path, target_size) {
            return Some(img);
        }
    } else if ext.as_deref() == Some("xpm") {
        return load_xpm_icon_path(path);
    } else if target_size.is_some() && matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) {
        if let Some(img) = load_raster_icon_path(path, target_size) {
            return Some(img);
        }
    }
    if let Ok(img) = slint::Image::load_from_path(path) {
        return Some(img);
    }
    load_raster_icon_path(path, target_size).or_else(|| load_xpm_icon_path(path))
}

fn load_raster_icon_path(path: &Path, target_size: Option<u32>) -> Option<slint::Image> {
    let bytes = std::fs::read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?;
    if let Some(size) = target_size.filter(|size| *size > 0) {
        if img.width() != size || img.height() != size {
            img = img.resize(size, size, image::imageops::FilterType::Lanczos3);
        }
    }
    let img = img.to_rgba8();
    let (w, h) = img.dimensions();
    let buf =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&img.into_raw(), w, h);
    Some(slint::Image::from_rgba8(buf))
}

fn load_xpm_icon_path(path: &Path) -> Option<slint::Image> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines = xpm_string_lines(&text);
    let header = lines.first()?;
    let mut header_parts = header.split_whitespace();
    let width: usize = header_parts.next()?.parse().ok()?;
    let height: usize = header_parts.next()?.parse().ok()?;
    let color_count: usize = header_parts.next()?.parse().ok()?;
    let chars_per_pixel: usize = header_parts.next()?.parse().ok()?;
    if width == 0 || height == 0 || chars_per_pixel == 0 || width > 4096 || height > 4096 {
        return None;
    }
    if lines.len() < 1 + color_count + height {
        return None;
    }

    let mut colors = HashMap::new();
    for line in &lines[1..1 + color_count] {
        let (key, rest) = split_xpm_key(line, chars_per_pixel)?;
        let color = parse_xpm_color(rest)?;
        colors.insert(key, color);
    }

    let mut rgba = Vec::with_capacity(width.checked_mul(height)?.checked_mul(4)?);
    for line in &lines[1 + color_count..1 + color_count + height] {
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < width.checked_mul(chars_per_pixel)? {
            return None;
        }
        for x in 0..width {
            let start = x * chars_per_pixel;
            let key: String = chars[start..start + chars_per_pixel].iter().collect();
            rgba.extend_from_slice(colors.get(&key)?);
        }
    }

    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        &rgba,
        width as u32,
        height as u32,
    );
    Some(slint::Image::from_rgba8(buf))
}

fn xpm_string_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;
    for ch in text.chars() {
        if in_string {
            if escape {
                current.push(ch);
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                lines.push(std::mem::take(&mut current));
                in_string = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_string = true;
        }
    }
    lines
}

fn split_xpm_key(line: &str, chars_per_pixel: usize) -> Option<(String, &str)> {
    let mut end = 0;
    for (count, (idx, ch)) in line.char_indices().enumerate() {
        if count == chars_per_pixel {
            break;
        }
        end = idx + ch.len_utf8();
    }
    if line[..end].chars().count() != chars_per_pixel {
        return None;
    }
    Some((line[..end].to_string(), &line[end..]))
}

fn parse_xpm_color(rest: &str) -> Option<[u8; 4]> {
    let mut tokens = rest.split_whitespace();
    while let Some(token) = tokens.next() {
        if token != "c" {
            continue;
        }
        let color = tokens.next()?;
        if color.eq_ignore_ascii_case("none") {
            return Some([0, 0, 0, 0]);
        }
        return parse_xpm_hex_color(color);
    }
    None
}

fn parse_xpm_hex_color(color: &str) -> Option<[u8; 4]> {
    let hex = color.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let r = parse_xpm_nibble(chars.next()?)? * 17;
            let g = parse_xpm_nibble(chars.next()?)? * 17;
            let b = parse_xpm_nibble(chars.next()?)? * 17;
            Some([r, g, b, 255])
        }
        6 => Some([
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255,
        ]),
        _ => None,
    }
}

fn parse_xpm_nibble(ch: char) -> Option<u8> {
    ch.to_digit(16).map(|v| v as u8)
}

/// Resolve an icon by name (or absolute path) from an explicit theme path then
/// the system icon theme dirs -> a Slint image. Used by the dock + the SNI tray.
pub fn find_icon(name: &str, theme_path: &str) -> Option<slint::Image> {
    find_icon_internal(name, theme_path, None)
}

/// Resolve an icon like [`find_icon`], but pre-render raster sources near the
/// requested logical size. This mirrors GTK/GNOME's size-aware icon path and
/// avoids leaving large app icons for the renderer to downsample every frame.
pub fn find_icon_at_size(name: &str, theme_path: &str, logical_size: u32) -> Option<slint::Image> {
    find_icon_internal(name, theme_path, Some(logical_size.max(1)))
}

// (name, theme_path, target_size) → resolved image (or None, cached so misses
// aren't re-searched).
type IconCache = std::collections::HashMap<(String, String, Option<u32>), Option<slint::Image>>;

thread_local! {
    // Resolved-icon cache: the theme search stats dozens of paths per lookup, and
    // find_icon runs for every tile + submenu row on every control-centre refresh,
    // so an uncached lookup is a real open-time cost. slint::Image is cheap to
    // clone (ref-counted).
    static ICON_CACHE: std::cell::RefCell<IconCache> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn find_icon_internal(
    name: &str,
    theme_path: &str,
    target_size: Option<u32>,
) -> Option<slint::Image> {
    let key = (name.to_string(), theme_path.to_string(), target_size);
    if let Some(hit) = ICON_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }
    let result = find_icon_uncached(name, theme_path, target_size);
    inspect_log_icon(name, theme_path, target_size, &result);
    ICON_CACHE.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}

/// When `GNOBLIN_INSPECT` is set, append each unique icon resolution (logged
/// once, on cache miss) to `$XDG_RUNTIME_DIR/gnoblin-inspect/icons-<pid>.jsonl`
/// so the inspector can see exactly which icon resolved for each name/size and at
/// what pixel dimensions — the Slint-side counterpart to the compositor's scene
/// dump. No-op unless the env var is set.
fn inspect_log_icon(
    name: &str,
    theme_path: &str,
    size: Option<u32>,
    result: &Option<slint::Image>,
) {
    if std::env::var_os("GNOBLIN_INSPECT").is_none() {
        return;
    }
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => std::path::PathBuf::from(d).join("gnoblin-inspect"),
        None => return,
    };
    let _ = std::fs::create_dir_all(&dir);
    let dims = result
        .as_ref()
        .map(|img| {
            let s = img.size();
            format!("[{},{}]", s.width, s.height)
        })
        .unwrap_or_else(|| "null".to_string());
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let line = format!(
        "{{\"name\":\"{}\",\"theme\":\"{}\",\"req_size\":{},\"resolved\":{},\"dims\":{}}}\n",
        esc(name),
        esc(theme_path),
        size.map(|s| s.to_string()).unwrap_or_else(|| "null".into()),
        result.is_some(),
        dims,
    );
    use std::io::Write;
    let path = dir.join(format!("icons-{}.jsonl", std::process::id()));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Read the visual properties of a Slint item if it's a (Border)Rectangle: the
/// per-corner border radius, border width, and the background + border colours
/// (ARGB bytes). Returns a JSON fragment (leading comma) or None. Uses the
/// unstable item downcast + render-trait accessors.
fn slint_item_props(item: &i_slint_core::item_tree::ItemRc) -> Option<String> {
    use i_slint_core::items::{BorderRectangle, ComplexText, ItemRef, Rectangle, SimpleText};
    use std::fmt::Write;

    let mut out = String::new();
    if let Some(br) = ItemRef::downcast_pin::<BorderRectangle>(item.borrow()) {
        let bg = br.background().color();
        let bc = br.border_color().color();
        let _ = write!(
            out,
            ",\"radius\":{:.1},\"border_w\":{:.1},\
             \"bg\":[{},{},{},{}],\"border_col\":[{},{},{},{}]",
            br.border_radius().get(),
            br.border_width().get(),
            bg.red(),
            bg.green(),
            bg.blue(),
            bg.alpha(),
            bc.red(),
            bc.green(),
            bc.blue(),
            bc.alpha(),
        );
        return Some(out);
    }
    if let Some(rect) = ItemRef::downcast_pin::<Rectangle>(item.borrow()) {
        let bg = rect.background().color();
        let _ = write!(
            out,
            ",\"bg\":[{},{},{},{}]",
            bg.red(),
            bg.green(),
            bg.blue(),
            bg.alpha()
        );
        return Some(out);
    }
    // Text comes in two item flavours (SimpleText / ComplexText) with the same
    // text/font_size/font_weight/colour Property fields. Read them directly via
    // FIELD_OFFSETS (the text()/color() methods are ambiguous between inherent and
    // RenderText-trait versions).
    macro_rules! try_text {
        ($ty:ty) => {
            if let Some(t) = ItemRef::downcast_pin::<$ty>(item.borrow()) {
                let text = <$ty>::FIELD_OFFSETS.text().apply_pin(t).get();
                let fs = <$ty>::FIELD_OFFSETS.font_size().apply_pin(t).get().get();
                let fw = <$ty>::FIELD_OFFSETS.font_weight().apply_pin(t).get();
                let col = <$ty>::FIELD_OFFSETS.color().apply_pin(t).get().color();
                let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
                let _ = write!(
                    out,
                    ",\"text\":\"{}\",\"font_size\":{:.1},\"font_weight\":{},\"color\":[{},{},{},{}]",
                    esc(text.as_str()),
                    fs,
                    fw,
                    col.red(),
                    col.green(),
                    col.blue(),
                    col.alpha(),
                );
                return Some(out);
            }
        };
    }
    try_text!(SimpleText);
    try_text!(ComplexText);
    None
}

/// Recursively serialise a Slint item subtree (depth-first) into `out` as JSON
/// objects: depth, item index, geometry [x,y,w,h], accessible role, element
/// type name (empty unless built with SLINT_EMIT_DEBUG_INFO), and — for
/// rectangles — the visual props (radius/border/colours). Bounded so a
/// pathological tree can't run away.
fn walk_slint_elements(
    item: &i_slint_core::item_tree::ItemRc,
    depth: u32,
    out: &mut String,
    first: &mut bool,
    count: &mut usize,
) {
    use std::fmt::Write;
    if *count >= 800 || depth > 16 {
        return;
    }
    *count += 1;
    let g = item.geometry();
    let role = format!("{:?}", item.accessible_role());
    let ty = item
        .element_type_names_and_ids(0)
        .and_then(|v| v.into_iter().next())
        .map(|(t, _id)| t.to_string())
        .unwrap_or_default();
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    if !*first {
        out.push(',');
    }
    *first = false;
    let props = slint_item_props(item).unwrap_or_default();
    let _ = write!(
        out,
        "{{\"depth\":{},\"i\":{},\"geom\":[{:.1},{:.1},{:.1},{:.1}],\"role\":\"{}\",\"type\":\"{}\"{}}}",
        depth,
        item.index(),
        g.origin.x,
        g.origin.y,
        g.size.width,
        g.size.height,
        esc(&role),
        esc(&ty),
        props,
    );
    let mut child = item.first_child();
    while let Some(c) = child {
        walk_slint_elements(&c, depth + 1, out, first, count);
        child = c.next_sibling();
    }
}

fn find_icon_uncached(
    name: &str,
    theme_path: &str,
    target_size: Option<u32>,
) -> Option<slint::Image> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return load_icon_path_sized(Path::new(name), target_size);
    }
    if !theme_path.is_empty() {
        for ext in ["svg", "png", "xpm"] {
            let p = format!("{theme_path}/{name}.{ext}");
            if let Some(img) = load_icon_path_sized(Path::new(&p), target_size) {
                return Some(img);
            }
        }
    }
    let mut theme_dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(PathBuf::from(&home).join(".icons").join(theme));
        }
    }
    if let Some(data_home) = xdg_data_home() {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(data_home.join("icons").join(theme));
        }
    }
    for data_dir in xdg_data_dirs() {
        for theme in ["hicolor", "Adwaita", "breeze"] {
            theme_dirs.push(data_dir.join("icons").join(theme));
        }
    }
    const SIZES: &[&str] = &[
        "scalable", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24", "symbolic",
    ];
    for theme in theme_dirs {
        for size in SIZES {
            for cat in ["apps", "status", "devices"] {
                for ext in ["svg", "png", "xpm"] {
                    let p = theme.join(size).join(cat).join(format!("{name}.{ext}"));
                    if p.exists() {
                        if let Some(img) = load_icon_path_sized(&p, target_size) {
                            return Some(img);
                        }
                    }
                }
            }
        }
    }
    for ext in ["svg", "png", "xpm"] {
        let p = format!("/usr/share/pixmaps/{name}.{ext}");
        if let Some(img) = load_icon_path_sized(Path::new(&p), target_size) {
            return Some(img);
        }
    }
    None
}

/// The bar-specific behaviour: build + show its Slint component, refresh its
/// data, and expose its window for input. The component is created inside
/// `show()` (after the Slint platform is installed).
pub trait BarApp {
    /// Create the Slint component, push initial data, show it. Called once the
    /// EGL/renderer platform is ready, with the surface's logical size and the
    /// containing output's size (for positioning the glass backdrop).
    fn show(
        &mut self,
        width: u32,
        height: u32,
        screen_w: u32,
        screen_h: u32,
    ) -> Result<(), RuntimeError>;
    /// The layer surface was resized after the component was shown. The Slint
    /// window itself has already received `WindowEvent::Resized`; clients that
    /// cache output-sized geometry can update their own properties here.
    fn resized(&mut self, _width: u32, _height: u32, _screen_w: u32, _screen_h: u32) {}
    /// Periodic refresh (clock, running apps, …). Return true if anything
    /// changed and a redraw is needed.
    fn tick(&mut self) -> bool;
    /// The component's window, for dispatching pointer events.
    fn window(&self) -> Option<&slint::Window>;
    /// When the surface is `full_height`, return true to make the WHOLE surface
    /// grab input (so an open dropdown's outside-clicks dismiss it); false keeps
    /// input to the top `input_height` strip so clicks pass through to windows.
    fn input_full(&self) -> bool {
        false
    }
    /// Override the input region with explicit surface-local rects `(x,y,w,h)`.
    /// When Some on a `full_height` surface, ONLY these rects accept input
    /// (everything else passes through) — used by the notification daemon so
    /// only the cards are clickable. Default None (use `input_full()`).
    fn input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        None
    }
    /// Return true to tear down the surface and exit `run()` — used by one-shot
    /// on-demand clients (e.g. the window menu) that close after a pick/dismiss.
    fn should_exit(&self) -> bool {
        false
    }
    /// A key was pressed (only when `BarConfig.keyboard`). `text` is the typed
    /// UTF-8 or a `slint::platform::Key` char for special keys. Clients that take
    /// text input (the launcher) handle it here. Default: no-op.
    fn key_pressed(&mut self, _text: &slint::SharedString) {}
}

/// Where + how big the layer-shell surface is.
pub struct BarConfig {
    pub namespace: &'static str,
    pub anchor: Anchor,
    pub layer: Layer,
    /// Logical height in px (width spans the anchored edges).
    pub height: u32,
    /// Reserved work-area edge in px (0 = none).
    pub exclusive_zone: i32,
    /// Make the surface span the whole output (anchored on all edges) while the
    /// visible bar still lives in the top `height` px. Lets in-scene dropdowns
    /// render below the bar over windows. Input is limited to the bar strip
    /// unless `BarApp::input_full()` returns true. Default false (current bars).
    pub full_height: bool,
    /// On a `full_height` surface, set an EMPTY input region so all input passes
    /// through to windows below (e.g. the wallpaper). Overrides `input_full()`.
    pub input_passthrough: bool,
    /// Request keyboard focus (KeyboardInteractivity::OnDemand) and forward key
    /// events into Slint — for clients with text input (e.g. the launcher).
    pub keyboard: bool,
}

impl Default for BarConfig {
    fn default() -> Self {
        BarConfig {
            namespace: "gnoblin-bar",
            anchor: Anchor::TOP,
            layer: Layer::Top,
            height: 34,
            exclusive_zone: 0,
            full_height: false,
            input_passthrough: false,
            keyboard: false,
        }
    }
}

// ── EGL context as a Slint OpenGLInterface ──────────────────────────────────

struct EglState {
    egl: egl::Instance<egl::Static>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    _wl_egl: wayland_egl::WlEglSurface,
}

// When the compositor goes away, eglSwapBuffers/eglMakeCurrent fail but
// eglGetError() returns SUCCESS (the failure is at the Wayland layer), so
// khronos-egl's `get_error().unwrap()` panics on `None`. Catch that so a dead
// display surfaces as an `Err` — Slint's render() then returns Err and the run
// loop exits cleanly instead of the process aborting.
fn guard_egl<T>(
    what: &str,
    f: impl FnOnce() -> Result<T, egl::Error>,
) -> Result<T, Box<dyn Error + Send + Sync>> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e)),
        Err(_) => Err(format!("EGL {what} failed (compositor gone?)").into()),
    }
}

unsafe impl OpenGLInterface for EglState {
    fn ensure_current(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        guard_egl("make_current", || {
            self.egl.make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
        })
    }
    fn swap_buffers(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        guard_egl("swap_buffers", || {
            self.egl.swap_buffers(self.display, self.surface)
        })
    }
    fn resize(
        &self,
        width: NonZeroU32,
        height: NonZeroU32,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self._wl_egl
            .resize(width.get() as i32, height.get() as i32, 0, 0);
        Ok(())
    }
    fn get_proc_address(&self, name: &CStr) -> *const c_void {
        match name
            .to_str()
            .ok()
            .and_then(|n| self.egl.get_proc_address(n))
        {
            Some(p) => p as *const c_void,
            None => std::ptr::null(),
        }
    }
}

fn setup_egl(
    conn: &Connection,
    surface: &wl_surface::WlSurface,
    w: u32,
    h: u32,
) -> Result<EglState, RuntimeError> {
    let egl = egl::Instance::new(egl::Static);
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let display =
        unsafe { egl.get_display(display_ptr) }.ok_or_else(|| runtime_error("EGL: no display"))?;
    egl.initialize(display)
        .map_err(|e| runtime_error(format!("EGL: initialize: {e}")))?;
    egl.bind_api(egl::OPENGL_ES_API)
        .map_err(|e| runtime_error(format!("EGL: bind GLES: {e}")))?;

    let config_attribs = [
        egl::SURFACE_TYPE,
        egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE,
        egl::OPENGL_ES2_BIT,
        egl::RED_SIZE,
        8,
        egl::GREEN_SIZE,
        8,
        egl::BLUE_SIZE,
        8,
        egl::ALPHA_SIZE,
        8,
        egl::NONE,
    ];
    let config = egl
        .choose_first_config(display, &config_attribs)
        .map_err(|e| runtime_error(format!("EGL: choose config: {e}")))?
        .ok_or_else(|| runtime_error("EGL: no matching config"))?;
    let context_attribs = [egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE];
    let context = egl
        .create_context(display, config, None, &context_attribs)
        .map_err(|e| runtime_error(format!("EGL: create context: {e}")))?;
    let wl_egl = wayland_egl::WlEglSurface::new(surface.id(), w.max(1) as i32, h.max(1) as i32)
        .map_err(|e| runtime_error(format!("wl_egl_window: {e}")))?;
    let egl_surface = unsafe {
        egl.create_window_surface(display, config, wl_egl.ptr() as *mut c_void, None)
            .map_err(|e| runtime_error(format!("EGL: window surface: {e}")))?
    };
    Ok(EglState {
        egl,
        display,
        surface: egl_surface,
        context,
        _wl_egl: wl_egl,
    })
}

// ── Slint WindowAdapter backed by the FemtoVG/EGL renderer ──────────────────

struct BarAdapter {
    window: slint::Window,
    renderer: FemtoVGRenderer,
    size: Cell<PhysicalSize>,
    needs_redraw: Cell<bool>,
}

impl BarAdapter {
    fn new(egl: EglState, w: u32, h: u32) -> Result<Rc<Self>, slint::PlatformError> {
        // FemtoVGRenderer::new reads GL_VERSION immediately, so the context must
        // already be current — it does not call ensure_current() for us.
        egl.ensure_current()?;
        let renderer = FemtoVGRenderer::new(egl)?;
        Ok(Rc::new_cyclic(|weak: &Weak<BarAdapter>| BarAdapter {
            window: slint::Window::new(weak.clone()),
            renderer,
            size: Cell::new(PhysicalSize::new(w, h)),
            needs_redraw: Cell::new(true),
        }))
    }
}

impl WindowAdapter for BarAdapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn request_redraw(&self) {
        self.needs_redraw.set(true);
    }
}

struct BarPlatform {
    egl: RefCell<Option<EglState>>,
    size: (u32, u32),
    shared: Rc<RefCell<Option<Rc<BarAdapter>>>>,
    start: Instant,
}

impl Platform for BarPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let egl = self
            .egl
            .borrow_mut()
            .take()
            .ok_or_else(|| slint::PlatformError::from("EGL context already taken"))?;
        let adapter = BarAdapter::new(egl, self.size.0, self.size.1)?;
        *self.shared.borrow_mut() = Some(adapter.clone());
        Ok(adapter)
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

// ── output resolution (multi-monitor) ──────────────────────────────────────
//
// A layer surface binds its output at CREATION time, but sctk's `OutputState`
// only knows the outputs after a roundtrip — and a roundtrip needs a dispatch
// target. To avoid threading an `Option<LayerSurface>` through `State`, we
// resolve the `--output <connector>` name on a throwaway event queue (the same
// connection, so the resulting `wl_output` is valid for `get_layer_surface`)
// before the real surface is created. With no `--output`, the compositor picks.

struct OutputProbe {
    registry_state: RegistryState,
    output_state: OutputState,
}

impl OutputHandler for OutputProbe {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for OutputProbe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_output!(OutputProbe);
delegate_registry!(OutputProbe);

/// Find the `wl_output` whose connector name matches `name` (e.g. "DP-1",
/// "Meta-0"). Enumerates outputs on a dedicated queue so it can run before the
/// layer surface exists. Returns `None` if no output matches.
fn resolve_output(
    conn: &Connection,
    globals: &GlobalList,
    name: &str,
) -> Option<wl_output::WlOutput> {
    let mut queue = conn.new_event_queue::<OutputProbe>();
    let qh = queue.handle();
    let mut probe = OutputProbe {
        registry_state: RegistryState::new(globals),
        output_state: OutputState::new(globals, &qh),
    };
    // First roundtrip binds the wl_output globals; the second drains their
    // name/geometry events so `info().name` is populated.
    let _ = queue.roundtrip(&mut probe);
    let _ = queue.roundtrip(&mut probe);
    let found = probe
        .output_state
        .outputs()
        .find(|o| probe.output_state.info(o).and_then(|i| i.name).as_deref() == Some(name));
    if found.is_none() {
        let names: Vec<String> = probe
            .output_state
            .outputs()
            .filter_map(|o| probe.output_state.info(&o).and_then(|i| i.name))
            .collect();
        eprintln!("gnoblin: --output '{name}' not found; available outputs: {names:?}");
    }
    found
}

// ── sctk layer-shell client state ───────────────────────────────────────────

struct State {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    layer: LayerSurface,
    qh: QueueHandle<State>,
    conn: Connection,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    // Logical surface size + the output's integer buffer scale (HiDPI). The EGL
    // buffer + the Slint window report PHYSICAL pixels = logical * scale.
    width: u32,
    height: u32,
    scale: u32,
    configured: bool,
    exit: bool,
    adapter: Option<Rc<BarAdapter>>,
    app: Box<dyn BarApp>,
    // full-height surface support: input limited to the top `input_height` strip
    // unless the app requests the whole surface (open dropdown).
    full_height: bool,
    input_height: u32,
    input_passthrough: bool,
    input_rects_applied: Option<Vec<(i32, i32, i32, i32)>>,
    input_region_dirty: bool,
    // Frame-callback throttling: a buffer commit (eglSwapBuffers) requests a frame
    // callback; we must NOT commit again until it's delivered, or mutter aborts on
    // `frame_callback_list` not being empty. `frame_pending` gates all rendering.
    frame_pending: bool,
    // Layer-shell configure storms can arrive in small bursts while the devkit's
    // virtual monitor materializes/resizes. If we render immediately after the
    // first configure in the burst, mutter can send another configure before it
    // processes our buffer commit and then reject the attach as "before
    // ack_configure". Delay the first post-configure buffer by one tick.
    last_configure_at: Option<Instant>,
    startup_error: Option<String>,
}

impl State {
    fn init_slint(&mut self) -> Result<(), RuntimeError> {
        let surface = self.layer.wl_surface().clone();
        // The Slint platform + EGL buffer are created ONCE here. The
        // scale_factor_changed event may not have fired before this first
        // configure, leaving self.scale at 1 — which would build the platform at
        // the wrong (1×) size on a HiDPI output and never fully recover. Seed the
        // scale from the output's actual factor now so the buffer is physical-
        // sized from the start.
        if let Some(s) = self
            .output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .map(|i| i.scale_factor.max(1) as u32)
        {
            self.scale = s;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        // The EGL buffer is sized in PHYSICAL pixels (logical × output scale).
        if self.scale != 1 {
            surface.set_buffer_scale(self.scale as i32);
        }
        let egl = setup_egl(&self.conn, &surface, pw, ph)?;
        let shared = Rc::new(RefCell::new(None));
        let platform = BarPlatform {
            egl: RefCell::new(Some(egl)),
            size: (pw, ph),
            shared: shared.clone(),
            start: Instant::now(),
        };
        slint::platform::set_platform(Box::new(platform))
            .map_err(|e| runtime_error(format!("set Slint platform: {e}")))?;

        let (screen_w, screen_h) = self.screen_size();
        self.app.show(self.width, self.height, screen_w, screen_h)?;
        let window = self
            .app
            .window()
            .ok_or_else(|| runtime_error("Slint app did not create a window"))?;
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: self.scale as f32,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: LogicalSize::new(self.width as f32, self.height as f32),
        });
        self.adapter = shared.borrow().clone();
        self.inspect_log_window();
        Ok(())
    }

    /// When `GNOBLIN_INSPECT` is set, write this client's self-view —
    /// logical/physical size, output scale, theme, full-height/input strip — to
    /// `$XDG_RUNTIME_DIR/gnoblin-inspect/window-<pid>.json` (overwritten with the
    /// latest state). The inspector correlates it to the compositor surface by
    /// pid, so e.g. a client's `theme_dark` can be checked against the ring the
    /// compositor drew. No-op unless the env var is set.
    fn inspect_log_window(&self) {
        if std::env::var_os("GNOBLIN_INSPECT").is_none() {
            return;
        }
        let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(d) => std::path::PathBuf::from(d).join("gnoblin-inspect"),
            None => return,
        };
        let _ = std::fs::create_dir_all(&dir);
        // Raw output info the client sees (to debug HiDPI logical-size issues).
        let (ol, osf, om) = self
            .output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .map(|i| {
                let mode = i
                    .modes
                    .iter()
                    .find(|m| m.current)
                    .map(|m| m.dimensions)
                    .unwrap_or((0, 0));
                (i.logical_size, i.scale_factor, mode)
            })
            .unwrap_or((None, 1, (0, 0)));
        let ol = ol
            .map(|(w, h)| format!("[{w},{h}]"))
            .unwrap_or_else(|| "null".into());
        // The ACTUAL committed buffer size (the EGL/Slint adapter physical size) —
        // to tell whether a HiDPI render bug is the client buffer being wrong vs
        // mutter mis-scaling a correct buffer.
        let buf = self
            .adapter
            .as_ref()
            .map(|a| {
                let s = a.size.get();
                format!("[{},{}]", s.width, s.height)
            })
            .unwrap_or_else(|| "null".into());
        // What Slint ITSELF thinks: window physical size + its scale_factor. If
        // scale_factor is 4 (not 2), the renderer double-scales the content.
        let (slint_win, slint_sc) = self
            .app
            .window()
            .map(|w| {
                let s = w.size();
                let sc = i_slint_core::window::WindowInner::from_pub(w).scale_factor();
                (format!("[{},{}]", s.width, s.height), sc)
            })
            .unwrap_or_else(|| ("null".into(), 0.0));
        let json = format!(
            "{{\"pid\":{},\"theme_dark\":{},\"scale\":{},\"logical\":[{},{}],\
             \"physical\":[{},{}],\"egl_buffer\":{},\"slint_win\":{},\"slint_scale\":{:.2},\
             \"full_height\":{},\"input_height\":{},\
             \"out_logical\":{},\"out_scale\":{},\"out_mode\":[{},{}]}}\n",
            std::process::id(),
            crate::theme::is_dark(),
            self.scale,
            self.width,
            self.height,
            self.width * self.scale,
            self.height * self.scale,
            buf,
            slint_win,
            slint_sc,
            self.full_height,
            self.input_height,
            ol,
            osf,
            om.0,
            om.1,
        );
        let path = dir.join(format!("window-{}.json", std::process::id()));
        let _ = std::fs::write(path, json);
        self.inspect_log_elements(&dir);
    }

    /// Walk the live Slint item tree (per-element geometry/role/type) and write it
    /// to `elements-<pid>.json`. Uses the UNSTABLE `i_slint_core` item-tree API
    /// (the only way to read a compiled component's element tree). Element type
    /// names require `SLINT_EMIT_DEBUG_INFO=1` at build time; geometry + role work
    /// regardless. No-op unless `GNOBLIN_INSPECT` is set (checked by the caller).
    fn inspect_log_elements(&self, dir: &std::path::Path) {
        use i_slint_core::item_tree::ItemRc;

        let window = match self.app.window() {
            Some(w) => w,
            None => return,
        };
        // try_component (not component()) so a window whose root isn't set yet
        // can't panic the client from the inspector path.
        let comp = match i_slint_core::window::WindowInner::from_pub(window).try_component() {
            Some(c) => c,
            None => return,
        };
        let root = ItemRc::new(comp, 0);
        let mut out = String::from("[");
        let mut first = true;
        let mut count = 0usize;
        walk_slint_elements(&root, 0, &mut out, &mut first, &mut count);
        out.push(']');
        let path = dir.join(format!("elements-{}.json", std::process::id()));
        let _ = std::fs::write(path, out);
    }

    /// Re-apply the current logical size + scale to the live surface: the EGL
    /// buffer becomes PHYSICAL (logical × scale), the Slint window keeps LOGICAL
    /// coords. Called on a configure resize or an output scale change.
    fn apply_size(&mut self) {
        if self.adapter.is_none() {
            return;
        }
        let (pw, ph) = (self.width * self.scale, self.height * self.scale);
        if let Some(adapter) = &self.adapter {
            adapter.size.set(PhysicalSize::new(pw.max(1), ph.max(1)));
        }
        if let Some(window) = self.app.window() {
            window.dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: self.scale as f32,
            });
            window.dispatch_event(WindowEvent::Resized {
                size: LogicalSize::new(self.width as f32, self.height as f32),
            });
        } else {
            eprintln!("gnoblin-shell-ui: Slint app window disappeared during resize; exiting.");
            self.exit = true;
            return;
        }
        let (screen_w, screen_h) = self.screen_size();
        self.app
            .resized(self.width, self.height, screen_w, screen_h);
        self.inspect_log_window();
        // The input region is sized in surface-logical px. Apply it from the
        // post-dispatch commit point so it cannot race unread configures.
        self.input_region_dirty = true;
        // Draw the new size, but obey the single-outstanding-frame-callback rule:
        // if a frame is already in flight, the `frame` callback will redraw. Forcing
        // a render here (the old `frame_pending = false; render()`) committed a 2nd
        // frame callback while one was pending → mutter aborts on
        // `frame_callback_list` not being empty (seen as a devkit crash on the
        // rapid placeholder→real-size configures at startup).
        if let Some(a) = &self.adapter {
            a.needs_redraw.set(true);
        }
        // The post-dispatch render (run()'s loop) draws this — committing here
        // could race an unread resize configure.
    }

    /// The containing output's logical size. For full-height layer surfaces, the
    /// compositor-configured surface size is the authoritative output-sized value;
    /// output metadata can lag behind both startup and resize configures.
    fn screen_size(&self) -> (u32, u32) {
        if self.full_height && self.configured {
            return (self.width.max(1), self.height.max(1));
        }
        self.output_state
            .outputs()
            .next()
            .and_then(|o| self.output_state.info(&o))
            .and_then(|i| {
                // Prefer the LOGICAL output size. Fall back to the current mode
                // divided by the output scale — the mode is in PHYSICAL pixels, so
                // using it raw on a scaled (HiDPI) output sizes surfaces for the
                // unscaled resolution and renders only their top-left quarter.
                i.logical_size.or_else(|| {
                    let s = i.scale_factor.max(1);
                    i.modes
                        .iter()
                        .find(|m| m.current)
                        .map(|m| (m.dimensions.0 / s, m.dimensions.1 / s))
                })
            })
            .map(|(w, h)| (w.max(1) as u32, h.max(1) as u32))
            .unwrap_or((self.width.max(1), self.height.max(1)))
    }

    fn has_active_animations(&self) -> bool {
        self.adapter
            .as_ref()
            .map(|a| a.window.has_active_animations())
            .unwrap_or(false)
    }

    fn configure_settle_remaining(&self) -> Option<Duration> {
        let elapsed = self.last_configure_at?.elapsed();
        if elapsed >= CONFIGURE_RENDER_DELAY {
            None
        } else {
            Some(CONFIGURE_RENDER_DELAY - elapsed)
        }
    }

    fn next_dispatch_timeout(&self) -> Duration {
        if let Some(remaining) = self.configure_settle_remaining() {
            return remaining.min(IDLE_DISPATCH_TIMEOUT);
        }
        if self.has_active_animations() && !self.frame_pending {
            return Duration::ZERO;
        }
        slint::platform::duration_until_next_timer_update()
            .unwrap_or(IDLE_DISPATCH_TIMEOUT)
            .min(IDLE_DISPATCH_TIMEOUT)
    }

    /// Let Slint advance timers/animations before deciding whether a frame is
    /// needed. `has_active_animations()` describes the previous render pass; if
    /// it was true, the advanced animation time must produce at least one more
    /// draw so the animation can converge even when no pointer input arrives.
    fn pump_slint(&mut self) {
        if self.adapter.is_none() {
            return;
        }
        let had_active_animations = self.has_active_animations();
        slint::platform::update_timers_and_animations();
        if had_active_animations {
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
        }
    }

    /// True if there's a frame worth drawing — the app/Slint flagged a redraw,
    /// or Slint reports an active animation from the last scene evaluation.
    fn wants_redraw(&self) -> bool {
        let dirty = self
            .adapter
            .as_ref()
            .map(|a| a.needs_redraw.get())
            .unwrap_or(false);
        dirty || self.has_active_animations()
    }

    fn ready_to_render(&mut self) -> bool {
        if self.frame_pending {
            return false;
        }
        if self.configure_settle_remaining().is_some() {
            return false;
        }
        if self.last_configure_at.is_some() {
            self.last_configure_at = None;
        }
        true
    }

    /// Draw + commit ONE frame.
    fn render(&mut self) {
        let surface = self.layer.wl_surface().clone();
        surface.frame(&self.qh, surface.clone());
        let result = self.adapter.as_ref().map(|adapter| {
            adapter.needs_redraw.set(false);
            adapter.renderer.render()
        });
        match result {
            Some(Ok(())) => {
                self.input_region_committed_with_render();
                self.frame_pending = true;
            }
            // A render failure here means the EGL surface/display is gone (the
            // compositor exited) — stop cleanly rather than spin or abort.
            Some(Err(e)) => {
                eprintln!("gnoblin-shell-ui: render failed ({e}); compositor gone — exiting.");
                self.exit = true;
            }
            None => {}
        }
    }

    fn tick(&mut self) {
        if self.app.tick() {
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
        }
        self.input_region_dirty = true;
        // No render here — the main loop renders post-dispatch, once configures
        // are acked (see run()'s loop). Rendering from this timer could commit a
        // buffer while a configure sits unread on the socket.
    }

    /// On a full-height surface, scope the input region to the rects that should
    /// accept pointer input; everything else passes through to windows below.
    /// Priority: passthrough (nothing) → app `input_rects()` (e.g. notification
    /// cards) → whole surface while `input_full()` (open menu/launcher) → the
    /// bar strip.
    fn desired_input_rects(&self) -> Option<Vec<(i32, i32, i32, i32)>> {
        if self.input_passthrough {
            Some(Vec::new())
        } else if let Some(r) = self.app.input_rects() {
            // A custom region is honoured even on a non-full-height bar (e.g.
            // the dock, whose surface includes click-through headroom above
            // the band for its right-click menu).
            Some(r)
        } else if !self.full_height {
            // Non-full-height bar with no custom region: leave the compositor
            // default (the whole fixed-height surface stays interactive).
            None
        } else if self.app.input_full() {
            Some(vec![(0, 0, self.width as i32, self.height as i32)])
        } else {
            Some(vec![(0, 0, self.width as i32, self.input_height as i32)])
        }
    }

    /// Apply pending input-region changes from the main post-dispatch path. This
    /// keeps all layer-surface commits behind the configure-drain barrier.
    fn apply_input_region(&mut self, commit: bool) {
        if !self.input_region_dirty {
            return;
        }
        let rects = self.desired_input_rects();
        if self.input_rects_applied == rects {
            self.input_region_dirty = false;
            return;
        }
        let surface = self.layer.wl_surface();
        if let Some(rects) = rects {
            if let Ok(region) = Region::new(&self.compositor) {
                for (x, y, w, h) in &rects {
                    region.add(*x, *y, *w, *h);
                }
                surface.set_input_region(Some(region.wl_region()));
                if commit {
                    surface.commit();
                    self.input_rects_applied = Some(rects);
                    self.input_region_dirty = false;
                } else {
                    // This pending region is committed by the render path below.
                    // Keep it dirty until that render succeeds so skipped frames
                    // cannot make our bookkeeping lie about hit-test state.
                    self.input_rects_applied = None;
                }
            }
        } else {
            surface.set_input_region(None);
            if commit {
                surface.commit();
                self.input_rects_applied = None;
                self.input_region_dirty = false;
            } else {
                self.input_rects_applied = Some(Vec::new());
            }
        }
    }

    fn input_region_committed_with_render(&mut self) {
        if self.input_region_dirty {
            self.input_rects_applied = self.desired_input_rects();
            self.input_region_dirty = false;
        }
    }
}

fn map_button(code: u32) -> PointerEventButton {
    match code {
        273 => PointerEventButton::Right,
        274 => PointerEventButton::Middle,
        _ => PointerEventButton::Left,
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_scale: i32,
    ) {
        // HiDPI: render the buffer at `new_scale`× so content is crisp + correctly
        // sized on a scaled output, instead of a 1× buffer the compositor upscales.
        let scale = new_scale.max(1) as u32;
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        surface.set_buffer_scale(new_scale.max(1));
        if self.configured {
            self.apply_size();
        }
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        // The throttling callback for our last commit arrived — free to draw the
        // next frame (continuing an animation only if there's still work).
        self.frame_pending = false;
        // The next frame (if an animation is still running) is drawn by the
        // post-dispatch render in run()'s loop.
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(ptr) = self.seat_state.get_pointer(qh, &seat) {
                self.pointer = Some(ptr);
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kbd);
            }
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
        if capability == Capability::Keyboard {
            if let Some(k) = self.keyboard.take() {
                k.release();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

/// Map an sctk key event to the text Slint expects — special keys become their
/// `slint::platform::Key` char; everything else uses the event's UTF-8.
fn key_to_text(event: &KeyEvent) -> Option<slint::SharedString> {
    use slint::platform::Key;
    let special: Option<Key> = match event.keysym {
        Keysym::Escape => Some(Key::Escape),
        Keysym::Return | Keysym::KP_Enter => Some(Key::Return),
        Keysym::BackSpace => Some(Key::Backspace),
        Keysym::Delete => Some(Key::Delete),
        Keysym::Tab => Some(Key::Tab),
        Keysym::Left => Some(Key::LeftArrow),
        Keysym::Right => Some(Key::RightArrow),
        Keysym::Up => Some(Key::UpArrow),
        Keysym::Down => Some(Key::DownArrow),
        Keysym::Home => Some(Key::Home),
        Keysym::End => Some(Key::End),
        _ => None,
    };
    if let Some(k) = special {
        return Some(char::from(k).into());
    }
    event.utf8.clone().filter(|s| !s.is_empty()).map(Into::into)
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if let Some(text) = key_to_text(&event) {
            self.app.key_pressed(&text);
            if let Some(a) = &self.adapter {
                a.needs_redraw.set(true);
            }
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        use PointerEventKind::*;
        let our_surface = self.layer.wl_surface().clone();
        for e in events {
            if e.surface != our_surface {
                continue;
            }
            let pos = LogicalPosition::new(e.position.0 as f32, e.position.1 as f32);
            let ev = match e.kind {
                Enter { .. } | Motion { .. } => Some(WindowEvent::PointerMoved { position: pos }),
                Leave { .. } => Some(WindowEvent::PointerExited),
                Press { button, .. } => Some(WindowEvent::PointerPressed {
                    position: pos,
                    button: map_button(button),
                }),
                Release { button, .. } => Some(WindowEvent::PointerReleased {
                    position: pos,
                    button: map_button(button),
                }),
                Axis {
                    horizontal,
                    vertical,
                    ..
                } => Some(WindowEvent::PointerScrolled {
                    position: pos,
                    delta_x: -horizontal.absolute as f32,
                    delta_y: -vertical.absolute as f32,
                }),
            };
            if let Some(ev) = ev {
                if let Some(window) = self.app.window() {
                    window.dispatch_event(ev);
                    // Slint callbacks can open/close menus/popouts, which changes
                    // BarApp::input_full()/input_rects(). Recompute hit testing in
                    // the same loop instead of waiting for the 100ms app tick.
                    self.input_region_dirty = true;
                    if let Some(a) = &self.adapter {
                        a.needs_redraw.set(true);
                    }
                } else {
                    eprintln!(
                        "gnoblin-shell-ui: Slint app window disappeared during input; exiting."
                    );
                    self.exit = true;
                }
            }
        }
        // Drawn by the post-dispatch render in run()'s loop.
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        self.last_configure_at = Some(Instant::now());
        // The compositor's chosen size. For an edge a dimension it leaves to us
        // comes back 0 — fall back to the output's size so we always span it.
        let (mut w, mut h) = configure.new_size;
        if w == 0 || h == 0 {
            let (ow, oh) = self.screen_size();
            if w == 0 {
                w = ow;
            }
            if h == 0 {
                h = oh;
            }
        }
        if !self.configured {
            self.width = w;
            self.height = h;
            self.configured = true;
            match self.init_slint() {
                Ok(()) => {
                    self.input_region_dirty = true;
                    // init_slint() leaves needs_redraw set; the post-dispatch render in
                    // run()'s loop draws the first frame once this configure is acked.
                }
                Err(e) => {
                    self.startup_error = Some(e.to_string());
                    self.exit = true;
                }
            }
        } else if w != self.width || h != self.height {
            // The output changed size (or sent the real size after a placeholder)
            // — resize the live surface to match instead of staying fixed.
            self.width = w;
            self.height = h;
            self.apply_size();
        }
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_keyboard!(State);
delegate_compositor!(State);
delegate_output!(State);
delegate_layer!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_registry!(State);

fn poll_timeout_ms(timeout: Duration) -> libc::c_int {
    if timeout.is_zero() {
        return 0;
    }
    let millis = timeout.as_millis().max(1);
    millis.min(libc::c_int::MAX as u128) as libc::c_int
}

fn flush_wayland_queue(event_queue: &mut EventQueue<State>) -> Result<(), WaylandError> {
    match event_queue.flush() {
        Err(WaylandError::Io(err)) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        result => result,
    }
}

fn dispatch_wayland(
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    if event_queue.dispatch_pending(state)? > 0 {
        flush_wayland_queue(event_queue)?;
        return Ok(());
    }

    flush_wayland_queue(event_queue)?;

    let Some(guard) = event_queue.prepare_read() else {
        event_queue.dispatch_pending(state)?;
        return Ok(());
    };

    let mut pollfd = libc::pollfd {
        fd: guard.connection_fd().as_raw_fd(),
        events: libc::POLLIN | libc::POLLERR | libc::POLLHUP,
        revents: 0,
    };
    let timeout_ms = poll_timeout_ms(timeout);
    let ready = loop {
        let ready = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if ready >= 0 {
            break ready;
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != ErrorKind::Interrupted {
            drop(guard);
            return Err(Box::new(err));
        }
    };

    if ready > 0 && pollfd.revents != 0 {
        match guard.read() {
            Err(WaylandError::Io(err)) if err.kind() == ErrorKind::WouldBlock => {}
            result => {
                result?;
            }
        }
        event_queue.dispatch_pending(state)?;
    } else {
        drop(guard);
    }

    Ok(())
}

fn run_due_ticks(state: &mut State, next_tick: &mut Instant) {
    let now = Instant::now();
    let mut ticks = 0;
    while now >= *next_tick && ticks < 5 {
        state.tick();
        *next_tick += APP_TICK_INTERVAL;
        ticks += 1;
    }
    if now >= *next_tick {
        *next_tick = now + APP_TICK_INTERVAL;
    }
}

/// Run a Slint `BarApp` as a wlr-layer-shell client until the compositor exits.
pub fn run(config: BarConfig, app: Box<dyn BarApp>) {
    if let Err(e) = try_run(config, app) {
        eprintln!("gnoblin-shell-ui: {e}");
    }
}

fn try_run(config: BarConfig, app: Box<dyn BarApp>) -> Result<(), RuntimeError> {
    // khronos-egl panics (caught in guard_egl) when the compositor disappears —
    // silence their verbose backtrace; keep the default hook for real bugs.
    {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_egl = info
                .location()
                .map(|l| l.file().contains("khronos-egl"))
                .unwrap_or(false);
            if !from_egl {
                default(info);
            }
        }));
    }

    let conn = Connection::connect_to_env()
        .map_err(|e| runtime_error(format!("connect to Wayland: {e}")))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| runtime_error(format!("registry init: {e}")))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wl_compositor: {e}")))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| runtime_error(format!("bind wlr-layer-shell: {e}")))?;

    // Multi-monitor: bind to the output named by `--output` if the compositor
    // launched us for a specific monitor; otherwise let it choose.
    // Explicit --output (per-output panels, window-menu role) wins; otherwise an
    // on-demand client falls back to GNOBLIN_ACTIVE_OUTPUT (the monitor the
    // compositor was focused on when it spawned us). Neither set → compositor picks.
    let target_output = ClientArgs::from_env()
        .output
        .or_else(|| {
            std::env::var("GNOBLIN_ACTIVE_OUTPUT")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .and_then(|name| resolve_output(&conn, &globals, &name));

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        config.layer,
        Some(config.namespace),
        target_output.as_ref(),
    );
    if config.full_height && config.exclusive_zone == 0 {
        // Span the whole output (all edges) so dropdowns can render below the
        // bar; nothing is reserved, so the ambiguous all-edges anchor is fine.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(0, 0);
    } else if config.full_height {
        // A bar that BOTH spans the output (for drop-downs) AND reserves an edge
        // (exclusive_zone): keep the real anchor (so the compositor can resolve
        // which edge the exclusive zone reserves — all-four anchors are
        // ambiguous and sctk has no v5 set_exclusive_edge), and request a huge
        // height that the compositor clamps to the output. We learn the real
        // size back from the configure event.
        layer.set_anchor(config.anchor);
        layer.set_size(0, 1 << 16);
    } else {
        layer.set_anchor(config.anchor);
        layer.set_size(0, config.height);
    }
    layer.set_exclusive_zone(config.exclusive_zone);
    layer.set_keyboard_interactivity(if config.keyboard {
        KeyboardInteractivity::Exclusive
    } else {
        KeyboardInteractivity::None
    });

    let mut state = State {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        compositor,
        layer,
        qh: qh.clone(),
        conn: conn.clone(),
        pointer: None,
        keyboard: None,
        width: 1280,
        height: config.height.max(1),
        scale: 1,
        configured: false,
        exit: false,
        adapter: None,
        app,
        full_height: config.full_height,
        input_height: config.height.max(1),
        input_passthrough: config.input_passthrough,
        input_rects_applied: None,
        input_region_dirty: true,
        frame_pending: false,
        last_configure_at: None,
        startup_error: None,
    };

    // Bind pointer/keyboard resources before the initial layer-surface commit.
    // Exclusive layer surfaces can receive focus as soon as the compositor sees
    // that commit; if wl_keyboard is still unbound, early typed keys in the
    // launcher/window-menu startup path can be dropped before an enter arrives.
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| runtime_error(format!("initial input registry roundtrip: {e}")))?;
    state.layer.commit();

    let mut next_tick = Instant::now();
    while !state.exit {
        let tick_timeout = next_tick.saturating_duration_since(Instant::now());
        let timeout = state.next_dispatch_timeout().min(tick_timeout);
        if dispatch_wayland(&mut event_queue, &mut state, timeout).is_err() {
            break;
        }
        // Drain any wayland events that arrived during/just after the blocking
        // dispatch above (e.g. the burst of placeholder→real-size configures while
        // the output settles at startup) so EVERY pending configure is read + acked
        // by sctk before we attach a buffer below. Without this, a commit can race
        // an unread configure → the compositor posts `cannot attach a buffer before
        // ack_configure` and the client dies.
        for _ in 0..4 {
            if dispatch_wayland(&mut event_queue, &mut state, Duration::ZERO).is_err() {
                state.exit = true;
                break;
            }
            if conn.protocol_error().is_some() {
                state.exit = true;
                break;
            }
        }
        if state.exit {
            break;
        }
        run_due_ticks(&mut state, &mut next_tick);
        state.pump_slint();
        let will_render = state.ready_to_render() && state.wants_redraw();
        state.apply_input_region(!will_render);
        if conn.protocol_error().is_some() {
            break;
        }
        // Render exactly once per iteration, now that all readable configures are
        // acked. (Committing from inside a handler or the timer was the original
        // race; everything just marks dirty and we draw here.)
        if will_render {
            state.render();
        }
        // If a protocol error did slip through anyway, the connection is dead —
        // exit cleanly instead of busy-looping on it.
        if conn.protocol_error().is_some() {
            break;
        }
        if state.app.should_exit() {
            break;
        }
    }
    if let Some(e) = state.startup_error.take() {
        return Err(runtime_error(e));
    }
    Ok(())
}

/// Load the configured wallpaper (`[appearance] wallpaper`), downscale + blur it
/// into a Slint image for de's "glass" backdrop. None when unset/unreadable, in
/// which case the bars fall back to their solid panel/dock background.
/// A human-friendly app name from an app-id: drop `.desktop`, take the segment
/// after the last `.` (reverse-DNS tail), and capitalise — e.g.
/// "org.gnome.Calculator" → "Calculator", "firefox" → "Firefox".
pub fn prettify_app(app_id: &str) -> String {
    let base = app_id
        .strip_suffix(".desktop")
        .unwrap_or(app_id)
        .rsplit('.')
        .next()
        .unwrap_or(app_id);
    if base.is_empty() {
        return String::new();
    }
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn load_backdrop() -> Option<slint::Image> {
    let cfg = config::Config::load();
    let path = cfg.get("appearance", "wallpaper")?;
    let img = image::open(path).ok()?;
    // Downscale first so the Gaussian blur is cheap; Slint stretches it back up.
    let small = img.resize(640, 400, image::imageops::FilterType::Triangle);
    let blurred = image::imageops::blur(&small.to_rgba8(), 18.0);
    let (w, h) = blurred.dimensions();
    let buf =
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&blurred.into_raw(), w, h);
    Some(slint::Image::from_rgba8(buf))
}

/// Local time/date for the topbar clock, without pulling a date crate.
pub fn clock_and_date() -> (String, String) {
    (
        datetime::format_local("%H:%M:%S").unwrap_or_else(|| "00:00:00".to_string()),
        datetime::format_local("%a").unwrap_or_default(),
    )
}
