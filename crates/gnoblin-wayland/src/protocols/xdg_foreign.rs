//! XML protocol `xdg_foreign_unstable_v2`; interfaces served: `zxdg_exporter_v2`, `zxdg_importer_v2`.
//!
//! Gnoblin's own implementation of xdg-foreign v2. One client names one of its toplevels by an
//! unguessable handle; another client imports the handle and makes that toplevel the parent of one
//! of its own toplevels. The compositor owns the handle table, so the portal `parent_window` lookup
//! can resolve a handle to its surface through [`XdgForeign::exported_surface`].
//!
//! The relation reaches the window manager the same way `xdg_toplevel.set_parent` does: the child's
//! `XdgToplevelSurfaceData.parent` is set and [`State::sync_window_role`] sends a `WindowRoleChanged`,
//! so the child floats above its parent.
//!
//! # Lifecycles
//!
//! An export moves through one transition:
//!
//! | from | event | to |
//! |---|---|---|
//! | `Exported` | `zxdg_exported_v2.destroy`, or the exported surface is destroyed | `Revoked` |
//!
//! `Revoked` is terminal. The record stays as a tombstone so the handle is never minted again and
//! an import of it is answered `destroyed`.
//!
//! An import moves through these states:
//!
//! | from | event | to |
//! |---|---|---|
//! | (new) | `import_toplevel` names a handle no live export holds | `Orphaned`, `destroyed` sent at once |
//! | (new) | `import_toplevel` names a live export | `Imported` |
//! | `Imported` or `Parented` | `set_parent_of` succeeds | `Parented` |
//! | `Imported` or `Parented` | the export is revoked | `Orphaned`, `destroyed` sent, children cleared |
//! | any | `zxdg_imported_v2.destroy` | removed; children cleared when still `Parented` |
//!
//! `set_parent_of` on an `Orphaned` import is accepted and sets nothing.
//!
//! # Failure modes
//!
//! 1. **Export of a surface that is not a toplevel.** `zxdg_exporter_v2.invalid_surface`; nothing
//!    is exported.
//! 2. **`set_parent_of` naming a child that is not a toplevel.** `zxdg_imported_v2.invalid_surface`.
//! 3. **`set_parent_of` naming a parent that would loop, or the child itself.** Refused with
//!    `zxdg_imported_v2.invalid_surface` by Smithay's `is_valid_parent`; the child keeps its
//!    previous parent.
//! 4. **Import of a handle no live export holds** (never minted, or revoked). The new import
//!    receives `destroyed` at once and records no relation.
//! 5. **The export is destroyed or its surface goes away.** Every import of the handle receives
//!    `destroyed` and its children lose the parent, so the window manager sees it cleared.
//! 6. **A handle is not guessable.** It is 128 bits from the kernel's random source, rendered as 32
//!    lowercase hex digits.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use gnoblin_log::debug_in;
use smithay::reexports::wayland_protocols::xdg::foreign::zv2::server::{
    zxdg_exported_v2::{self, ZxdgExportedV2},
    zxdg_exporter_v2::{self, ZxdgExporterV2},
    zxdg_imported_v2::{self, ZxdgImportedV2},
    zxdg_importer_v2::{self, ZxdgImporterV2},
};
use smithay::reexports::wayland_server::backend::{ClientId, GlobalId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::compositor;
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use smithay::wayland::shell::{is_toplevel_equivalent, is_valid_parent};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use crate::State;

/// The version of `zxdg_exporter_v2` and `zxdg_importer_v2` this compositor serves.
pub const VERSION: u32 = 1;

/// Length of a handle's random part, in bytes. The handle is twice this many hex digits.
const HANDLE_BYTES: usize = 16;

/// Creates the `zxdg_exporter_v2` and `zxdg_importer_v2` globals.
///
/// Returns both ids so the caller keeps them registered for the life of the display.
pub fn global(display: &DisplayHandle) -> (GlobalId, GlobalId) {
    let exporter = display.create_global::<State, ZxdgExporterV2, _>(VERSION, XdgForeignGlobalData);
    let importer = display.create_global::<State, ZxdgImporterV2, _>(VERSION, XdgForeignGlobalData);
    (exporter, importer)
}

/// The global data of both interfaces. Neither is gated by the access roster, so it carries nothing.
pub struct XdgForeignGlobalData;

/// The user data of a bound `zxdg_exporter_v2`.
pub struct XdgForeignExporterData;

/// The user data of a bound `zxdg_importer_v2`.
pub struct XdgForeignImporterData;

/// The user data of a `zxdg_exported_v2`: the handle it minted, which names its record.
pub struct XdgForeignExportedData {
    /// The handle sent in the `handle` event; the key of the export record.
    handle: String,
}

/// The user data of a `zxdg_imported_v2`: the handle it was made with.
pub struct XdgForeignImportedData {
    /// The handle the client imported; the key of the export it refers to.
    handle: String,
}

/// The lifecycle of an export. `Revoked` is terminal; see the module's transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignExportState {
    /// The handle names a live surface and imports may use it.
    Exported,
    /// The export was destroyed or its surface went away. The handle names nothing.
    Revoked,
}

impl ForeignExportState {
    /// Moves an export to `Revoked`. Returns whether it changed, so a second revoke (the destroy
    /// request and the surface's destruction can both run) does nothing.
    fn revoke(&mut self) -> bool {
        match self {
            Self::Exported => {
                *self = Self::Revoked;
                true
            }
            Self::Revoked => false,
        }
    }
}

/// The lifecycle of an import. See the module's transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignImportState {
    /// The import names a live export and has not set a parent.
    Imported,
    /// The import has set a parent on at least one child.
    Parented,
    /// The handle is gone: it never named a live export, or the export was revoked.
    Orphaned,
}

/// One export record, keyed by its handle.
struct ForeignExport {
    /// The exported toplevel. Its destruction hook revokes the export, so the handle never names a
    /// dead surface.
    surface: WlSurface,
    /// Where the export is in its lifecycle.
    state: ForeignExportState,
}

/// One import record, keyed by the imported object's id.
struct ForeignImport {
    /// The object the client holds; kept so `destroyed` can be sent when the export goes.
    object: ZxdgImportedV2,
    /// The handle the import was made with.
    handle: String,
    /// Where the import is in its lifecycle.
    state: ForeignImportState,
    /// The children this import set a parent on. Cleared when the import is revoked or destroyed,
    /// so each child's parent is taken back in one place.
    children: Vec<WlSurface>,
}

/// The handle table: every export, every import, and the surfaces whose destruction it watches.
#[derive(Default)]
pub struct XdgForeign {
    /// Every export ever made, by handle. Revoked exports stay as tombstones so a handle is never
    /// minted twice.
    exports: HashMap<String, ForeignExport>,
    /// Every live import, by object id. An import is removed when its object is destroyed.
    imports: HashMap<ObjectId, ForeignImport>,
    /// Surfaces that already carry a destruction hook from this table, so each surface is hooked
    /// once however many exports and children it has.
    hooked: HashSet<ObjectId>,
}

impl XdgForeign {
    /// The surface a live handle names, or `None` for a handle never minted, revoked, or whose
    /// surface has gone.
    ///
    /// This is the compositor's lookup, for code outside the protocol such as the portal
    /// `parent_window` mapping. Clients reach a handle only through `zxdg_importer_v2`.
    pub fn exported_surface(&self, handle: &str) -> Option<&WlSurface> {
        self.exports
            .get(handle)
            .filter(|export| export.state == ForeignExportState::Exported && export.surface.is_alive())
            .map(|export| &export.surface)
    }
}

/// Mints a handle no export holds: 128 random bits as 32 lowercase hex digits.
///
/// The random source is the kernel's, so a handle cannot be predicted from earlier ones. A
/// collision with any existing record, live or tombstoned, is redrawn.
fn mint_handle(exports: &HashMap<String, ForeignExport>) -> String {
    loop {
        let mut bytes = [0_u8; HANDLE_BYTES];
        getrandom::fill(&mut bytes).expect("the kernel supplies randomness for a handle");
        let mut handle = String::with_capacity(HANDLE_BYTES * 2);
        for byte in bytes {
            // Writing into a String cannot fail.
            let _ = write!(handle, "{byte:02x}");
        }
        if !exports.contains_key(&handle) {
            return handle;
        }
    }
}

/// Registers the surface's destruction hook once, so the table learns when the surface goes and can
/// revoke the exports and children that name it.
fn watch(state: &mut State, surface: &WlSurface) {
    if state.xdg_foreign.hooked.insert(surface.id()) {
        compositor::add_destruction_hook::<State, _>(surface, surface_destroyed);
    }
}

/// Called when a watched `wl_surface` is destroyed.
///
/// Revokes every live export whose surface this is, which sends `destroyed` to its importers, and
/// drops this surface from every import's children (its window is going away, so there is no
/// parent to clear).
fn surface_destroyed(state: &mut State, surface: &WlSurface) {
    state.xdg_foreign.hooked.remove(&surface.id());
    let exported: Vec<String> = state
        .xdg_foreign
        .exports
        .iter()
        .filter(|(_, export)| export.state == ForeignExportState::Exported && export.surface == *surface)
        .map(|(handle, _)| handle.clone())
        .collect();
    for handle in exported {
        revoke_export(state, &handle);
    }
    for import in state.xdg_foreign.imports.values_mut() {
        import.children.retain(|child| child != surface);
    }
}

/// Revokes an export: marks it revoked, sends `destroyed` to every live import of its handle, and
/// takes the parent back from each child those imports set.
///
/// A second call for the same handle does nothing.
fn revoke_export(state: &mut State, handle: &str) {
    let Some(export) = state.xdg_foreign.exports.get_mut(handle) else {
        return;
    };
    if !export.state.revoke() {
        return;
    }
    let parent = export.surface.clone();
    debug_in!("wayland", handle = %handle, "[xdg-foreign] an export was revoked");

    let importers: Vec<ObjectId> = state
        .xdg_foreign
        .imports
        .iter()
        .filter(|(_, import)| import.handle == handle && import.state != ForeignImportState::Orphaned)
        .map(|(id, _)| id.clone())
        .collect();
    for id in importers {
        let Some(import) = state.xdg_foreign.imports.get_mut(&id) else {
            continue;
        };
        let children = std::mem::take(&mut import.children);
        import.state = ForeignImportState::Orphaned;
        let object = import.object.clone();
        for child in &children {
            clear_parent(state, child, &parent);
        }
        object.destroyed();
    }
}

/// Clears `child`'s parent if it is still `expected`, then tells the window manager.
///
/// The check guards a child a client has since parented another way: only the relation this table
/// made is undone.
fn clear_parent(state: &mut State, child: &WlSurface, expected: &WlSurface) {
    let cleared = compositor::with_states(child, |states| {
        let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() else {
            return false;
        };
        // A poisoned lock means another thread panicked while holding the role. The data is still
        // the last value written, so take it rather than abort the connection.
        let mut role = data.lock().unwrap_or_else(|error| error.into_inner());
        if role.parent.as_ref() == Some(expected) {
            role.parent = None;
            true
        } else {
            false
        }
    });
    if cleared && let Some(window) = state.window_of(child) {
        state.sync_window_role(window);
    }
}

/// Makes `parent` the parent of `child` and tells the window manager.
///
/// The child is taken off any other import's children first, so a child is listed by at most one
/// import.
fn set_parent(state: &mut State, import: &ObjectId, child: &WlSurface, parent: &WlSurface) {
    for (id, other) in state.xdg_foreign.imports.iter_mut() {
        if id != import {
            other.children.retain(|listed| listed != child);
        }
    }

    compositor::with_states(child, |states| {
        if let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>() {
            let mut role = data.lock().unwrap_or_else(|error| error.into_inner());
            role.parent = Some(parent.clone());
        }
    });
    if let Some(record) = state.xdg_foreign.imports.get_mut(import) {
        if !record.children.contains(child) {
            record.children.push(child.clone());
        }
        record.state = ForeignImportState::Parented;
    }
    watch(state, child);
    if let Some(window) = state.window_of(child) {
        state.sync_window_role(window);
    }
}

impl GlobalDispatch2<ZxdgExporterV2, State> for XdgForeignGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgExporterV2>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, XdgForeignExporterData);
    }
}

impl GlobalDispatch2<ZxdgImporterV2, State> for XdgForeignGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZxdgImporterV2>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, XdgForeignImporterData);
    }
}

impl Dispatch2<ZxdgExporterV2, State> for XdgForeignExporterData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZxdgExporterV2,
        request: zxdg_exporter_v2::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zxdg_exporter_v2::Request::ExportToplevel { id, surface } => {
                if !is_toplevel_equivalent(&surface) {
                    resource.post_error(
                        zxdg_exporter_v2::Error::InvalidSurface,
                        "exported surface is not an xdg_toplevel",
                    );
                    return;
                }
                let handle = mint_handle(&state.xdg_foreign.exports);
                let exported = data_init.init(id, XdgForeignExportedData { handle: handle.clone() });
                exported.handle(handle.clone());
                watch(state, &surface);
                state.xdg_foreign.exports.insert(
                    handle.clone(),
                    ForeignExport {
                        surface,
                        state: ForeignExportState::Exported,
                    },
                );
                debug_in!("wayland", handle = %handle, "[xdg-foreign] a toplevel was exported");
            }
            zxdg_exporter_v2::Request::Destroy => {}
            // The request enums are non-exhaustive: a request from a newer revision is ignored.
            _ => {}
        }
    }
}

impl Dispatch2<ZxdgImporterV2, State> for XdgForeignImporterData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZxdgImporterV2,
        request: zxdg_importer_v2::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zxdg_importer_v2::Request::ImportToplevel { id, handle } => {
                let live = state.xdg_foreign.exported_surface(&handle).is_some();
                let object = data_init.init(id, XdgForeignImportedData { handle: handle.clone() });
                let key = object.id();
                if live {
                    state.xdg_foreign.imports.insert(
                        key,
                        ForeignImport {
                            object,
                            handle,
                            state: ForeignImportState::Imported,
                            children: Vec::new(),
                        },
                    );
                } else {
                    // A handle no live export holds: the import is destroyed at once and kept as an
                    // orphan so its later requests are answered without making a relation.
                    object.destroyed();
                    state.xdg_foreign.imports.insert(
                        key,
                        ForeignImport {
                            object,
                            handle,
                            state: ForeignImportState::Orphaned,
                            children: Vec::new(),
                        },
                    );
                }
            }
            zxdg_importer_v2::Request::Destroy => {}
            // The request enums are non-exhaustive: a request from a newer revision is ignored.
            _ => {}
        }
    }
}

impl Dispatch2<ZxdgExportedV2, State> for XdgForeignExportedData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZxdgExportedV2,
        _request: zxdg_exported_v2::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // The only request is `destroy`, which revokes in `destroyed`.
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, _resource: &ZxdgExportedV2) {
        revoke_export(state, &self.handle);
    }
}

impl Dispatch2<ZxdgImportedV2, State> for XdgForeignImportedData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZxdgImportedV2,
        request: zxdg_imported_v2::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zxdg_imported_v2::Request::SetParentOf { surface: child } => {
                if !is_toplevel_equivalent(&child) {
                    resource.post_error(
                        zxdg_imported_v2::Error::InvalidSurface,
                        "the child surface is not an xdg_toplevel",
                    );
                    return;
                }
                // An import whose export is gone is orphaned: it accepts the request but sets nothing.
                let Some(parent) = state.xdg_foreign.exported_surface(&self.handle).cloned() else {
                    debug_in!("wayland", handle = %self.handle, "[xdg-foreign] set_parent_of on an orphaned import");
                    return;
                };
                if !is_valid_parent(&child, &parent) {
                    resource.post_error(zxdg_imported_v2::Error::InvalidSurface, "invalid parent relationship");
                    return;
                }
                set_parent(state, &resource.id(), &child, &parent);
            }
            zxdg_imported_v2::Request::Destroy => {}
            // The request enums are non-exhaustive: a request from a newer revision is ignored.
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &ZxdgImportedV2) {
        let Some(import) = state.xdg_foreign.imports.remove(&resource.id()) else {
            return;
        };
        // Destroying the import takes back the relations it set. The export itself stays live.
        if let Some(parent) = state.xdg_foreign.exported_surface(&import.handle).cloned() {
            for child in &import.children {
                clear_parent(state, child, &parent);
            }
        }
    }
}
