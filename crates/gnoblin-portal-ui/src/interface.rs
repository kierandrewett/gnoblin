//! The portal interfaces Gnoblin serves (D19 table) and the typed payloads the built-in dialogs render and fill.

use std::fmt;

/// One `org.freedesktop.impl.portal.*` interface, or a method split out of one
/// where the built-in UI and the idempotency differ (`PickColor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interface {
    ScreenCast,
    Screenshot,
    PickColor,
    RemoteDesktop,
    InputCapture,
    GlobalShortcuts,
    Clipboard,
    Settings,
    Inhibit,
    FileChooser,
    AppChooser,
    Print,
    Account,
    Email,
    Notification,
    Wallpaper,
    Lockdown,
    DynamicLauncher,
    Background,
    Secret,
    OpenUri,
    Trash,
}

/// Returned by [`Interface::from_name`] for a name outside the contract table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownInterface(pub String);

impl fmt::Display for UnknownInterface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown portal interface {:?}", self.0)
    }
}

impl std::error::Error for UnknownInterface {}

impl Interface {
    /// Every interface in the contract table, in table order.
    pub const ALL: [Self; 22] = [
        Self::ScreenCast,
        Self::Screenshot,
        Self::PickColor,
        Self::RemoteDesktop,
        Self::InputCapture,
        Self::GlobalShortcuts,
        Self::Clipboard,
        Self::Settings,
        Self::Inhibit,
        Self::FileChooser,
        Self::AppChooser,
        Self::Print,
        Self::Account,
        Self::Email,
        Self::Notification,
        Self::Wallpaper,
        Self::Lockdown,
        Self::DynamicLauncher,
        Self::Background,
        Self::Secret,
        Self::OpenUri,
        Self::Trash,
    ];

    /// The D-Bus interface suffix (`FileChooser` for `org.freedesktop.impl.portal.FileChooser`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ScreenCast => "ScreenCast",
            Self::Screenshot => "Screenshot",
            Self::PickColor => "PickColor",
            Self::RemoteDesktop => "RemoteDesktop",
            Self::InputCapture => "InputCapture",
            Self::GlobalShortcuts => "GlobalShortcuts",
            Self::Clipboard => "Clipboard",
            Self::Settings => "Settings",
            Self::Inhibit => "Inhibit",
            Self::FileChooser => "FileChooser",
            Self::AppChooser => "AppChooser",
            Self::Print => "Print",
            Self::Account => "Account",
            Self::Email => "Email",
            Self::Notification => "Notification",
            Self::Wallpaper => "Wallpaper",
            Self::Lockdown => "Lockdown",
            Self::DynamicLauncher => "DynamicLauncher",
            Self::Background => "Background",
            Self::Secret => "Secret",
            Self::OpenUri => "OpenURI",
            Self::Trash => "Trash",
        }
    }

    /// Parses a contract name back to the interface.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownInterface`] for any name outside the table.
    pub fn from_name(name: &str) -> Result<Self, UnknownInterface> {
        Self::ALL
            .into_iter()
            .find(|interface| interface.name() == name)
            .ok_or_else(|| UnknownInterface(name.to_owned()))
    }
}

/// What the user granted. The dialogs fill it; the broker checks it against the
/// request (`gnoblin_portal_broker::validate_reply`) before it can reach a caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// Consent given (Screenshot, Wallpaper, Background, Secret first use, ...).
    Allowed,
    /// FileChooser: the chosen paths and the filter index in effect.
    Files { paths: Vec<String>, filter: usize },
    /// AppChooser: the offered desktop file id, and whether to remember it.
    App { desktop_file_id: String, remember: bool },
    /// Print: copies and an inclusive page range.
    Print {
        copies: u32,
        first_page: u32,
        last_page: u32,
    },
    /// ScreenCast: the cursor mode and the source count granted.
    Capture { cursor_mode: u32, sources: u32 },
    /// RemoteDesktop: the device types granted (bitmask).
    Devices { device_types: u32 },
}
