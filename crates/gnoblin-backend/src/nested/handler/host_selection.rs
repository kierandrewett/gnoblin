//! The host's clipboard and primary selection, as the nested session sees them.
//!
//! The rules (what crosses, when, how much) are in [`crate::selection`]; this is the wire: the parent's data device
//! and primary selection device, the offers it sends, and the sources this session offers back.
//!
//! The parent sends its selection to the client that has the keyboard, and only to it, so the session sees the host's
//! clipboard exactly while its window is focused. Everything here is bookkeeping around that:
//!
//! - **An offer is described before it is the selection.** `data_offer` creates the object and `offer` events name
//!   its types; only the `selection` event after them says it is the selection. Offers that never become one (a
//!   drag's) are destroyed when the drag leaves.
//! - **Our own copy comes back.** When this session sets the host's selection the host tells its focused client about
//!   the new selection, which is the session itself. The echo is recognised by its types and ignored once, so a copy
//!   in the session does not become a host copy that is offered back to the session.
//! - **Focus is the gate both ways.** A selection set by the session while the window is not focused is not offered to
//!   the host, and leaving the window withdraws the host's selection from the session.

use std::collections::HashMap;
use std::os::fd::{AsFd, OwnedFd};

use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1, zwp_primary_selection_offer_v1,
    zwp_primary_selection_source_v1,
};

use crate::selection::{self, SelectionRefusal, SelectionTarget};

use super::Handler;

/// What the connection learnt that the session's [`Host`](crate::Host) has to hear, in the order it arrived.
#[derive(Debug)]
pub(crate) enum Note {
    /// The host's selection changed: the types it offers now, or `None` when it holds none.
    HostChanged {
        target: SelectionTarget,
        mime_types: Option<Vec<String>>,
    },
    /// The host pastes the session's selection as `mime_type` into `fd`.
    HostWants {
        target: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    },
}

/// The host's offer of one selection, kept so a client of the session can paste from it.
enum HostOffer {
    Data(wl_data_offer::WlDataOffer),
    Primary(zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1),
}

impl HostOffer {
    fn receive(&self, mime: &str, fd: &impl AsFd) {
        match self {
            Self::Data(offer) => offer.receive(mime.to_owned(), fd.as_fd()),
            Self::Primary(offer) => offer.receive(mime.to_owned(), fd.as_fd()),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Data(offer) => offer.destroy(),
            Self::Primary(offer) => offer.destroy(),
        }
    }
}

/// The source this session offered the host for one selection.
enum SessionSource {
    Data(wl_data_source::WlDataSource),
    Primary(zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1),
}

impl SessionSource {
    fn id(&self) -> ObjectId {
        match self {
            Self::Data(source) => source.id(),
            Self::Primary(source) => source.id(),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Data(source) => source.destroy(),
            Self::Primary(source) => source.destroy(),
        }
    }
}

/// One selection's state on this connection.
#[derive(Default)]
struct Channel {
    /// What the host offers the window now, with the types it named.
    host: Option<(HostOffer, Vec<String>)>,
    /// What the session offered the host, with the types it named.
    session: Option<(SessionSource, Vec<String>)>,
    /// Whether the host's next selection event is the echo of the session's own set.
    echo: bool,
}

impl Channel {
    fn drop_host(&mut self) -> bool {
        match self.host.take() {
            Some((offer, _)) => {
                offer.destroy();
                true
            }
            None => false,
        }
    }
}

/// The parent's selection devices and what the session and the host currently offer each other.
#[derive(Default)]
pub(crate) struct Bridge {
    data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    primary_manager: Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    primary_device: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    /// Offers that were announced and are not the selection (yet): their types so far.
    offers: HashMap<ObjectId, Vec<String>>,
    /// The offer a drag entered with, destroyed when it leaves.
    drag: Option<wl_data_offer::WlDataOffer>,
    clipboard: Channel,
    primary: Channel,
    /// Whether the session's window has the parent's keyboard.
    focused: bool,
    /// The newest serial of an input event the parent sent, which `set_selection` must name.
    serial: u32,
    /// What the host has to be told, in order.
    notes: Vec<Note>,
}

impl Bridge {
    /// A bridge over the managers the parent offers; either may be absent.
    pub(crate) fn new(
        data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
        primary_manager: Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    ) -> Self {
        Self {
            data_manager,
            primary_manager,
            ..Self::default()
        }
    }

    /// Creates the devices once the parent's seat is known. Idempotent.
    pub(crate) fn bind_devices(&mut self, seat: &wl_seat::WlSeat, qh: &QueueHandle<Handler>) {
        if self.data_device.is_none() {
            self.data_device = self
                .data_manager
                .as_ref()
                .map(|manager| manager.get_data_device(seat, qh, ()));
        }
        if self.primary_device.is_none() {
            self.primary_device = self
                .primary_manager
                .as_ref()
                .map(|manager| manager.get_device(seat, qh, ()));
        }
    }

    /// The newest serial an input event of the parent carried.
    pub(crate) fn note_serial(&mut self, serial: u32) {
        self.serial = serial;
    }

    /// The window gained the parent's keyboard: the parent is about to send its selection.
    pub(crate) fn focus_in(&mut self, serial: u32) {
        self.serial = serial;
        self.focused = true;
        // An echo is only owed while the focus it was set under lasts.
        self.clipboard.echo = false;
        self.primary.echo = false;
    }

    /// The window lost the parent's keyboard: the host's selections are withdrawn from the session.
    pub(crate) fn focus_out(&mut self) {
        self.focused = false;
        for (target, channel) in [
            (SelectionTarget::Clipboard, &mut self.clipboard),
            (SelectionTarget::Primary, &mut self.primary),
        ] {
            channel.echo = false;
            if channel.drop_host() {
                self.notes.push(Note::HostChanged {
                    target,
                    mime_types: None,
                });
            }
        }
    }

    /// Takes what the host has to be told.
    pub(crate) fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }

    fn channel(&mut self, target: SelectionTarget) -> &mut Channel {
        match target {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        }
    }

    /// The parent says `offer` (or nothing) is now its selection.
    fn host_selected(&mut self, target: SelectionTarget, offer: Option<HostOffer>, id: Option<ObjectId>) {
        // The host sends its selection to the client that has its keyboard focus and to no other, so receiving one is
        // proof of the focus even from a host with no `wl_keyboard` to say so (a headless Sway has none). Losing the
        // focus is still `focus_out`, which a host with a keyboard sends.
        if offer.is_some() {
            self.focused = true;
        }
        let mime_types = id.and_then(|id| self.offers.remove(&id)).unwrap_or_default();
        let channel = self.channel(target);
        let echo = std::mem::take(&mut channel.echo);
        let ours = channel
            .session
            .as_ref()
            .is_some_and(|(_, offered)| *offered == mime_types);
        if echo && ours && offer.is_some() {
            // The session's own copy, coming back. Nothing changed for the session.
            if let Some(offer) = offer {
                offer.destroy();
            }
            return;
        }
        let had = channel.drop_host();
        let Some(offer) = offer else {
            if had {
                self.notes.push(Note::HostChanged {
                    target,
                    mime_types: None,
                });
            }
            return;
        };
        let bridged = selection::bridged_mimes(&mime_types);
        let channel = self.channel(target);
        if bridged.is_empty() {
            // Nothing that crosses (or a secret): the offer is not kept and the session sees an empty selection.
            offer.destroy();
            if had {
                self.notes.push(Note::HostChanged {
                    target,
                    mime_types: None,
                });
            }
            return;
        }
        channel.host = Some((offer, mime_types));
        self.notes.push(Note::HostChanged {
            target,
            mime_types: Some(bridged),
        });
    }

    /// A client of the session reads the host's selection as `mime`, into `fd`.
    ///
    /// # Errors
    ///
    /// [`SelectionRefusal::NoOffer`] when the host offers nothing to a window that has the keyboard, and
    /// [`SelectionRefusal::MimeNotBridged`] for a type that does not cross or that the host did not offer.
    pub(crate) fn read(&mut self, target: SelectionTarget, mime: &str, fd: OwnedFd) -> Result<(), SelectionRefusal> {
        if !self.focused {
            return Err(SelectionRefusal::NoOffer);
        }
        let channel = self.channel(target);
        let Some((offer, offered)) = channel.host.as_ref() else {
            return Err(SelectionRefusal::NoOffer);
        };
        if !selection::is_bridged(mime) || !offered.iter().any(|name| name == mime) {
            return Err(SelectionRefusal::MimeNotBridged);
        }
        let Ok((read, write)) = rustix::pipe::pipe() else {
            return Err(SelectionRefusal::NoOffer);
        };
        offer.receive(mime, &write);
        // The parent holds its own copy of the write end now; ours must close, or the reader never sees the end.
        drop(write);
        drop(selection::transfer(read, fd));
        Ok(())
    }

    /// A client of the session set (`Some`) or cleared (`None`) a selection: offer it to the host when the window has
    /// the keyboard.
    pub(crate) fn offer(
        &mut self,
        target: SelectionTarget,
        mime_types: Option<Vec<String>>,
        qh: &QueueHandle<Handler>,
    ) {
        if !self.focused {
            return;
        }
        let serial = self.serial;
        let wanted = mime_types.filter(|types| !types.is_empty());
        let (data_manager, data_device) = (self.data_manager.clone(), self.data_device.clone());
        let (primary_manager, primary_device) = (self.primary_manager.clone(), self.primary_device.clone());
        let channel = self.channel(target);
        match wanted {
            Some(types) => {
                let source = match target {
                    SelectionTarget::Clipboard => {
                        let (Some(manager), Some(device)) = (data_manager, data_device) else {
                            return;
                        };
                        let source = manager.create_data_source(qh, ());
                        for mime in &types {
                            source.offer(mime.clone());
                        }
                        device.set_selection(Some(&source), serial);
                        SessionSource::Data(source)
                    }
                    SelectionTarget::Primary => {
                        let (Some(manager), Some(device)) = (primary_manager, primary_device) else {
                            return;
                        };
                        let source = manager.create_source(qh, ());
                        for mime in &types {
                            source.offer(mime.clone());
                        }
                        device.set_selection(Some(&source), serial);
                        SessionSource::Primary(source)
                    }
                };
                if let Some((old, _)) = channel.session.replace((source, types)) {
                    old.destroy();
                }
                channel.echo = true;
            }
            None => {
                let Some((old, _)) = channel.session.take() else {
                    return;
                };
                match (target, &data_device, &primary_device) {
                    (SelectionTarget::Clipboard, Some(device), _) => device.set_selection(None, serial),
                    (SelectionTarget::Primary, _, Some(device)) => device.set_selection(None, serial),
                    _ => {}
                }
                old.destroy();
                channel.echo = false;
            }
        }
    }

    /// The host asks for the session's selection: the note the host is told when the connection is dispatched.
    fn host_wants(&mut self, target: SelectionTarget, mime_type: String, fd: OwnedFd) {
        self.notes.push(Note::HostWants { target, mime_type, fd });
    }

    /// The host replaced a source this session offered.
    fn source_cancelled(&mut self, id: &ObjectId) {
        for channel in [&mut self.clipboard, &mut self.primary] {
            if channel.session.as_ref().is_some_and(|(source, _)| source.id() == *id) {
                if let Some((source, _)) = channel.session.take() {
                    source.destroy();
                }
                channel.echo = false;
            }
        }
    }
}

wayland_client::delegate_noop!(Handler: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Handler: ignore zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1);

impl Dispatch<wl_data_device::WlDataDevice, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let bridge = &mut state.selection;
        match event {
            wl_data_device::Event::DataOffer { id } => {
                bridge.offers.insert(id.id(), Vec::new());
            }
            wl_data_device::Event::Selection { id } => {
                let key = id.as_ref().map(Proxy::id);
                bridge.host_selected(SelectionTarget::Clipboard, id.map(HostOffer::Data), key);
            }
            // A drag is not bridged. Its offer is released when it leaves.
            wl_data_device::Event::Enter { id: Some(offer), .. } => {
                bridge.offers.remove(&offer.id());
                if let Some(old) = bridge.drag.replace(offer) {
                    old.destroy();
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = bridge.drag.take() {
                    offer.destroy();
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Handler, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for Handler {
    fn event(
        state: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            // An offer the session already kept as the selection does not change its types.
            if let Some(types) = state.selection.offers.get_mut(&offer.id()) {
                types.push(mime_type);
            }
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Handler {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                state.selection.host_wants(SelectionTarget::Clipboard, mime_type, fd);
            }
            wl_data_source::Event::Cancelled => state.selection.source_cancelled(&source.id()),
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let bridge = &mut state.selection;
        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                bridge.offers.insert(offer.id(), Vec::new());
            }
            zwp_primary_selection_device_v1::Event::Selection { id } => {
                let key = id.as_ref().map(Proxy::id);
                bridge.host_selected(SelectionTarget::Primary, id.map(HostOffer::Primary), key);
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Handler, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()),
    ]);
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()> for Handler {
    fn event(
        state: &mut Self,
        offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && let Some(types) = state.selection.offers.get_mut(&offer.id())
        {
            types.push(mime_type);
        }
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()> for Handler {
    fn event(
        state: &mut Self,
        source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                state.selection.host_wants(SelectionTarget::Primary, mime_type, fd);
            }
            zwp_primary_selection_source_v1::Event::Cancelled => state.selection.source_cancelled(&source.id()),
            _ => {}
        }
    }
}
