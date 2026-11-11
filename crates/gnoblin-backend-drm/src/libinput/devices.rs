//! The devices the seat has, and the numbers they were announced under.
//!
//! libinput names a device by its kernel name (`event7`), which the kernel reuses: unplug a
//! keyboard, plug another in, and both are `event7`. The contract's [`DeviceId`] must not be
//! reused, because a press or a gesture that names a removed device would resolve to the next one.
//! [`Devices`] is the table that turns one into the other. It is pure: the glue reads a
//! libinput device into a [`DeviceInfo`] draft and this module numbers it, so the numbering
//! rules are tested on a machine with no input device.
//!
//! # Failure modes first
//!
//! 1. **A removed device's number is given to the next device.** The counter only moves up:
//!    `a_number_is_never_reused`.
//! 2. **An event from a device nobody announced is attributed to another device.** It resolves to
//!    nothing, and the caller falls back to [`DeviceId::VIRTUAL`]: `an_unknown_kernel_name_has_no_number`.
//! 3. **A device announced twice is two devices.** The second announcement is refused, so the
//!    seat never holds one device under two numbers: `a_device_is_announced_once`.
//! 4. **A removal of a device nobody announced removes another.** It removes nothing:
//!    `removing_an_unknown_device_removes_nothing`.

use gnoblin_backend::DeviceInfo;
use gnoblin_types::DeviceId;

/// One device the seat has: the kernel's name for it and what the backend announced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Device {
    /// The kernel's name for the device, such as `"event7"`.
    pub sysname: String,
    /// What the backend announced, with the number every event of the device carries.
    pub info: DeviceInfo,
}

/// The seat's devices, in the order they arrived.
#[derive(Clone, Debug)]
pub struct Devices {
    known: Vec<Device>,
    /// The number the next device is given. It only moves up.
    next: u64,
}

impl Default for Devices {
    fn default() -> Self {
        Self {
            known: Vec::new(),
            next: 1,
        }
    }
}

impl Devices {
    /// A table with no device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Numbers a device that arrived and records it. The `draft`'s own `id` is ignored.
    ///
    /// `None` when the kernel name is already known: libinput announces a device once, so a
    /// second announcement is a duplicate and gets no second number.
    pub fn announce(&mut self, sysname: &str, draft: DeviceInfo) -> Option<DeviceInfo> {
        if self.id_of(sysname).is_some() {
            return None;
        }
        let info = DeviceInfo {
            id: DeviceId::new(self.next),
            ..draft
        };
        self.next = self.next.saturating_add(1);
        self.known.push(Device {
            sysname: sysname.to_owned(),
            info: info.clone(),
        });
        Some(info)
    }

    /// The number a kernel name was announced under.
    #[must_use]
    pub fn id_of(&self, sysname: &str) -> Option<DeviceId> {
        self.known
            .iter()
            .find(|device| device.sysname == sysname)
            .map(|device| device.info.id)
    }

    /// The number a kernel name was announced under, or [`DeviceId::VIRTUAL`] for a device nobody
    /// announced.
    #[must_use]
    pub fn id_or_virtual(&self, sysname: &str) -> DeviceId {
        self.id_of(sysname).unwrap_or(DeviceId::VIRTUAL)
    }

    /// Forgets a device and returns the number it had.
    pub fn remove(&mut self, sysname: &str) -> Option<DeviceId> {
        let index = self.known.iter().position(|device| device.sysname == sysname)?;
        Some(self.known.remove(index).info.id)
    }

    /// The devices, in the order they arrived.
    #[must_use]
    pub fn all(&self) -> &[Device] {
        &self.known
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_backend::DeviceKind;

    fn draft(name: &str, kind: DeviceKind) -> DeviceInfo {
        DeviceInfo::new(DeviceId::VIRTUAL, name, kind)
    }

    #[test]
    fn a_number_is_never_reused() {
        let mut devices = Devices::new();
        let first = devices
            .announce("event7", draft("Keyboard A", DeviceKind::Keyboard))
            .expect("announced");
        assert_eq!(first.id, DeviceId::new(1));
        assert_eq!(devices.remove("event7"), Some(DeviceId::new(1)));
        // The kernel gives the next keyboard the same name; it must not get the same number.
        let second = devices
            .announce("event7", draft("Keyboard B", DeviceKind::Keyboard))
            .expect("announced");
        assert_eq!(second.id, DeviceId::new(2));
        assert_eq!(second.name, "Keyboard B");
    }

    #[test]
    fn an_unknown_kernel_name_has_no_number() {
        let mut devices = Devices::new();
        devices.announce("event3", draft("Mouse", DeviceKind::Mouse));
        assert_eq!(devices.id_of("event9"), None);
        assert_eq!(devices.id_or_virtual("event9"), DeviceId::VIRTUAL);
        assert_eq!(devices.id_or_virtual("event3"), DeviceId::new(1));
    }

    #[test]
    fn a_device_is_announced_once() {
        let mut devices = Devices::new();
        assert!(devices.announce("event3", draft("Mouse", DeviceKind::Mouse)).is_some());
        assert!(devices.announce("event3", draft("Mouse", DeviceKind::Mouse)).is_none());
        assert_eq!(devices.all().len(), 1);
    }

    #[test]
    fn removing_an_unknown_device_removes_nothing() {
        let mut devices = Devices::new();
        devices.announce("event3", draft("Mouse", DeviceKind::Mouse));
        assert_eq!(devices.remove("event4"), None);
        assert_eq!(devices.all().len(), 1);
    }

    #[test]
    fn the_announcement_keeps_what_the_driver_reported() {
        let mut devices = Devices::new();
        let mut macro_pad = draft("Macro Pad", DeviceKind::Keyboard);
        macro_pad.vendor = 0x1234;
        macro_pad.product = 0x5678;
        macro_pad.seat = "seat1".to_owned();
        let info = devices.announce("event5", macro_pad).expect("announced");
        assert_eq!(
            (info.vendor, info.product, info.seat.as_str()),
            (0x1234, 0x5678, "seat1")
        );
        assert_eq!(devices.all()[0].info, info);
    }
}
