//! XML protocol `appmenu` (KDE): the interfaces `org_kde_kwin_appmenu_manager` and `org_kde_kwin_appmenu`.
//!
//! A client with an application menu (Qt with the platform theme's global menu, GTK with `appmenu-gtk-module`,
//! LibreOffice) exports it on the session bus as a `com.canonical.dbusmenu` object and tells the compositor where:
//! a DBus service name and an object path, for one `wl_surface`. The compositor draws no menu. It publishes the
//! address on the window's node as `appmenu_service` and `appmenu_path`, and a shell that shows a global menu
//! connects to it and reads what the object exports. A property change is the tree's own change event, so the shell
//! hears a new or a withdrawn menu as it hears any other change of the window.
//!
//! ## Lifetime
//!
//! The address belongs to the client's `org_kde_kwin_appmenu` object, not to the surface and not to the window:
//!
//! - `set_address` sets it, and a second one replaces it.
//! - `release`, or a client that went away, withdraws it. The node's two properties return to the schema's default,
//!   empty, so a shell never connects to a menu whose owner is gone.
//! - An object made for a surface that has no toplevel yet keeps its address and applies it when the toplevel appears
//!   ([`toplevel_created`]); a toplevel destroyed takes its node, and the properties, with it.
//!
//! A client that makes a second object for a surface it already has one for is served: the address that was set last
//! is the one published, and withdrawing the object that set it leaves the other's address in force.
//!
//! ## What an address may be
//!
//! The protocol says both strings are DBus names "in Latin-1 matching the DBus specifications", and the shell will
//! open a bus connection to the service, so an address that is not one is dropped and logged: a service is a unique
//! name (`:1.42`) or a well-known name (two or more dot-separated elements of letters, digits, `_` and `-`, the first
//! not a digit), at most 255 bytes; a path is `/` or `/` followed by elements of letters, digits and `_` separated by
//! single `/`. The protocol has no error to answer an invalid address with, so nothing is posted: the object keeps
//! its previous address, which is what a client that raced its own bus registration expects.
//!
//! ## Failure modes
//!
//! Each is a test in `tests/kde_appmenu.rs`, except the last, which the wire guarantees:
//!
//! 1. **The address never reaches the shell**: `a_set_address_is_published_on_the_window_node`.
//! 2. **A replaced address leaves the old one behind**: `a_second_address_replaces_the_first`.
//! 3. **A menu outlives its object**: `releasing_the_appmenu_clears_the_node`; a second object's address stays in
//!    force when the first is released: `releasing_one_of_two_objects_keeps_the_others_address`. A client that
//!    disconnects takes its window and so its node, and the object with it.
//! 4. **An object made before the toplevel is lost**: `an_address_set_before_the_toplevel_is_applied_to_it`.
//! 5. **A malformed address reaches a bus connection**: `an_invalid_address_is_dropped`, and the unit tests below.
//! 6. **Another client's window can be given a menu**: the object is made for the surface the client names, and a
//!    surface of another client cannot be named: the connection's object ids are its own.

use smithay::reexports::wayland_server::backend::{ClientId, GlobalId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use wayland_protocols_plasma::appmenu::server::org_kde_kwin_appmenu::{self as appmenu, OrgKdeKwinAppmenu};
use wayland_protocols_plasma::appmenu::server::org_kde_kwin_appmenu_manager::{
    self as manager, OrgKdeKwinAppmenuManager,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// The version of `org_kde_kwin_appmenu_manager` this session serves.
pub const MANAGER_VERSION: u32 = 2;

/// What this protocol advertises. Every client may bind it: it names a menu on the client's own surface.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "org-kde-kwin-appmenu",
    interfaces: &[ServedInterface::new("org_kde_kwin_appmenu_manager", MANAGER_VERSION)],
    privilege: Privilege::Core,
};

/// The longest name DBus allows: 255 bytes.
const MAX_NAME: usize = 255;

/// What this protocol asks of the compositor.
pub trait Host: Sized + 'static {
    /// Publishes the address on `surface`'s window node; nothing happens for a surface with no toplevel yet. An
    /// empty service and path withdraw it.
    fn publish_appmenu(&mut self, surface: &WlSurface, service: &str, path: &str);
    /// The protocol's bookkeeping.
    fn appmenus(&mut self) -> &mut KdeAppmenus;
}

/// One appmenu object a client made.
struct AppmenuRecord {
    /// The surface it belongs to.
    surface: WlSurface,
    /// The object, to tell it from the client's others.
    object: OrgKdeKwinAppmenu,
    /// The address the object set: its DBus service and object path.
    address: Option<(String, String)>,
    /// When the address was set, so the latest of two objects' addresses is the published one.
    stamp: u64,
}

/// The global and the objects clients made.
pub struct KdeAppmenus {
    /// Kept alive so `org_kde_kwin_appmenu_manager` stays registered.
    #[allow(dead_code)]
    global: GlobalId,
    /// Every live appmenu object.
    records: Vec<AppmenuRecord>,
    /// The next stamp.
    clock: u64,
}

impl KdeAppmenus {
    /// The address in force for `surface`: the one set last by an object that still lives.
    fn address_of(&self, surface: &WlSurface) -> Option<&(String, String)> {
        self.records
            .iter()
            .filter(|record| record.surface == *surface)
            .filter_map(|record| record.address.as_ref().map(|address| (record.stamp, address)))
            .max_by_key(|(stamp, _)| *stamp)
            .map(|(_, address)| address)
    }
}

/// The user data of a bound manager.
pub struct OrgKdeKwinAppmenuManagerUserData;

/// The user data of one appmenu object: the surface it belongs to.
pub struct AppmenuUserData {
    surface: WlSurface,
}

/// The global data of the manager.
pub struct OrgKdeKwinAppmenuManagerGlobalData;

/// Creates the `org_kde_kwin_appmenu_manager` global.
pub fn register<D>(registrar: &Registrar<'_>) -> KdeAppmenus
where
    D: GlobalDispatch<OrgKdeKwinAppmenuManager, OrgKdeKwinAppmenuManagerGlobalData> + 'static,
{
    KdeAppmenus {
        global: registrar
            .display()
            .create_global::<D, OrgKdeKwinAppmenuManager, _>(MANAGER_VERSION, OrgKdeKwinAppmenuManagerGlobalData),
        records: Vec::new(),
        clock: 0,
    }
}

/// Publishes whatever address is in force for `surface`, or withdraws it when there is none.
fn republish<H: Host>(host: &mut H, surface: &WlSurface) {
    let (service, path) = host.appmenus().address_of(surface).cloned().unwrap_or_default();
    host.publish_appmenu(surface, &service, &path);
}

/// A toplevel was created: an address an object set for its surface before then is published now.
pub fn toplevel_created<H: Host>(host: &mut H, surface: &WlSurface) {
    if host.appmenus().address_of(surface).is_some() {
        republish(host, surface);
    }
}

/// Whether `name` is a DBus bus name: a unique name (`:` and elements) or a well-known one (two or more elements).
fn is_bus_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME || !name.is_ascii() {
        return false;
    }
    let (unique, rest) = match name.strip_prefix(':') {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    let elements: Vec<&str> = rest.split('.').collect();
    if elements.len() < 2 {
        return false;
    }
    elements.iter().enumerate().all(|(index, element)| {
        !element.is_empty()
            && element.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            // A well-known name's first element may not start with a digit; the other elements of any name may.
            && (unique || index > 0 || !element.as_bytes()[0].is_ascii_digit())
    })
}

/// Whether `path` is a DBus object path.
fn is_object_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    path.len() <= 4096
        && rest.split('/').all(|element| {
            !element.is_empty() && element.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

impl<D> GlobalDispatch2<OrgKdeKwinAppmenuManager, D> for OrgKdeKwinAppmenuManagerGlobalData
where
    D: Dispatch<OrgKdeKwinAppmenuManager, OrgKdeKwinAppmenuManagerUserData> + 'static,
{
    fn bind(
        &self,
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, OrgKdeKwinAppmenuManagerUserData);
    }
}

impl<D> Dispatch2<OrgKdeKwinAppmenuManager, D> for OrgKdeKwinAppmenuManagerUserData
where
    D: Host + Dispatch<OrgKdeKwinAppmenu, AppmenuUserData> + 'static,
{
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        _manager: &OrgKdeKwinAppmenuManager,
        request: manager::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        // `release` is the manager's destructor: Smithay destroys the object.
        if let manager::Request::Create { id, surface } = request {
            let object = data_init.init(
                id,
                AppmenuUserData {
                    surface: surface.clone(),
                },
            );
            state.appmenus().records.push(AppmenuRecord {
                surface,
                object,
                address: None,
                stamp: 0,
            });
            log::debug!(area = "appmenu", "a client made an appmenu object");
        }
    }
}

impl<D: Host> Dispatch2<OrgKdeKwinAppmenu, D> for AppmenuUserData {
    fn request(
        &self,
        state: &mut D,
        _client: &Client,
        object: &OrgKdeKwinAppmenu,
        request: appmenu::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        if let appmenu::Request::SetAddress {
            service_name,
            object_path,
        } = request
        {
            if !is_bus_name(&service_name) || !is_object_path(&object_path) {
                log::warn!(
                    area = "appmenu",
                    service = %service_name,
                    path = %object_path,
                    "an appmenu address that is not a DBus name and object path was dropped"
                );
                return;
            }
            let menus = state.appmenus();
            menus.clock += 1;
            let stamp = menus.clock;
            if let Some(record) = menus.records.iter_mut().find(|record| record.object == *object) {
                record.address = Some((service_name, object_path));
                record.stamp = stamp;
            }
            republish(state, &self.surface);
        }
    }

    fn destroyed(&self, state: &mut D, _client: ClientId, object: &OrgKdeKwinAppmenu) {
        state.appmenus().records.retain(|record| record.object != *object);
        republish(state, &self.surface);
    }
}

#[cfg(test)]
mod tests {
    use super::{is_bus_name, is_object_path};

    #[test]
    fn a_unique_name_and_a_well_known_name_are_bus_names() {
        assert!(is_bus_name(":1.42"));
        assert!(is_bus_name("org.kde.kwin"));
        assert!(is_bus_name("com.canonical.AppMenu-Registrar"));
        assert!(is_bus_name("org.gnome.Terminal_1"));
    }

    #[test]
    fn a_name_that_is_not_a_bus_name_is_refused() {
        assert!(!is_bus_name(""));
        assert!(!is_bus_name("nodots"), "a well-known name has two elements at least");
        assert!(!is_bus_name("org..kde"), "an element is not empty");
        assert!(
            !is_bus_name("1org.kde"),
            "a well-known name's first element does not start with a digit"
        );
        assert!(!is_bus_name("org.kde space"));
        assert!(!is_bus_name("org.kde/path"));
        assert!(!is_bus_name("org.kdé.kwin"), "ASCII only");
        assert!(!is_bus_name(&format!("org.{}", "a".repeat(260))), "at most 255 bytes");
    }

    #[test]
    fn a_path_of_elements_is_an_object_path() {
        assert!(is_object_path("/"));
        assert!(is_object_path("/MenuBar/1"));
        assert!(is_object_path("/com/canonical/menu/_1A"));
    }

    #[test]
    fn a_path_that_is_not_an_object_path_is_refused() {
        assert!(!is_object_path(""));
        assert!(!is_object_path("MenuBar"));
        assert!(!is_object_path("/MenuBar/"), "no trailing slash");
        assert!(!is_object_path("//x"), "no empty element");
        assert!(!is_object_path("/menu-bar"), "only letters, digits and underscores");
    }
}
