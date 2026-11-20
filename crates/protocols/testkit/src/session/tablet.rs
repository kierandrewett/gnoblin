//! The tablet client the tests of `zwp_tablet_v2` drive: it records every event a tablet, tool and pad object is told in
//! [`App::tablet_log`], one line each.

use std::thread;
use std::time::{Duration, Instant};

use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::tablet::zv2::client::{
    zwp_tablet_manager_v2, zwp_tablet_pad_group_v2, zwp_tablet_pad_ring_v2, zwp_tablet_pad_strip_v2, zwp_tablet_pad_v2,
    zwp_tablet_seat_v2, zwp_tablet_tool_v2, zwp_tablet_v2,
};

use super::{App, Client, TIMEOUT};

/// An enum's name from its `Debug` text, without the `Value(..)` a protocol enum is wrapped in.
pub fn plain(value: &impl std::fmt::Debug) -> String {
    let text = format!("{value:?}");
    text.strip_prefix("Value(")
        .and_then(|rest| rest.strip_suffix(')'))
        .map_or(text.clone(), str::to_owned)
}

// ---- the client ------------------------------------------------------------------------------------------------------

impl Dispatch<zwp_tablet_manager_v2::ZwpTabletManagerV2, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwp_tablet_manager_v2::ZwpTabletManagerV2,
        _: zwp_tablet_manager_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_tablet_seat_v2::ZwpTabletSeatV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_seat_v2::ZwpTabletSeatV2,
        event: zwp_tablet_seat_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.tablet_log.push(
            match event {
                zwp_tablet_seat_v2::Event::TabletAdded { .. } => "tablet_added",
                zwp_tablet_seat_v2::Event::ToolAdded { .. } => "tool_added",
                zwp_tablet_seat_v2::Event::PadAdded { .. } => "pad_added",
                _ => "seat_other",
            }
            .to_owned(),
        );
    }

    wayland_client::event_created_child!(App, zwp_tablet_seat_v2::ZwpTabletSeatV2, [
        zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (zwp_tablet_v2::ZwpTabletV2, ()),
        zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE => (zwp_tablet_tool_v2::ZwpTabletToolV2, ()),
        zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE => (zwp_tablet_pad_v2::ZwpTabletPadV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_v2::ZwpTabletV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_v2::ZwpTabletV2,
        event: zwp_tablet_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_v2::Event::Name { name } => state.tablet_log.push(format!("tablet_name {name}")),
            zwp_tablet_v2::Event::Id { vid, pid } => state.tablet_log.push(format!("tablet_id {vid:x} {pid:x}")),
            zwp_tablet_v2::Event::Removed => state.tablet_log.push("tablet_removed".to_owned()),
            _ => {}
        }
    }
}

impl Dispatch<zwp_tablet_tool_v2::ZwpTabletToolV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_tool_v2::ZwpTabletToolV2,
        event: zwp_tablet_tool_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_tool_v2::Event;
        let line = match event {
            Event::Type { tool_type } => format!("tool_type {}", plain(&tool_type)),
            Event::HardwareSerial {
                hardware_serial_hi,
                hardware_serial_lo,
            } => format!("tool_serial {hardware_serial_hi} {hardware_serial_lo}"),
            Event::Capability { capability } => format!("tool_capability {}", plain(&capability)),
            Event::ProximityIn { .. } => "proximity_in".to_owned(),
            Event::ProximityOut => "proximity_out".to_owned(),
            Event::Down { .. } => "down".to_owned(),
            Event::Up => "up".to_owned(),
            Event::Motion { x, y } => format!("motion {x:.2} {y:.2}"),
            Event::Pressure { pressure } => format!("pressure {pressure}"),
            Event::Distance { distance } => format!("distance {distance}"),
            Event::Tilt { tilt_x, tilt_y } => format!("tilt {tilt_x:.1} {tilt_y:.1}"),
            Event::Button { button, state, .. } => format!("button {button} {}", plain(&state)),
            Event::Frame { .. } => "frame".to_owned(),
            Event::Removed => "tool_removed".to_owned(),
            _ => return,
        };
        state.tablet_log.push(line);
    }
}

impl Dispatch<zwp_tablet_pad_v2::ZwpTabletPadV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_pad_v2::ZwpTabletPadV2,
        event: zwp_tablet_pad_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_tablet_pad_v2::Event;
        let line = match event {
            Event::Buttons { buttons } => format!("pad_buttons {buttons}"),
            Event::Done => "pad_done".to_owned(),
            Event::Enter { .. } => "pad_enter".to_owned(),
            Event::Leave { .. } => "pad_leave".to_owned(),
            Event::Button { button, state, .. } => format!("pad_button {button} {}", plain(&state)),
            Event::Removed => "pad_removed".to_owned(),
            _ => return,
        };
        state.tablet_log.push(line);
    }

    wayland_client::event_created_child!(App, zwp_tablet_pad_v2::ZwpTabletPadV2, [
        zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
        event: zwp_tablet_pad_group_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_pad_group_v2::Event::Ring { .. } => state.tablet_log.push("pad_has_ring".to_owned()),
            zwp_tablet_pad_group_v2::Event::Strip { .. } => state.tablet_log.push("pad_has_strip".to_owned()),
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, [
        zwp_tablet_pad_group_v2::EVT_RING_OPCODE => (zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()),
        zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()),
    ]);
}

impl Dispatch<zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2,
        event: zwp_tablet_pad_ring_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_pad_ring_v2::Event::Angle { degrees } => state.tablet_log.push(format!("pad_ring {degrees:.1}")),
            zwp_tablet_pad_ring_v2::Event::Stop => state.tablet_log.push("pad_ring_stop".to_owned()),
            _ => {}
        }
    }
}

impl Dispatch<zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2,
        event: zwp_tablet_pad_strip_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_tablet_pad_strip_v2::Event::Position { position } => {
                state.tablet_log.push(format!("pad_strip {position}"));
            }
            zwp_tablet_pad_strip_v2::Event::Stop => state.tablet_log.push("pad_strip_stop".to_owned()),
            _ => {}
        }
    }
}

impl Client {
    /// Binds the tablet manager and asks for the seat's tablet seat.
    pub fn bind_tablets(&mut self) -> zwp_tablet_seat_v2::ZwpTabletSeatV2 {
        let manager: zwp_tablet_manager_v2::ZwpTabletManagerV2 = self
            .globals
            .bind(&self.qh, 1..=1, ())
            .expect("the session serves zwp_tablet_manager_v2");
        let seat = manager.get_tablet_seat(&self.seat, &self.qh, ());
        self.roundtrip();
        seat
    }

    /// Waits until the tablet log satisfies `predicate`.
    pub fn wait_log(&mut self, what: &str, predicate: impl Fn(&[String]) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.roundtrip();
            if predicate(&self.state.tablet_log) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: the client's tablet log is {:#?}",
                self.state.tablet_log
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// How many lines of the log start with `prefix`.
    pub fn logged(&self, prefix: &str) -> usize {
        self.state
            .tablet_log
            .iter()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    /// The numbers after `prefix` on the first line that starts with it.
    pub fn numbers_of(&self, prefix: &str, skip: usize) -> Option<Vec<f64>> {
        self.state
            .tablet_log
            .iter()
            .filter(|line| line.starts_with(prefix))
            .nth(skip)
            .map(|line| {
                line[prefix.len()..]
                    .split_whitespace()
                    .filter_map(|word| word.parse().ok())
                    .collect()
            })
    }
}
