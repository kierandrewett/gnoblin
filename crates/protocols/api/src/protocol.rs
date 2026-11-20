//! The table every protocol crate fills in, and the handle it creates its globals through.

use smithay::reexports::wayland_server::DisplayHandle;

use crate::access::Global;

/// One global a protocol advertises: its interface name and the highest version served.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServedInterface {
    /// The interface name in the registry, such as `wp_viewporter`.
    pub name: &'static str,
    /// The highest version the compositor serves. A client may bind any version up to it.
    pub version: u32,
}

impl ServedInterface {
    /// An interface served up to `version`.
    #[must_use]
    pub const fn new(name: &'static str, version: u32) -> Self {
        Self { name, version }
    }
}

/// Which clients may see a global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privilege {
    /// Every client that is not sandboxed sees it, and no roster entry gates it.
    Core,
    /// Every client that is not sandboxed sees it, but the global is gated through the roster so
    /// that a session can turn it off.
    Open,
    /// A privileged global: the roster decides which clients see it, and a client that is not privileged or trusted
    /// does not.
    Roster(Global),
}

/// What one protocol crate says about itself: the one place that names its globals.
///
/// The compositor collects every crate's constant into a table (`gnoblin_wayland_protocols::ALL`).
/// The advertised-globals test compares that table with a live registry, so a crate cannot
/// advertise a version it does not declare, and cannot declare one it does not advertise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolInfo {
    /// The crate's protocol name, such as `wp-viewporter`.
    pub name: &'static str,
    /// The globals the protocol creates, in the order it creates them.
    pub interfaces: &'static [ServedInterface],
    /// Who may see them.
    pub privilege: Privilege,
}

/// The handle a protocol crate creates its globals through.
///
/// The compositor builds one while it assembles a session. It carries what every crate needs to
/// create a global, so adding a new need is one field here and no change to a crate that does not
/// use it.
#[derive(Clone, Copy, Debug)]
pub struct Registrar<'a> {
    display: &'a DisplayHandle,
}

impl<'a> Registrar<'a> {
    /// A registrar for the session whose display is `display`.
    #[must_use]
    pub const fn new(display: &'a DisplayHandle) -> Self {
        Self { display }
    }

    /// The display the globals are created on.
    #[must_use]
    pub const fn display(&self) -> &'a DisplayHandle {
        self.display
    }
}
