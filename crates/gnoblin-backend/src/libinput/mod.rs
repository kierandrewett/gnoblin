//! The seat's input devices: libinput over the session, mapped to the contract's events.
//!
//! This is the mechanism half of the Input card (design/rust-compositor.md, "Input"). The
//! decisions are next door in [`map`] (what a device event means) and [`settings`] (what the
//! configuration asks a device to be), both pure and unit-tested. This module is what is left:
//! a libinput context opened through the seat's session, a device table, and the translation
//! from Smithay's device-agnostic events into [`DeviceEvent`]s.
//!
//! ```text
//! libinput event -> DeviceEvent (map::Mapper) -> InputEvent -> the compositor's seat
//!                -> RawEvent (gestures, tablets) -> gnoblin-input's router
//! ```
//!
//! # Which device a configuration section is for
//!
//! libinput reports capabilities, not kinds, so [`kind_of`] is the rule that turns them into
//! the configuration's kinds: a touchpad reports tap finger counts, a touch screen has the
//! touch capability, a keyboard has the keyboard capability and no pointer capability, and
//! everything else is a mouse. It is one function so the rule has one place to change.
//!
//! # Failure modes first
//!
//! 1. **A device removed with a button held leaves it held.** [`map::Mapper::cancel`] runs on
//!    removal and on a pause, so the release is delivered:
//!    `map::tests::a_removed_device_releases_every_held_button`.
//! 2. **A resumed session delivers the events of the pause.** The context is suspended on a
//!    pause and the queue is dropped, because a delta accumulated while another VT was on the
//!    display is a pointer jump nobody asked for.
//! 3. **A refused setting stops the rest of the configuration.** It is counted and skipped
//!    ([`Applier`]): `settings::tests::a_refused_setting_does_not_stop_the_rest`.
//! 4. **An event is translated with the wrong scale.** libinput states absolute and touch
//!    positions as fractions of the device's own range, so they pass through as they are and
//!    the mapper places them on an output: `map::tests::an_absolute_point_lands_where_it_was_reported`.
//! 5. **A gesture reaches the seat as a pointer click.** Gestures are [`RawEvent`]s and never
//!    [`InputEvent`]s, so the router sees them and no client does.

pub mod map;
pub mod settings;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, Event, GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputEvent as BackendEvent,
    KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TouchEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::Session as _;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::input as libinput;
use tracing::{debug, info, warn};

use crate::drm::{RawEvent, RawGestureKind};
use crate::{InputEvent, InputSource, TouchPhase};

pub use map::{DeviceEvent, Mapper, Screen};
pub use settings::{Accel, Applier, ButtonMap, Click, DeviceKind, DeviceSettings, Scroll, Setting};

/// One event libinput produced, as Smithay's device-agnostic wrapper.
type Queued = BackendEvent<LibinputInputBackend>;

/// Why the input devices could not be opened.
#[derive(Debug)]
pub enum LibinputError {
    /// libinput refused the seat, which a session whose seat udev does not know reports.
    Seat(String),
    /// The seat's session would not lend the input devices.
    Devices(String),
}

impl std::fmt::Display for LibinputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seat(reason) => write!(formatter, "libinput cannot assign the seat: {reason}"),
            Self::Devices(reason) => write!(formatter, "libinput cannot open the devices: {reason}"),
        }
    }
}

impl std::error::Error for LibinputError {}

/// One device the seat knows about.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Device {
    /// The backend's own number, which a [`RawEvent`] names.
    pub id: u64,
    /// The kernel's name for the device, such as `"event7"`.
    pub sysname: String,
    /// The name the device reports, such as `"SynPS/2 Synaptics TouchPad"`.
    pub name: String,
    /// The kind of device, which selects the configuration section.
    pub kind: DeviceKind,
}

/// The seat's input source: libinput, its devices and the queue the contract drains.
pub struct Libinput {
    /// The events the loop source has produced and the backend has not translated yet.
    queue: Rc<RefCell<VecDeque<Queued>>>,
    /// The translation and the state it remembers.
    mapper: Mapper,
    /// The contract's events, ready for [`InputSource::next_event`].
    ready: VecDeque<InputEvent>,
    /// The events for `gnoblin-input`'s router.
    raw: VecDeque<RawEvent>,
    /// The devices the seat has, in the order they arrived.
    devices: Vec<Device>,
    /// The number the next device is given.
    next: u64,
    /// Whether the seat is suspended, in which case nothing is delivered.
    suspended: bool,
    /// The libinput adapter, which [`Libinput::source`] hands to the loop once.
    context: Option<LibinputInputBackend>,
    /// Where each touch slot last was. libinput states no position on a release, so a release
    /// is placed where its slot was last seen.
    touch_positions: BTreeMap<u32, (f64, f64)>,
}

impl std::fmt::Debug for Libinput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Libinput")
            .field("devices", &self.devices)
            .field("ready", &self.ready.len())
            .field("suspended", &self.suspended)
            .finish_non_exhaustive()
    }
}

impl Libinput {
    /// Opens libinput on the seat's session.
    ///
    /// The session is what lends the device nodes; libinput's own udev assignment needs the
    /// seat's name and nothing else. The context is created here and wrapped in Smithay's
    /// adapter, which is the only part of this module that knows libinput's event types.
    ///
    /// # Errors
    ///
    /// [`LibinputError`] when the seat cannot be assigned, which is a session whose seat udev
    /// does not describe.
    pub fn open(session: &LibSeatSession) -> Result<Self, LibinputError> {
        let seat = session.seat();
        let interface = LibinputSessionInterface::from(session.clone());
        let mut context = libinput::Libinput::new_with_udev(interface);
        context.udev_assign_seat(&seat).map_err(|error| {
            warn!(seat = %seat, ?error, "[input] libinput refused the seat");
            LibinputError::Seat(format!("{error:?}"))
        })?;
        // The adapter is what turns libinput's own events into Smithay's; it is created here
        // and its events are drained through the loop source rather than by this module, so
        // the reading happens on the compositor's own thread and in its own iteration.
        let adapter = LibinputInputBackend::new(context);
        info!(seat = %seat, "[input] libinput is ready");
        Ok(Self {
            queue: Rc::new(RefCell::new(VecDeque::new())),
            mapper: Mapper::new(),
            ready: VecDeque::new(),
            raw: VecDeque::new(),
            devices: Vec::new(),
            next: 1,
            suspended: false,
            context: Some(adapter),
            touch_positions: BTreeMap::new(),
        })
    }

    /// Registers libinput's event source with the compositor's loop.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when the loop refuses the source, which leaves a session whose input
    /// never arrives: the caller reports it and stops.
    pub fn register<S: 'static>(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        let queue = Rc::clone(&self.queue);
        loop_handle
            .insert_source(self.source()?, move |event, _, _data| {
                let mut queue = queue.borrow_mut();
                queue.push_back(event);
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// The libinput source, ready to be inserted into a loop by the caller.
    ///
    /// It is separate from [`Libinput::register`] so a session that owns its own loop can
    /// insert the source with its own callback.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when libinput refuses to hand over its descriptor, which happens only when
    /// the context was closed.
    pub fn source(&mut self) -> io::Result<LibinputInputBackend> {
        self.context
            .take()
            .ok_or_else(|| io::Error::other("the libinput context is already registered"))
    }

    /// The devices the seat has.
    #[must_use]
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Tells the mapper which outputs absolute devices report on.
    ///
    /// A hotplug calls this with the new table, so an absolute device whose output went away is
    /// relocated instead of writing to an output that is no longer scanned out.
    pub fn set_screens(&mut self, screens: Vec<Screen>) {
        self.mapper.set_screens(screens);
    }

    /// The next event for `gnoblin-input`'s router, which the contract cannot carry.
    pub fn next_raw(&mut self) -> Option<RawEvent> {
        self.raw.pop_front()
    }

    /// Stops delivering events until [`Libinput::resume`].
    ///
    /// A pause drops what is queued: a delta a device collected while another VT was on the
    /// display would move the pointer by an amount nobody asked for.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.queue.borrow_mut().clear();
        self.ready.clear();
        self.raw.clear();
        for event in self.mapper.cancel() {
            self.ready.push_back(event);
        }
    }

    /// Delivers events again.
    pub fn resume(&mut self) {
        self.suspended = false;
    }

    /// Translates everything the source queued.
    ///
    /// It is called from the backend's own dispatch, so the devices are read in the
    /// compositor's iteration and nowhere else.
    pub fn pump(&mut self) {
        if self.suspended {
            self.queue.borrow_mut().clear();
            return;
        }
        loop {
            let Some(event) = self.queue.borrow_mut().pop_front() else {
                return;
            };
            self.translate(event);
        }
    }

    /// Queues one gesture event for the router, naming the device that produced it.
    ///
    /// A device the seat has not announced has no number to name, so its gesture is dropped
    /// rather than attributed to another device.
    fn gesture(&mut self, device: &libinput::Device, kind: RawGestureKind, begin: bool, end: bool, value: f64) {
        let sysname = device.sysname().to_string();
        let Some(known) = self.devices.iter().find(|known| known.sysname == sysname) else {
            warn!(sysname = %sysname, "[input] a gesture from a device that was never announced");
            return;
        };
        self.raw.push_back(RawEvent::Gesture {
            device: known.id,
            kind,
            begin,
            end,
            value,
        });
    }

    /// One queued event, as the contract's event and the router's.
    fn translate(&mut self, event: Queued) {
        let mut produced: Vec<InputEvent> = Vec::new();
        match event {
            BackendEvent::DeviceAdded { device } => {
                let kind = kind_of(&device);
                let record = Device {
                    id: self.next,
                    sysname: device.sysname().to_owned(),
                    name: device.name().to_string(),
                    kind,
                };
                debug!(name = %record.name, ?kind, "[input] a device arrived");
                produced.push(InputEvent::DeviceAdded { id: record.id, kind });
                self.next = self.next.saturating_add(1);
                self.devices.push(record);
            }
            BackendEvent::DeviceRemoved { device } => {
                let sysname = device.sysname().to_owned();
                let id = self
                    .devices
                    .iter()
                    .find(|known| known.sysname == sysname)
                    .map(|known| known.id);
                self.devices.retain(|known| known.sysname != sysname);
                info!(sysname = %sysname, "[input] a device went away");
                produced.extend(self.mapper.cancel());
                if let Some(id) = id {
                    produced.push(InputEvent::DeviceRemoved { id });
                }
            }
            BackendEvent::Keyboard { event } => produced.extend(self.mapper.event(DeviceEvent::Key {
                code: event.key_code().raw().saturating_sub(8),
                pressed: event.state() == smithay::backend::input::KeyState::Pressed,
            })),
            BackendEvent::PointerMotion { event } => {
                let delta = event.delta();
                produced.extend(self.mapper.event(DeviceEvent::Motion {
                    dx: delta.x,
                    dy: delta.y,
                }));
            }
            BackendEvent::PointerMotionAbsolute { event } => {
                produced.extend(self.mapper.event(DeviceEvent::Absolute {
                    x: event.x(),
                    y: event.y(),
                }));
            }
            BackendEvent::PointerButton { event } => produced.extend(self.mapper.event(DeviceEvent::Button {
                code: event.button_code(),
                pressed: event.state() == smithay::backend::input::ButtonState::Pressed,
            })),
            BackendEvent::PointerAxis { event } => {
                let (horizontal, vertical) = axis(&event);
                produced.extend(self.mapper.event(DeviceEvent::Scroll { horizontal, vertical }));
            }
            BackendEvent::TouchDown { event } => {
                let (slot, x, y) = (slot_of(&event), event.x(), event.y());
                self.touch_positions.insert(slot, (x, y));
                produced.extend(self.mapper.event(DeviceEvent::Touch {
                    slot,
                    x,
                    y,
                    phase: TouchPhase::Down,
                }));
            }
            BackendEvent::TouchMotion { event } => {
                let (slot, x, y) = (slot_of(&event), event.x(), event.y());
                self.touch_positions.insert(slot, (x, y));
                produced.extend(self.mapper.event(DeviceEvent::Touch {
                    slot,
                    x,
                    y,
                    phase: TouchPhase::Motion,
                }));
            }
            // A release and a cancel carry no position, so they are placed where their slot was
            // last seen. A slot with no position was never delivered, so it has nothing to end.
            BackendEvent::TouchUp { event } => {
                let slot = slot_of(&event);
                if let Some((x, y)) = self.touch_positions.remove(&slot) {
                    produced.extend(self.mapper.event(DeviceEvent::Touch {
                        slot,
                        x,
                        y,
                        phase: TouchPhase::Up,
                    }));
                }
            }
            BackendEvent::TouchCancel { event } => {
                let slot = slot_of(&event);
                if let Some((x, y)) = self.touch_positions.remove(&slot) {
                    produced.extend(self.mapper.event(DeviceEvent::Touch {
                        slot,
                        x,
                        y,
                        phase: TouchPhase::Cancel,
                    }));
                }
            }
            BackendEvent::GestureSwipeBegin { event } => {
                self.gesture(&event.device(), RawGestureKind::Swipe, true, false, 0.0);
            }
            BackendEvent::GestureSwipeUpdate { event } => {
                let delta = event.delta();
                self.gesture(&event.device(), RawGestureKind::Swipe, false, false, -delta.y);
            }
            BackendEvent::GestureSwipeEnd { event } => {
                self.gesture(&event.device(), RawGestureKind::Swipe, false, true, 0.0);
            }
            BackendEvent::GesturePinchBegin { event } => {
                self.gesture(&event.device(), RawGestureKind::Pinch, true, false, 0.0);
            }
            BackendEvent::GesturePinchUpdate { event } => {
                self.gesture(&event.device(), RawGestureKind::Pinch, false, false, event.scale());
            }
            BackendEvent::GesturePinchEnd { event } => {
                self.gesture(&event.device(), RawGestureKind::Pinch, false, true, 0.0);
            }
            BackendEvent::GestureHoldBegin { event } => {
                self.gesture(&event.device(), RawGestureKind::Hold, true, false, 0.0);
            }
            BackendEvent::GestureHoldEnd { event } => {
                self.gesture(&event.device(), RawGestureKind::Hold, false, true, 0.0);
            }
            // A touch frame is the boundary between two atomic updates, which the seat applies
            // in order anyway; a tablet tool is read by `gnoblin-input` from the raw channel,
            // which this increment does not route yet; a switch is a lid or a tablet mode the
            // session's own policy reads from the device list.
            BackendEvent::TouchFrame { .. }
            | BackendEvent::TabletToolAxis { .. }
            | BackendEvent::TabletToolProximity { .. }
            | BackendEvent::TabletToolTip { .. }
            | BackendEvent::TabletToolButton { .. }
            | BackendEvent::SwitchToggle { .. } => {}
            other => {
                debug!(?other, "[input] an event this seat does not use");
            }
        }
        self.ready.extend(produced);
    }
}

impl InputSource for Libinput {
    fn next_event(&mut self) -> Option<InputEvent> {
        self.ready.pop_front()
    }
}

/// The touch slot a touch event belongs to, as the key the mapper tracks its touches by.
///
/// A device with a single touch point states no slot, which Smithay reports as -1; that point
/// is slot 0.
fn slot_of<E: TouchEvent<LibinputInputBackend>>(event: &E) -> u32 {
    u32::try_from(i32::from(event.slot())).unwrap_or(0)
}

/// The two scroll axes of one event, in fractions of a step.
///
/// A wheel states its amount in 1/120 of a step and a touchpad states it in pixels; both are
/// what [`DeviceEvent::Scroll`] carries as a fraction of a step, so the mapper's accumulator
/// has one unit to work in.
fn axis<E: PointerAxisEvent<LibinputInputBackend>>(event: &E) -> (f64, f64) {
    let amount = |axis: Axis| match event.amount_v120(axis) {
        Some(steps) => steps / 120.0,
        None => event.amount(axis).unwrap_or(0.0) / 120.0,
    };
    (amount(Axis::Horizontal), amount(Axis::Vertical))
}

/// The configuration kind of a device, from the capabilities the driver reports.
///
/// libinput names capabilities rather than kinds, and the configuration's sections are kinds.
/// The rule, in order: a device that reports a tap finger count is a touchpad, a device with a
/// touch capability is a touch screen, a device with a keyboard capability and no pointer
/// capability is a keyboard, and anything else is a mouse.
#[must_use]
pub fn kind_of(device: &libinput::Device) -> DeviceKind {
    if device.config_tap_finger_count() > 0 {
        DeviceKind::Touchpad
    } else if device.has_capability(libinput::DeviceCapability::Touch) {
        DeviceKind::Touch
    } else if device.has_capability(libinput::DeviceCapability::Keyboard)
        && !device.has_capability(libinput::DeviceCapability::Pointer)
    {
        DeviceKind::Keyboard
    } else {
        DeviceKind::Mouse
    }
}
