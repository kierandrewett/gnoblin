//! The clipboard: `wl_data_device_manager` (XML `wayland.xml`) and the selection state every
//! selection protocol shares. The primary selection, `ext_data_control_v1` and
//! `wlr_data_control_unstable_v1` are in their own modules.
//!
//! ## What the compositor's part is
//!
//! Smithay owns the protocol objects, the offers and the seat's record of which
//! selection is current. Two things stay here:
//!
//! - the *source* of the current selection, because a `receive` request from another
//!   client is answered by handing the destination a pipe and asking the source's
//!   client to write into it — the compositor never copies the bytes itself;
//! - the two privileged globals, which a clipboard manager uses to read and write the
//!   selection without being the focused application. Those are decided per client
//!   ([`crate::access`]): a sandboxed or unprivileged application does not see them at
//!   all, so a manager cannot be smuggled into a sandbox.
//!
//! ## Focus is the gate for the ordinary clipboard
//!
//! `wl_data_device` only delivers a selection to the client that has keyboard focus,
//! so the keyboard focus and the data-device focus are kept in step here: a client
//! that is focused may paste, and one that is not has no offer to paste from. That is
//! the protocol's rule, not an extra one.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/selection.rs` unless it says otherwise.
//!
//! 1. **A client pastes a selection whose owner has gone.** The offer is cleared when
//!    the source's client disconnects, so the destination is told the selection is
//!    empty instead of holding an offer that can only be answered with nothing —
//!    `a_selection_does_not_outlive_its_owner`.
//! 2. **A data-control manager is visible to an application.** Both data-control
//!    globals are filtered: an unprivileged connection does not see them, a privileged
//!    one does (`a_data_control_manager_needs_a_privileged_client`), and a sandboxed
//!    one does not either (`a_sandboxed_client_sees_no_privileged_global` in
//!    `tests/access.rs`).
//! 3. **A paste is served from a copy the compositor kept.** The bytes come from the
//!    source client through the pipe, so a source that writes nothing yields nothing:
//!    the compositor holds a mime-type list and a handle, never the data —
//!    `a_copy_between_two_clients_carries_the_source_bytes`.
//! 4. **A privileged paste request after the manager's own protocol is off.** Both
//!    data-control globals answer to the single `ext_data_control` configuration name,
//!    so turning the family off hides both: `turning_data_control_off_hides_both_managers`.

use std::os::unix::io::OwnedFd;

use smithay::input::Seat;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use tracing::{debug, trace};

use crate::State;

/// The selection targets the session holds a source for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionKind {
    /// The clipboard: what `Ctrl+V` pastes.
    Clipboard,
    /// The primary selection: what a middle click pastes.
    Primary,
}

/// The session's selections.
///
/// Only their mime types are recorded, and only for the observation: the bytes never
/// pass through the compositor. Smithay forwards a `receive` on a client-set offer
/// straight to the source's own client, so nothing here is in the data path — which is
/// also why a source whose owner has gone cannot serve stale bytes: the offer itself is
/// withdrawn when the source resource is destroyed.
#[derive(Default)]
pub(crate) struct Selections {
    clipboard: Option<Vec<String>>,
    primary: Option<Vec<String>>,
}

impl Selections {
    /// The mime types of a target, for the observation.
    fn mime_types(&self, target: SelectionKind) -> Option<Vec<String>> {
        match target {
            SelectionKind::Clipboard => self.clipboard.clone(),
            SelectionKind::Primary => self.primary.clone(),
        }
    }

    /// Replaces a target's selection.
    fn set(&mut self, target: SelectionKind, mime_types: Option<Vec<String>>) {
        let slot = match target {
            SelectionKind::Clipboard => &mut self.clipboard,
            SelectionKind::Primary => &mut self.primary,
        };
        *slot = mime_types;
    }
}

impl SelectionHandler for State {
    /// Nothing outside the selection itself needs to be carried along with it: the
    /// source is stored here and the bytes never leave the two clients.
    type SelectionUserData = ();

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, seat: Seat<Self>) {
        let _ = &seat;
        let target = match ty {
            SelectionTarget::Clipboard => SelectionKind::Clipboard,
            SelectionTarget::Primary => SelectionKind::Primary,
        };
        // Captured now, while the source's client is certainly there: asking later
        // would be asking a resource that may be gone.
        let mime_types = source.as_ref().map(SelectionSource::mime_types);
        match &mime_types {
            Some(mime_types) => debug!(?target, ?mime_types, "[selection] set"),
            None => debug!(?target, "[selection] cleared"),
        }
        self.selections.set(target, mime_types);
        self.publish();
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        let _ = user_data;
        let _ = &seat;
        let target = match ty {
            SelectionTarget::Clipboard => SelectionKind::Clipboard,
            SelectionTarget::Primary => SelectionKind::Primary,
        };
        // A client-set selection never reaches here: Smithay forwards that `receive` to
        // the source's own client, which is what keeps the bytes out of the compositor.
        // This path is a compositor-set selection, and this slice sets none, so the
        // honest answer is end of file — dropping the write end is exactly that.
        trace!(?target, mime_type, "[selection] no compositor selection to send");
        drop(fd);
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

impl State {
    /// The mime types of the current clipboard selection, for a diagnostics reader.
    pub(crate) fn clipboard_mime_types(&self) -> Option<Vec<String>> {
        self.selections.mime_types(SelectionKind::Clipboard)
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
