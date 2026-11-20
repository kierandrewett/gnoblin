//! The table every protocol crate fills in, and the handle it creates its globals through.

use std::sync::Arc;

use smithay::reexports::wayland_server::{Client, DisplayHandle};

use crate::access::{Access, AccessPolicy, Global};

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
    /// A global the roster gates: it decides which clients see it. A privileged one is hidden from every client that is not
    /// privileged or trusted; an open one only from a sandboxed client.
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
    access: &'a Arc<Access>,
}

impl<'a> Registrar<'a> {
    /// A registrar for the session whose display is `display` and whose clients `access` decides about.
    #[must_use]
    pub const fn new(display: &'a DisplayHandle, access: &'a Arc<Access>) -> Self {
        Self { display, access }
    }

    /// The display the globals are created on.
    #[must_use]
    pub const fn display(&self) -> &'a DisplayHandle {
        self.display
    }

    /// The roster that answers which connection may bind a privileged global.
    #[must_use]
    pub const fn access(&self) -> &'a Arc<Access> {
        self.access
    }

    /// The per-client filter of `global`: the one answer the roster gives, in the shape Smithay's states take.
    pub fn filter(&self, global: Global) -> impl for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static {
        let access = Arc::clone(self.access);
        move |client: &Client| AccessPolicy::allows(&*access, &client.id(), global)
    }
}
