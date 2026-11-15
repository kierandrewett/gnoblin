//! The parent's `zwp_tablet_v2` objects, handed to [`super::super::tablet::Tablets`].
//!
//! Every handler here is one call on the state machine and nothing else: the decisions (what a frame means, which
//! tool is over which tablet) live in `tablet.rs`, where they are tested without a socket. The only thing this file
//! does that the state machine cannot is name a surface's output, which needs the connection's records.
//!
//! The manager is bound at version 1: the dial a version 2 pad adds has no event in the contract, and a host that
//! offers 2 still speaks 1 to a client that asked for it.

use wayland_client::protocol::wl_seat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2, zwp_tablet_pad_group_v2, zwp_tablet_pad_ring_v2, zwp_tablet_pad_strip_v2, zwp_tablet_pad_v2,
    zwp_tablet_seat_v2, zwp_tablet_tool_v2, zwp_tablet_v2,
};

use crate::{ToolAxes, ToolKind};

use super::Handler;

/// The protocol number of an object, the key `Tablets` uses for it.
fn key(object: &impl Proxy) -> u32 {
    object.id().protocol_id()
}

impl Handler {
    /// Asks the parent for its tablet seat, once the parent's seat is known. Idempotent.
    pub(super) fn bind_tablets(&mut self, seat: &wl_seat::WlSeat, qh: &QueueHandle<Self>) {
        if self.tablet_seat.is_none() {
            self.tablet_seat = self
                .tablet_manager
                .as_ref()
                .map(|manager| manager.get_tablet_seat(seat, qh, ()));
        }
    }

    /// Runs `change` on the tablet state and queues the events it produced on the session's seat.
    fn tablet(&mut self, change: impl FnOnce(&mut crate::nested::tablet::Tablets)) {
        change(&mut self.tablets);
        let events = self.tablets.drain();
        self.table.seat().extend(events);
    }
}

wayland_client::delegate_noop!(Handler: ignore zwp_tablet_manager_v2::ZwpTabletManagerV2);

impl Dispatch<zwp_tablet_seat_v2::ZwpTabletSeatV2, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_seat_v2::ZwpTabletSeatV2,
        event: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_seat_v2::Event::TabletAdded { id } => state.tablet(|tablets| tablets.tablet_added(key(&id))),
            zwp_tablet_seat_v2::Event::ToolAdded { id } => state.tablet(|tablets| tablets.tool_added(key(&id))),
            zwp_tablet_seat_v2::Event::PadAdded { id } => state.tablet(|tablets| tablets.pad_added(key(&id))),
            _ => {}
        }
    }

    wayland_client::event_created_child!(Handler, zwp_tablet_seat_v2::ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (zwp_tablet_v2::ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (zwp_tablet_tool_v2::ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (zwp_tablet_pad_v2::ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_v2::ZwpTabletV2, ()> for Handler {
    fn event(
        state: &mut Self,
        tablet: &zwp_tablet_v2::ZwpTabletV2,
        event: zwp_tablet_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = key(tablet);
        match event {
            zwp_tablet_v2::Event::Name { name } => state.tablet(|tablets| tablets.tablet_name(id, &name)),
            zwp_tablet_v2::Event::Id { vid, pid } => state.tablet(|tablets| tablets.tablet_ids(id, vid, pid)),
            zwp_tablet_v2::Event::Done => state.tablet(|tablets| tablets.tablet_done(id)),
            zwp_tablet_v2::Event::Removed => {
                state.tablet(|tablets| tablets.tablet_removed(id));
                tablet.destroy();
            }
            _ => {}
        }
    }
}

/// The kind a wire tool type is, [`ToolKind::Pen`] for a type this build does not know.
fn kind_of(kind: &WEnum<zwp_tablet_tool_v2::Type>) -> ToolKind {
    match kind {
        WEnum::Value(zwp_tablet_tool_v2::Type::Eraser) => ToolKind::Eraser,
        WEnum::Value(zwp_tablet_tool_v2::Type::Brush) => ToolKind::Brush,
        WEnum::Value(zwp_tablet_tool_v2::Type::Pencil) => ToolKind::Pencil,
        WEnum::Value(zwp_tablet_tool_v2::Type::Airbrush) => ToolKind::Airbrush,
        WEnum::Value(zwp_tablet_tool_v2::Type::Finger) => ToolKind::Finger,
        WEnum::Value(zwp_tablet_tool_v2::Type::Mouse) => ToolKind::Mouse,
        WEnum::Value(zwp_tablet_tool_v2::Type::Lens) => ToolKind::Lens,
        _ => ToolKind::Pen,
    }
}

/// The axis a wire capability names, or none for one this build does not know.
fn axis_of(capability: &WEnum<zwp_tablet_tool_v2::Capability>) -> ToolAxes {
    match capability {
        WEnum::Value(zwp_tablet_tool_v2::Capability::Tilt) => ToolAxes::TILT,
        WEnum::Value(zwp_tablet_tool_v2::Capability::Pressure) => ToolAxes::PRESSURE,
        WEnum::Value(zwp_tablet_tool_v2::Capability::Distance) => ToolAxes::DISTANCE,
        WEnum::Value(zwp_tablet_tool_v2::Capability::Rotation) => ToolAxes::ROTATION,
        WEnum::Value(zwp_tablet_tool_v2::Capability::Slider) => ToolAxes::SLIDER,
        WEnum::Value(zwp_tablet_tool_v2::Capability::Wheel) => ToolAxes::WHEEL,
        _ => ToolAxes::NONE,
    }
}

impl Dispatch<zwp_tablet_tool_v2::ZwpTabletToolV2, ()> for Handler {
    fn event(
        state: &mut Self,
        tool: &zwp_tablet_tool_v2::ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_tool_v2::Event;
        let id = key(tool);
        match event {
            Event::Type { tool_type } => state.tablet(|tablets| tablets.tool_kind(id, kind_of(&tool_type))),
            Event::HardwareSerial {
                hardware_serial_hi,
                hardware_serial_lo,
            } => state.tablet(|tablets| tablets.tool_serial(id, hardware_serial_hi, hardware_serial_lo)),
            Event::HardwareIdWacom {
                hardware_id_hi,
                hardware_id_lo,
            } => state.tablet(|tablets| tablets.tool_wacom(id, hardware_id_hi, hardware_id_lo)),
            Event::Capability { capability } => {
                state.tablet(|tablets| tablets.tool_capability(id, axis_of(&capability)));
            }
            Event::Done => state.tablet(|tablets| tablets.tool_done(id)),
            Event::Removed => {
                state.tablet(|tablets| tablets.tool_removed(id));
                tool.destroy();
            }
            Event::ProximityIn { tablet, surface, .. } => {
                let output = state.output_of(&surface);
                state.tablet(|tablets| tablets.proximity_in(id, key(&tablet), output));
            }
            Event::ProximityOut => state.tablet(|tablets| tablets.proximity_out(id)),
            Event::Down { .. } => state.tablet(|tablets| tablets.tip(id, true)),
            Event::Up => state.tablet(|tablets| tablets.tip(id, false)),
            Event::Motion { x, y } => state.tablet(|tablets| tablets.motion(id, x, y)),
            Event::Pressure { pressure } => state.tablet(|tablets| tablets.pressure(id, pressure)),
            Event::Distance { distance } => state.tablet(|tablets| tablets.distance(id, distance)),
            Event::Tilt { tilt_x, tilt_y } => state.tablet(|tablets| tablets.tilt(id, tilt_x, tilt_y)),
            Event::Rotation { degrees } => state.tablet(|tablets| tablets.rotation(id, degrees)),
            Event::Slider { position } => state.tablet(|tablets| tablets.slider(id, position)),
            Event::Wheel { degrees, clicks } => state.tablet(|tablets| tablets.wheel(id, degrees, clicks)),
            Event::Button {
                button, state: pressed, ..
            } => {
                let pressed = matches!(pressed, WEnum::Value(zwp_tablet_tool_v2::ButtonState::Pressed));
                state.tablet(|tablets| tablets.tool_button(id, button, pressed));
            }
            Event::Frame { .. } => state.tablet(|tablets| tablets.frame(id)),
            _ => {}
        }
    }
}

impl Dispatch<zwp_tablet_pad_v2::ZwpTabletPadV2, ()> for Handler {
    fn event(
        state: &mut Self,
        pad: &zwp_tablet_pad_v2::ZwpTabletPadV2,
        event: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_pad_v2::Event;
        let id = key(pad);
        match event {
            Event::Group { pad_group } => state.tablet(|tablets| tablets.pad_group(id, key(&pad_group))),
            Event::Buttons { buttons } => state.tablet(|tablets| tablets.pad_buttons(id, buttons)),
            Event::Done => state.tablet(|tablets| tablets.pad_done(id)),
            Event::Button {
                button, state: pressed, ..
            } => {
                let pressed = matches!(pressed, WEnum::Value(zwp_tablet_pad_v2::ButtonState::Pressed));
                state.tablet(|tablets| tablets.pad_button(id, button, pressed));
            }
            Event::Removed => {
                state.tablet(|tablets| tablets.pad_removed(id));
                pad.destroy();
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Handler, zwp_tablet_pad_v2::ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()> for Handler {
    fn event(
        state: &mut Self,
        group: &zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
        event: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = key(group);
        match event {
            zwp_tablet_pad_group_v2::Event::Ring { ring } => state.tablet(|tablets| tablets.group_ring(id, key(&ring))),
            zwp_tablet_pad_group_v2::Event::Strip { strip } => {
                state.tablet(|tablets| tablets.group_strip(id, key(&strip)));
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Handler, zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()> for Handler {
    fn event(
        state: &mut Self,
        ring: &zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
        event: zwp_tablet_pad_ring_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = key(ring);
        match event {
            zwp_tablet_pad_ring_v2::Event::Angle { degrees } => state.tablet(|tablets| tablets.ring_angle(id, degrees)),
            zwp_tablet_pad_ring_v2::Event::Stop => state.tablet(|tablets| tablets.ring_stop(id)),
            zwp_tablet_pad_ring_v2::Event::Frame { .. } => state.tablet(|tablets| tablets.ring_frame(id)),
            _ => {}
        }
    }
}

impl Dispatch<zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()> for Handler {
    fn event(
        state: &mut Self,
        strip: &zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
        event: zwp_tablet_pad_strip_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = key(strip);
        match event {
            zwp_tablet_pad_strip_v2::Event::Position { position } => {
                state.tablet(|tablets| tablets.strip_position(id, position));
            }
            zwp_tablet_pad_strip_v2::Event::Stop => state.tablet(|tablets| tablets.strip_stop(id)),
            zwp_tablet_pad_strip_v2::Event::Frame { .. } => state.tablet(|tablets| tablets.strip_frame(id)),
            _ => {}
        }
    }
}
