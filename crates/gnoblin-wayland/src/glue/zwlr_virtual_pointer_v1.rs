//! `gnoblin-wayland-zwlr-virtual-pointer-v1`: a virtual pointer's events take the seat's own input path.

use gnoblin_backend::{Host as _, InputEvent};
use gnoblin_types::{DeviceId, OutputId, Point, Rect};
use gnoblin_wayland_protocols::zwlr_virtual_pointer_v1::{self, Scroll, VirtualPointers};
use smithay::backend::input::InputEvent as DeviceEvent;
use smithay::wayland::virtual_pointer::{VirtualPointerBackend, VirtualPointerHandler};

use crate::State;

impl zwlr_virtual_pointer_v1::Host for State {
    fn deliver_motion_relative(&mut self, dx: i32, dy: i32) {
        self.input(InputEvent::PointerMotionRelative {
            device: DeviceId::VIRTUAL,
            dx,
            dy,
        });
    }

    fn deliver_motion_to(&mut self, output: OutputId, position: Point) {
        self.input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output,
            position,
        });
    }

    fn deliver_button(&mut self, button: u32, pressed: bool) {
        self.input(InputEvent::PointerButton {
            device: DeviceId::VIRTUAL,
            button,
            pressed,
        });
    }

    fn deliver_scroll(&mut self, scroll: Scroll) {
        self.input_scroll(scroll);
    }

    fn layout(&self) -> Vec<(OutputId, Rect)> {
        self.outputs.iter().map(|entry| (entry.backend, entry.rect)).collect()
    }

    fn virtual_pointers(&mut self) -> &mut VirtualPointers {
        &mut self.virtual_pointer
    }
}

impl VirtualPointerHandler for State {
    fn process_virtual_pointer_event(&mut self, event: DeviceEvent<VirtualPointerBackend>) {
        zwlr_virtual_pointer_v1::process(self, event);
    }
}
