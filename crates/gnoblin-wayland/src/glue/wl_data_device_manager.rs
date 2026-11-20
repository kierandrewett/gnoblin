//! `gnoblin-wayland-wl-data-device-manager`: the selection handlers are the state's, because a paste is a decision between
//! a client's source, the X11 window manager and the host's clipboard.

use std::os::unix::io::OwnedFd;

use smithay::input::Seat;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, clear_data_device_selection,
    request_data_device_client_selection, set_data_device_selection,
};
use smithay::wayland::selection::primary_selection::{
    clear_primary_selection, request_primary_client_selection, set_primary_selection,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};

use gnoblin_backend::selection::{self as bridge, SelectionTarget as BackendTarget};

use crate::State;
use crate::selection_bridge::{PasteRoute, ToX11};
use crate::xwayland::set_x11_selection_owner;

use crate::log;

impl SelectionHandler for State {
    /// Nothing outside the selection itself needs to be carried along with it: the
    /// source is stored here and the bytes never leave the two clients.
    type SelectionUserData = ();

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, seat: Seat<Self>) {
        let _ = &seat;
        // Captured now, while the source's client is certainly there: asking later
        // would be asking a resource that may be gone.
        let mime_types = source.as_ref().map(SelectionSource::mime_types);
        match &mime_types {
            Some(mime_types) => log::debug!(area = "selection", ?ty, ?mime_types, "set"),
            None => log::debug!(area = "selection", ?ty, "cleared"),
        }
        self.record_selection(ty, mime_types.clone());
        // X11 clients learn of the new Wayland selection through the window manager, so
        // they can ask for its data. A change with no X11 window manager is recorded only.
        let allowed = self.selection_allowed();
        let change = self
            .selection_owners
            .wayland_changed(ty, mime_types.as_deref(), allowed);
        // The host of a nested session is told too: what it can be offered is narrowed in `announce_to_backend`.
        self.announce_to_backend(ty, mime_types.as_deref());
        self.forward_to_x11(ty, change);
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let _ = (user_data, &seat);
        // A client-set selection never reaches here: Smithay forwards that `receive` to
        // the source's own client, which is what keeps the bytes out of the compositor.
        // Here the selection is compositor-set, which only happens when an X11 client
        // owns it, so the bytes come from that client through the window manager.
        let allowed = self.selection_allowed();
        match self.selection_owners.paste_route(ty, allowed) {
            PasteRoute::X11 => match self.x11_wm() {
                Some(wm) => {
                    if let Err(error) = wm.send_selection(ty, mime_type, fd) {
                        log::warn!(area = "selection", ?ty, ?error, "the X11 paste could not be served");
                    }
                }
                None => drop(fd),
            },
            // The bytes are the host's: the loop hands the descriptor to the backend, which reads the host's selection
            // into it and ends it when the host has no more or the limit is reached.
            PasteRoute::Host => self.backend_selection.push(BackendSelection::Read {
                target: backend_target(ty),
                mime_type,
                fd,
            }),
            // Dropping the write end is an empty read, the honest answer to a refused paste.
            PasteRoute::Refuse => drop(fd),
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

/// A drag and drop started by a client.
///
/// Interactive drags belong to the input slice: a drag that arrived without a pointer
/// grab cannot be finished by anybody, so the default (cancel the source) is what this
/// slice states explicitly rather than leaving to a silent no-op.
impl WaylandDndGrabHandler for State {}

/// What the loop has to tell or ask the backend about the selections, after a turn.
#[derive(Debug)]
pub(crate) enum BackendSelection {
    /// A client of the session set (`Some`) or cleared (`None`) a selection: the types that cross.
    Session {
        target: BackendTarget,
        mime_types: Option<Vec<String>>,
    },
    /// A client of the session pastes the host's selection into `fd`.
    Read {
        target: BackendTarget,
        mime_type: String,
        fd: OwnedFd,
    },
}

/// The backend's name for a Smithay selection target.
pub(crate) fn backend_target(target: SelectionTarget) -> BackendTarget {
    match target {
        SelectionTarget::Clipboard => BackendTarget::Clipboard,
        SelectionTarget::Primary => BackendTarget::Primary,
    }
}

/// Smithay's name for a backend selection target.
fn smithay_target(target: BackendTarget) -> SelectionTarget {
    match target {
        BackendTarget::Clipboard => SelectionTarget::Clipboard,
        BackendTarget::Primary => SelectionTarget::Primary,
    }
}

impl State {
    /// Takes what the backend has to be told, in the order it happened.
    pub(crate) fn take_backend_selection(&mut self) -> Vec<BackendSelection> {
        std::mem::take(&mut self.backend_selection)
    }

    /// Tells the backend a client of the session set or cleared a selection, narrowed to the types that cross. A
    /// locked session offers nothing.
    pub(crate) fn announce_to_backend(&mut self, ty: SelectionTarget, mime_types: Option<&[String]>) {
        let allowed = self.selection_allowed();
        let bridged = mime_types
            .filter(|_| allowed)
            .map(bridge::bridged_mimes)
            .filter(|types| !types.is_empty());
        self.backend_selection.push(BackendSelection::Session {
            target: backend_target(ty),
            mime_types: bridged,
        });
    }

    /// Applies what X11 must be told after a change on the Wayland side.
    pub(crate) fn forward_to_x11(&mut self, ty: SelectionTarget, change: ToX11) {
        let Some(wm) = self.x11_wm() else {
            return;
        };
        let result = match change {
            ToX11::Advertise(types) => set_x11_selection_owner(wm, ty, Some(types)),
            ToX11::Withdraw => set_x11_selection_owner(wm, ty, None),
            ToX11::Nothing => return,
        };
        if let Err(error) = result {
            log::warn!(
                area = "selection",
                ?ty,
                ?error,
                "the X11 selection could not be updated"
            );
        }
    }

    /// The host compositor's selection changed, or went with its keyboard focus: a compositor-owned source offering what
    /// crosses, or none.
    pub(crate) fn host_selection_changed(&mut self, target: BackendTarget, mime_types: Option<Vec<String>>) {
        let ty = smithay_target(target);
        let allowed = self.selection_allowed();
        // The backend narrows too; the compositor does not trust that to hold what it will offer its own clients.
        let types = mime_types.map(|offered| bridge::bridged_mimes(&offered));
        let was_host = self.selection_owners.host_owns(ty);
        let change = self.selection_owners.host_changed(ty, types.as_deref(), allowed);
        if self.selection_owners.host_owns(ty) {
            if let Some(types) = types {
                match ty {
                    SelectionTarget::Clipboard => {
                        set_data_device_selection(&self.display_handle, &self.seat, types.clone(), ());
                    }
                    SelectionTarget::Primary => {
                        set_primary_selection(&self.display_handle, &self.seat, types.clone(), ());
                    }
                }
                log::debug!(area = "selection", ?ty, mime_types = ?types, "the host's selection is offered");
                self.record_selection(ty, Some(types));
            }
        } else if was_host {
            match ty {
                SelectionTarget::Clipboard => clear_data_device_selection(&self.display_handle, &self.seat),
                SelectionTarget::Primary => clear_primary_selection(&self.display_handle, &self.seat),
            }
            log::debug!(area = "selection", ?ty, "the host's selection is withdrawn");
            self.record_selection(ty, None);
        }
        self.forward_to_x11(ty, change);
    }

    /// The host compositor pastes the session's selection as `mime_type` into `fd`.
    pub(crate) fn session_selection_read(&mut self, target: BackendTarget, mime_type: String, fd: OwnedFd) {
        let ty = smithay_target(target);
        // The host's own selection is never given back to it, and a locked session gives nothing.
        if !self.selection_allowed() || !bridge::is_bridged(&mime_type) || self.selection_owners.host_owns(ty) {
            drop(fd);
            return;
        }
        let result: Result<(), String> = match ty {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|error| error.to_string())
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).map_err(|error| error.to_string())
            }
        };
        if let Err(error) = result {
            log::debug!(area = "selection", ?ty, %error, "the host asked for a selection nobody holds");
        }
    }

    /// The mime types of the current clipboard selection, for a diagnostics reader.
    pub(crate) fn clipboard_mime_types(&self) -> Option<Vec<String>> {
        self.selections.mime_types(SelectionTarget::Clipboard)
    }

    /// The mime types of the current primary selection, for a diagnostics reader.
    pub(crate) fn primary_mime_types(&self) -> Option<Vec<String>> {
        self.selections.mime_types(SelectionTarget::Primary)
    }

    /// Records a selection's mime types for the observation, and publishes the change.
    ///
    /// Both the Wayland side and the X11 bridge go through here, so the observation
    /// reflects whichever side actually set the selection.
    pub(crate) fn record_selection(&mut self, ty: SelectionTarget, mime_types: Option<Vec<String>>) {
        self.selections.set(ty, mime_types);
        self.publish();
    }

    /// Tells the clipboard which connection has focus.
    ///
    /// `wl_data_device` delivers a selection to the focused client and to nobody else,
    /// so this follows the keyboard: a client pastes only while it is the one the user
    /// is typing into.
    pub(crate) fn set_selection_focus(&mut self, client: Option<&smithay::reexports::wayland_server::Client>) {
        let display_handle: &DisplayHandle = &self.display_handle;
        smithay::wayland::selection::data_device::set_data_device_focus(display_handle, &self.seat, client.cloned());
        smithay::wayland::selection::primary_selection::set_primary_focus(display_handle, &self.seat, client.cloned());
    }
}
