//! XDG desktop integration for gnoblin shell clients.

pub mod appmenu;
pub mod desktop_entry;
mod icons;
pub mod tray;

pub use desktop_entry::{
    desktop_actions, installed_desktop_entries, launch_desktop_action, launch_desktop_app,
    resolve_desktop_id, DesktopAction, DesktopEntryFile,
};
pub use icons::{find_icon, find_icon_at_size};
