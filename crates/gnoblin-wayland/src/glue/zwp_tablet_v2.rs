//! `gnoblin-wayland-zwp-tablet-v2`: the seat, the keyboard focus, the lock and the surface under a point are the state's.

use gnoblin_backend::{DeviceCaps, DeviceInfo};
use gnoblin_lock::State as LockState;
use gnoblin_types::DeviceId;
use gnoblin_wayland_protocols::zwp_tablet_v2::{self, Host, TabletDevice, TabletTables};
use smithay::input::tablet::{TabletSeat, TabletSeatTrait};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};

use crate::State;

impl Host for State {
    fn tablets(&mut self) -> &mut TabletTables<Self> {
        &mut self.tablets
    }

    fn tablet_seat(&self) -> TabletSeat<Self> {
        self.seat.tablet_seat()
    }

    fn surface_under(&self, location: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        State::surface_under(self, location)
    }

    fn keyboard_focus(&self) -> Option<WlSurface> {
        self.keyboard.current_focus()
    }

    fn tablets_locked(&self) -> bool {
        self.lock.state() != LockState::Unlocked
    }

    fn pen_activates(&mut self, location: Point<f64, Logical>) {
        self.activate_at(location);
    }

    fn remember_serial(&mut self, serial: u32) {
        self.activation.remember_serial(serial);
    }
}

impl State {
    /// A device joined the seat: a tablet if it has the tool capability, and a pad if it has the pad capability.
    pub(crate) fn tablet_device_added(&mut self, info: &DeviceInfo) {
        zwp_tablet_v2::device_added(
            self,
            &TabletDevice {
                id: info.id,
                name: &info.name,
                vendor: info.vendor,
                product: info.product,
                tool: info.caps.contains(DeviceCaps::TABLET_TOOL),
                pad: info.caps.contains(DeviceCaps::TABLET_PAD).then_some(info.pad),
            },
        );
    }

    /// A device left: its tools leave proximity first, then the tablet or pad goes.
    pub(crate) fn tablet_device_removed(&mut self, id: DeviceId) {
        zwp_tablet_v2::device_removed(self, id);
    }
}
