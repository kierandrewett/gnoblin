//! The parent's `zwp_linux_dmabuf_v1`: what it imports, and the answers to the buffers this session asks it to create.
//!
//! The decisions (which pairs, which device, which buffer is free) are in [`super::super::host_dmabuf`] and
//! [`super::super::present_dmabuf`], both plain data. This file is the wire: the parent's feedback becomes
//! [`HostDmabuf`], and a buffer's `created` or `failed` becomes a [`DmabufNote`] the target reads on the next dispatch.
//!
//! A buffer the parent created has no user data of its own that names its slot (the slot is the params object's), so
//! the buffer's identity is remembered here and its `release` is looked up.

use std::collections::HashMap;
use std::io::Read;

use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_buffer;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

use super::{Handler, SlotKey};
use crate::nested::host_dmabuf::HostDmabuf;

/// The largest format table this backend reads: a table is sixteen bytes a pair, and no host has a million of them.
const MAX_TABLE_BYTES: u32 = 1 << 20;

/// What the parent answered about a buffer the session asked it to create.
#[derive(Debug)]
pub(crate) enum DmabufNote {
    /// The parent created the buffer.
    Created {
        /// The output's record index.
        index: usize,
        /// The generation of buffers it belongs to.
        generation: u32,
        /// Its slot.
        slot: u8,
        /// The `wl_buffer` to attach.
        buffer: wl_buffer::WlBuffer,
    },
    /// The parent refused a buffer of this generation.
    Failed {
        /// The output's record index.
        index: usize,
        /// The generation of buffers it belongs to.
        generation: u32,
    },
}

/// The parent's dmabuf global, what it said, and the buffers it created.
#[derive(Default)]
pub(crate) struct DmabufHost {
    /// The global, when the parent offers one.
    pub(super) global: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    /// What the parent imports.
    pub(super) offer: HostDmabuf,
    /// Which slot each buffer the parent created is.
    buffers: HashMap<ObjectId, SlotKey>,
    /// What the target has not read yet.
    notes: Vec<DmabufNote>,
}

impl DmabufHost {
    /// The parent's dmabuf global, which has said nothing yet.
    pub(crate) fn new(global: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>) -> Self {
        Self {
            global,
            ..Self::default()
        }
    }
}

impl Handler {
    /// Asks a parent of version 4 for its feedback. The object lives as long as the connection: it is the parent's way of
    /// changing its mind, and a change reaches the next buffer the session creates.
    pub(crate) fn ask_dmabuf_feedback(&mut self, qh: &QueueHandle<Self>) {
        if let Some(global) = self.dmabuf.global.as_ref()
            && global.version() >= 4
        {
            let _feedback = global.get_default_feedback(qh, ());
        }
    }

    /// Forgets the buffers of an output that is gone.
    pub(super) fn forget_dmabuf_buffers_of(&mut self, index: usize) {
        self.dmabuf.buffers.retain(|_, key| key.index != index);
    }

    /// The parent's dmabuf global.
    pub fn dmabuf_global(&self) -> Option<&zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1> {
        self.dmabuf.global.as_ref()
    }

    /// What the parent imports.
    pub(crate) fn host_dmabuf(&self) -> &HostDmabuf {
        &self.dmabuf.offer
    }

    /// Takes the parent's answers about buffers.
    pub(crate) fn take_dmabuf_notes(&mut self) -> Vec<DmabufNote> {
        std::mem::take(&mut self.dmabuf.notes)
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        event: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_dmabuf_v1::Event::Format { format } => state.dmabuf.offer.legacy_format(format),
            zwp_linux_dmabuf_v1::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } => state.dmabuf.offer.legacy_modifier(format, modifier_hi, modifier_lo),
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_linux_dmabuf_feedback_v1::Event;
        let offer = &mut state.dmabuf.offer;
        match event {
            Event::MainDevice { device } => offer.main_device(&device),
            Event::FormatTable { fd, size } => {
                let mut bytes = Vec::new();
                let limit = u64::from(size.min(MAX_TABLE_BYTES));
                if std::fs::File::from(fd).take(limit).read_to_end(&mut bytes).is_ok() {
                    offer.format_table(&bytes);
                }
            }
            Event::TrancheTargetDevice { device } => offer.tranche_target(&device),
            Event::TrancheFormats { indices } => offer.tranche_formats(&indices),
            Event::TrancheDone => offer.tranche_done(),
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, SlotKey> for Handler {
    fn event(
        state: &mut Self,
        params: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        key: &SlotKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_buffer_params_v1::Event::Created { buffer } => {
                // The buffers of an older generation of this output are gone or going: forgetting them here bounds the
                // table to two generations.
                state
                    .dmabuf
                    .buffers
                    .retain(|_, known| known.index != key.index || known.generation >= key.generation);
                state.dmabuf.buffers.insert(buffer.id(), *key);
                state.dmabuf.notes.push(DmabufNote::Created {
                    index: key.index,
                    generation: key.generation,
                    slot: key.slot,
                    buffer,
                });
            }
            zwp_linux_buffer_params_v1::Event::Failed => {
                state.dmabuf.notes.push(DmabufNote::Failed {
                    index: key.index,
                    generation: key.generation,
                });
            }
            _ => {}
        }
        // A params object is used once.
        params.destroy();
    }

    wayland_client::event_created_child!(Handler, zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, [
        zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<wl_buffer::WlBuffer, ()> for Handler {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event
            && let Some(key) = state.dmabuf.buffers.get(&buffer.id())
        {
            state.table.released(key.index, key.generation, key.slot);
        }
    }
}
