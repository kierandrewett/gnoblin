//! The seat's input devices: libinput over the session, mapped to the contract's events.
//!
//! This is the mechanism half of input. The
//! decisions are next door in [`map`] (what a device event means) and in
//! `gnoblin_backend::input_settings` (what the configuration asks a device to be), both pure and
//! unit-tested, and [`configure`] makes a device so. This module is what is left: a libinput context
//! opened through the seat's session, a device table, and the translation from Smithay's
//! device-agnostic events into [`DeviceEvent`]s.
//!
//! ```text
//! libinput event -> DeviceEvent (map::Mapper) -> InputEvent -> the compositor's seat
//!                -> InputEvent::Touchpad (gestures) -> the policy's gesture router
//! ```
//!
//! # Which device a configuration section is for
//!
//! libinput reports capabilities, not kinds, so [`kind_of`] is the rule that turns them into
//! the configuration's kinds: a touchpad reports tap finger counts, a touch screen has the
//! touch capability, a keyboard has the keyboard capability and no pointer capability, a
//! trackball and a pointing stick carry udev's tags for them, and everything else is a mouse.
//! The rule is one pure function, [`classify`], so it has one place to change.
//!
//! # Failure modes first
//!
//! 1. **A device removed with a button held leaves it held.** [`map::Mapper::cancel`] runs on
//!    removal and on a pause, so the release is delivered:
//!    `map::tests::a_removed_device_releases_every_held_button`.
//! 2. **A resumed session delivers the events of the pause.** The context is suspended on a
//!    pause and the queue is dropped, because a delta accumulated while another VT was on the
//!    display is a pointer jump nobody asked for.
//! 3. **A refused setting stops the rest of the configuration.** It is recorded and skipped
//!    (`Applier`): `input_settings::tests::a_refused_setting_does_not_stop_the_rest`.
//! 4. **An event is translated with the wrong scale.** libinput's plain absolute and touch coordinates are
//!    millimetres, or the raw axis units of a device with no size, so the glue asks for the position transformed
//!    onto a fixed range and divides it back to a fraction of the device's own range ([`fraction`]); the mapper
//!    places that on an output. Passed through as they came, a virtio tablet's every point was clamped into the
//!    output's bottom-right corner and no client was ever under the pointer: the `tablet-position` and
//!    `tablet-pointer` checks of `scripts/ci/vm/input-check.sh`.
//! 5. **A gesture reaches the seat as a pointer click.** Gestures are [`InputEvent::Touchpad`]s
//!    and never pointer events, so the router sees them and no client does.

/// The configuration calls libinput takes, and the translation of a setting into them.
pub mod configure;
/// The devices the seat has, and the numbers they were announced under.
pub mod devices;
/// The translation rules: libinput device events become seat events, as pure logic over plain values.
pub mod map;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent,
    GestureSwipeUpdateEvent, InputEvent as BackendEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent, ProximityState, TabletToolButtonEvent, TabletToolCapabilities, TabletToolEvent,
    TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState, TabletToolType, TouchEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::Session as _;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::input as libinput;

use gnoblin_backend::input_settings::DeviceBook;
use gnoblin_backend::{
    DeviceCaps, DeviceInfo, DeviceKind, InputEvent, InputSource, PadLayout, RawTouchpadKind, RawTouchpadPhase,
    RawTouchpadSample, TabletAxes, TabletTool, ToolAxes, ToolKind, TouchPhase, single_line,
};
use gnoblin_config::Input;
use gnoblin_types::DeviceId;

pub use configure::Configurator;
pub use devices::{Device, Devices};
pub use map::{DeviceEvent, Mapper, Screen, TabletDeviceEvent};

use crate::log;

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

/// The seat's input source: libinput, its devices and the queue the contract drains.
pub struct Libinput {
    /// The events the loop source has produced and the backend has not translated yet.
    queue: Rc<RefCell<VecDeque<Queued>>>,
    /// The translation and the state it remembers.
    mapper: Mapper,
    /// The contract's events, ready for [`InputSource::next_event`].
    ready: VecDeque<InputEvent>,
    /// The devices the seat has, and the numbers they were announced under.
    devices: Devices,
    /// The document's `input` section and the libinput devices it was applied to, so a device that arrives is
    /// configured from it and a reload reaches every device the seat has.
    book: DeviceBook<Configurator<libinput::Device>>,
    /// Whether the seat is suspended, in which case nothing is delivered.
    suspended: bool,
    /// The libinput adapter, which [`Libinput::source`] hands to the loop once.
    context: Option<LibinputInputBackend>,
    /// Where each touch slot of each device last was. libinput states no position on a release, so a
    /// release is placed where its slot was last seen.
    touch_positions: BTreeMap<(DeviceId, u32), (f64, f64)>,
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
            log::warn!(area = "input", seat = %seat, ?error, "libinput refused the seat");
            LibinputError::Seat(format!("{error:?}"))
        })?;
        // The adapter is what turns libinput's own events into Smithay's; it is created here
        // and its events are drained through the loop source rather than by this module, so
        // the reading happens on the compositor's own thread and in its own iteration.
        let adapter = LibinputInputBackend::new(context);
        log::info!(area = "input", seat = %seat, "libinput is ready");
        Ok(Self {
            queue: Rc::new(RefCell::new(VecDeque::new())),
            mapper: Mapper::new(),
            ready: VecDeque::new(),
            devices: Devices::new(),
            book: DeviceBook::new(),
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

    /// The devices the seat has, in the order they arrived.
    #[must_use]
    pub fn devices(&self) -> &[Device] {
        self.devices.all()
    }

    /// Takes the document's `input` section: it is applied to every device the seat has now, and to each one
    /// that arrives later. A setting that left the document is put back to the device's own default.
    ///
    /// A device that refuses a setting is reported in the log once, and the rest of its settings still apply.
    pub fn set_input_settings(&mut self, input: &Input) {
        for refused in self.book.set_input(input.clone()) {
            refused.report();
        }
    }

    /// Tells the mapper which outputs absolute devices report on.
    ///
    /// A hotplug calls this with the new table, so an absolute device whose output went away is
    /// relocated instead of writing to an output that is no longer scanned out.
    pub fn set_screens(&mut self, screens: Vec<Screen>) {
        self.mapper.set_screens(screens);
    }

    /// Stops delivering events until [`Libinput::resume`].
    ///
    /// A pause drops what is queued: a delta a device collected while another VT was on the
    /// display would move the pointer by an amount nobody asked for.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.queue.borrow_mut().clear();
        self.ready.clear();
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

    /// Queues one touchpad gesture sample, naming the device that produced it.
    ///
    /// A device the seat has not announced has no number to name, so its sample is dropped
    /// rather than attributed to another device.
    fn gesture(
        &mut self,
        device: &libinput::Device,
        kind: RawTouchpadKind,
        phase: RawTouchpadPhase,
        step: GestureStep,
    ) {
        let Some(id) = self.devices.id_of(device.sysname()) else {
            log::warn!(area = "input", sysname = %device.sysname(), "a gesture from a device that was never announced");
            return;
        };
        self.ready.push_back(InputEvent::Touchpad(RawTouchpadSample {
            device: id,
            phase,
            kind,
            fingers: step.fingers,
            dx: step.delta.0,
            dy: step.delta.1,
            scale: step.scale,
            angle: step.angle,
        }));
    }

    /// The number a libinput device was announced under, or [`DeviceId::VIRTUAL`] for a device nobody
    /// announced. libinput announces every device before its first event, so the fallback is a
    /// defence and not a path.
    fn device_id(&self, device: &libinput::Device) -> DeviceId {
        self.devices.id_or_virtual(device.sysname())
    }

    /// One queued event, as the contract's event and the router's.
    fn translate(&mut self, event: Queued) {
        let mut produced: Vec<InputEvent> = Vec::new();
        match event {
            BackendEvent::DeviceAdded { device } => {
                let draft = describe(&device);
                if let Some(info) = self.devices.announce(device.sysname(), draft) {
                    log::debug!(area = "input", name = %info.name, id = %info.id, kind = ?info.kind, caps = %info.caps, "a device arrived");
                    // The device is configured the moment the seat knows it, before its first event is read.
                    for refused in self.book.add(&info, Configurator::new(device)) {
                        refused.report();
                    }
                    produced.push(InputEvent::DeviceAdded { device: info });
                }
            }
            BackendEvent::DeviceRemoved { device } => {
                let sysname = device.sysname().to_owned();
                log::info!(area = "input", sysname = %sysname, "a device went away");
                if let Some(id) = self.devices.remove(&sysname) {
                    let _ = self.book.remove(id);
                    // Only this device's held buttons and touch points end with it.
                    produced.extend(self.mapper.cancel_device(id));
                    self.touch_positions.retain(|(held, _), _| *held != id);
                    produced.push(InputEvent::DeviceRemoved { id });
                }
            }
            BackendEvent::Keyboard { event } => {
                let device = self.device_id(&event.device());
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Key {
                        code: event.key_code().raw().saturating_sub(8),
                        pressed: event.state() == smithay::backend::input::KeyState::Pressed,
                    },
                ));
            }
            BackendEvent::PointerMotion { event } => {
                let delta = event.delta();
                let device = self.device_id(&event.device());
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Motion {
                        dx: delta.x,
                        dy: delta.y,
                    },
                ));
            }
            BackendEvent::PointerMotionAbsolute { event } => {
                let device = self.device_id(&event.device());
                let (x, y) = fraction(&event);
                produced.extend(self.mapper.event(device, DeviceEvent::Absolute { x, y }));
            }
            BackendEvent::PointerButton { event } => {
                let device = self.device_id(&event.device());
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Button {
                        code: event.button_code(),
                        pressed: event.state() == smithay::backend::input::ButtonState::Pressed,
                    },
                ));
            }
            BackendEvent::PointerAxis { event } => {
                let (horizontal, vertical) = axis(&event);
                let device = self.device_id(&event.device());
                // `touchpad.scroll-speed` is the seat's multiplier: the device reports its own distance.
                let speed = self.book.device(device).map_or(1.0, Configurator::scroll_speed);
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Scroll {
                        horizontal: horizontal * speed,
                        vertical: vertical * speed,
                    },
                ));
            }
            BackendEvent::TouchDown { event } => {
                let device = self.device_id(&event.device());
                let (x, y) = fraction(&event);
                let slot = slot_of(&event);
                self.touch_positions.insert((device, slot), (x, y));
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Touch {
                        slot,
                        x,
                        y,
                        phase: TouchPhase::Down,
                    },
                ));
            }
            BackendEvent::TouchMotion { event } => {
                let device = self.device_id(&event.device());
                let (x, y) = fraction(&event);
                let slot = slot_of(&event);
                self.touch_positions.insert((device, slot), (x, y));
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Touch {
                        slot,
                        x,
                        y,
                        phase: TouchPhase::Motion,
                    },
                ));
            }
            // A release and a cancel carry no position, so they are placed where their slot was
            // last seen. A slot with no position was never delivered, so it has nothing to end.
            BackendEvent::TouchUp { event } => {
                let device = self.device_id(&event.device());
                let slot = slot_of(&event);
                if let Some((x, y)) = self.touch_positions.remove(&(device, slot)) {
                    produced.extend(self.mapper.event(
                        device,
                        DeviceEvent::Touch {
                            slot,
                            x,
                            y,
                            phase: TouchPhase::Up,
                        },
                    ));
                }
            }
            BackendEvent::TouchCancel { event } => {
                let device = self.device_id(&event.device());
                let slot = slot_of(&event);
                if let Some((x, y)) = self.touch_positions.remove(&(device, slot)) {
                    produced.extend(self.mapper.event(
                        device,
                        DeviceEvent::Touch {
                            slot,
                            x,
                            y,
                            phase: TouchPhase::Cancel,
                        },
                    ));
                }
            }
            BackendEvent::GestureSwipeBegin { event } => {
                let fingers = fingers_of(event.fingers());
                self.gesture(
                    &event.device(),
                    RawTouchpadKind::Swipe,
                    RawTouchpadPhase::Begin,
                    GestureStep::begin(fingers),
                );
            }
            BackendEvent::GestureSwipeUpdate { event } => {
                let delta = event.delta();
                self.gesture(
                    &event.device(),
                    RawTouchpadKind::Swipe,
                    RawTouchpadPhase::Update,
                    GestureStep {
                        delta: (delta.x, delta.y),
                        ..GestureStep::REST
                    },
                );
            }
            BackendEvent::GestureSwipeEnd { event } => {
                let phase = end_phase(event.cancelled());
                self.gesture(&event.device(), RawTouchpadKind::Swipe, phase, GestureStep::REST);
            }
            BackendEvent::GesturePinchBegin { event } => {
                let fingers = fingers_of(event.fingers());
                self.gesture(
                    &event.device(),
                    RawTouchpadKind::Pinch,
                    RawTouchpadPhase::Begin,
                    GestureStep::begin(fingers),
                );
            }
            BackendEvent::GesturePinchUpdate { event } => {
                self.gesture(
                    &event.device(),
                    RawTouchpadKind::Pinch,
                    RawTouchpadPhase::Update,
                    GestureStep {
                        delta: (event.delta_x(), event.delta_y()),
                        scale: event.scale(),
                        angle: event.angle_delta().to_radians(),
                        ..GestureStep::REST
                    },
                );
            }
            BackendEvent::GesturePinchEnd { event } => {
                let phase = end_phase(event.cancelled());
                self.gesture(&event.device(), RawTouchpadKind::Pinch, phase, GestureStep::REST);
            }
            BackendEvent::GestureHoldBegin { event } => {
                let fingers = fingers_of(event.fingers());
                self.gesture(
                    &event.device(),
                    RawTouchpadKind::Hold,
                    RawTouchpadPhase::Begin,
                    GestureStep::begin(fingers),
                );
            }
            BackendEvent::GestureHoldEnd { event } => {
                let phase = end_phase(event.cancelled());
                self.gesture(&event.device(), RawTouchpadKind::Hold, phase, GestureStep::REST);
            }
            // A tablet tool's changes are the mapper's: it places the position on an output and remembers which tools
            // hover, so a removal ends them. The pad has no event here at all: the toolkit's libinput source drops
            // a pad's buttons, rings and strips before they reach this queue (see `describe`).
            BackendEvent::TabletToolAxis { event } => {
                let device = self.device_id(&event.device());
                let (x, y) = tablet_position(&event);
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Tablet(TabletDeviceEvent::ToolAxis {
                        tool: tool_of(&event),
                        x,
                        y,
                        axes: axes_of(&event),
                    }),
                ));
            }
            BackendEvent::TabletToolProximity { event } => {
                let device = self.device_id(&event.device());
                let (x, y) = tablet_position(&event);
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Tablet(TabletDeviceEvent::ToolProximity {
                        tool: tool_of(&event),
                        inside: event.state() == ProximityState::In,
                        x,
                        y,
                        axes: axes_of(&event),
                    }),
                ));
            }
            BackendEvent::TabletToolTip { event } => {
                let device = self.device_id(&event.device());
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Tablet(TabletDeviceEvent::ToolTip {
                        tool: tool_of(&event),
                        down: <_ as TabletToolTipEvent<LibinputInputBackend>>::tip_state(&event)
                            == TabletToolTipState::Down,
                    }),
                ));
            }
            BackendEvent::TabletToolButton { event } => {
                let device = self.device_id(&event.device());
                produced.extend(self.mapper.event(
                    device,
                    DeviceEvent::Tablet(TabletDeviceEvent::ToolButton {
                        tool: tool_of(&event),
                        code: <_ as TabletToolButtonEvent<LibinputInputBackend>>::button(&event),
                        pressed: <_ as TabletToolButtonEvent<LibinputInputBackend>>::button_state(&event)
                            == ButtonState::Pressed,
                    }),
                ));
            }
            // A touch frame is the boundary between two atomic updates, which the seat applies in order anyway; a
            // switch is a lid or a tablet mode the session's own policy reads from the device list.
            BackendEvent::TouchFrame { .. } | BackendEvent::SwitchToggle { .. } => {}
            other => {
                log::debug!(area = "input", ?other, "an event this seat does not use");
            }
        }
        for event in &produced {
            log::debug!(area = "input", ?event, "a device event reaches the seat");
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

/// The range a position is asked for, so the fraction keeps far more than a pixel's worth of precision.
const POSITION_RANGE: i32 = 1 << 16;

/// An absolute point as a fraction of the device's own range on each axis, from 0 to 1.
///
/// `x()` and `y()` are not that: libinput gives them in millimetres from the device's top left, and for a device that
/// states no size, such as QEMU's virtio tablet, in its raw axis units (0 to 32767). Placed on an output as fractions,
/// every point of such a device lands past the far edge and is clamped into the output's corner.
fn fraction<E: AbsolutePositionEvent<LibinputInputBackend>>(event: &E) -> (f64, f64) {
    let range = f64::from(POSITION_RANGE);
    (
        event.x_transformed(POSITION_RANGE) / range,
        event.y_transformed(POSITION_RANGE) / range,
    )
}

/// The tablet tool an event is about, named by what it is.
fn tool_of<E: TabletToolEvent<LibinputInputBackend>>(event: &E) -> TabletTool {
    let descriptor = event.tool();
    let kind = match descriptor.tool_type {
        TabletToolType::Eraser => ToolKind::Eraser,
        TabletToolType::Brush => ToolKind::Brush,
        TabletToolType::Pencil => ToolKind::Pencil,
        TabletToolType::Airbrush => ToolKind::Airbrush,
        TabletToolType::Mouse => ToolKind::Mouse,
        TabletToolType::Lens => ToolKind::Lens,
        // A totem is a rotary device the contract has no kind for; it is carried as a pen, whose axes it names.
        TabletToolType::Pen | TabletToolType::Totem | TabletToolType::Unknown => ToolKind::Pen,
    };
    let mut axes = ToolAxes::NONE;
    for (capability, axis) in [
        (TabletToolCapabilities::TILT, ToolAxes::TILT),
        (TabletToolCapabilities::PRESSURE, ToolAxes::PRESSURE),
        (TabletToolCapabilities::DISTANCE, ToolAxes::DISTANCE),
        (TabletToolCapabilities::ROTATION, ToolAxes::ROTATION),
        (TabletToolCapabilities::SLIDER, ToolAxes::SLIDER),
        (TabletToolCapabilities::WHEEL, ToolAxes::WHEEL),
    ] {
        if descriptor.capabilities.contains(capability) {
            axes = axes.union(axis);
        }
    }
    TabletTool {
        kind,
        serial: descriptor.hardware_serial,
        hardware_id: descriptor.hardware_id_wacom,
        axes,
    }
}

/// The axes an event changed. libinput says which ones did, so a steady pen sends none.
fn axes_of<E: TabletToolEvent<LibinputInputBackend>>(event: &E) -> TabletAxes {
    TabletAxes {
        pressure: event.pressure_has_changed().then(|| event.pressure()),
        distance: event.distance_has_changed().then(|| event.distance()),
        tilt: (event.tilt_x_has_changed() || event.tilt_y_has_changed()).then(|| (event.tilt_x(), event.tilt_y())),
        rotation: event.rotation_has_changed().then(|| event.rotation()),
        slider: event.slider_has_changed().then(|| event.slider_position()),
        wheel: event
            .wheel_has_changed()
            .then(|| (event.wheel_delta(), event.wheel_delta_discrete())),
    }
}

/// A tool's position as a fraction of the tablet's range on each axis.
fn tablet_position<E: TabletToolEvent<LibinputInputBackend>>(event: &E) -> (f64, f64) {
    let range = f64::from(POSITION_RANGE);
    (
        event.x_transformed(POSITION_RANGE) / range,
        event.y_transformed(POSITION_RANGE) / range,
    )
}

/// What the driver and udev say about a device, as plain facts, for [`classify`].
#[derive(Clone, Copy, Debug, Default)]
struct Facts {
    /// The finger count libinput reports for tapping; zero for a device that cannot tap.
    tap_fingers: u32,
    /// The device has the touch capability.
    touch: bool,
    /// The device has the keyboard capability.
    keyboard: bool,
    /// The device has the pointer capability.
    pointer: bool,
    /// udev tags the device `ID_INPUT_POINTINGSTICK`.
    pointing_stick: bool,
    /// udev tags the device `ID_INPUT_TRACKBALL`.
    trackball: bool,
}

/// The configuration kind a device's facts make it.
///
/// The rule, in order: a device that reports a tap finger count is a touchpad, a device with a touch
/// capability is a touch screen, a device with a keyboard capability and no pointer capability is a
/// keyboard, a device udev tags as a pointing stick or as a trackball is that, and anything else is a
/// mouse. libinput has no capability for the last two, so without the udev tags a trackball would read the
/// mouse section and `input.trackball` and `input.pointing-stick` would apply to nothing.
fn classify(facts: Facts) -> DeviceKind {
    if facts.tap_fingers > 0 {
        DeviceKind::Touchpad
    } else if facts.touch {
        DeviceKind::Touch
    } else if facts.keyboard && !facts.pointer {
        DeviceKind::Keyboard
    } else if facts.pointing_stick {
        DeviceKind::PointingStick
    } else if facts.trackball {
        DeviceKind::Trackball
    } else {
        DeviceKind::Mouse
    }
}

/// Whether udev tags the input device `sysname` with `property` set to 1.
///
/// The tags come from systemd's hwdb (`ID_INPUT_TRACKBALL`, `ID_INPUT_POINTINGSTICK`), and libinput reads the same
/// ones. A device udev does not know, or a machine without the hwdb, is untagged, which is a mouse.
fn tagged(sysname: &str, property: &str) -> bool {
    smithay::reexports::udev::Device::from_subsystem_sysname("input".to_owned(), sysname.to_owned())
        .ok()
        .and_then(|device| device.property_value(property).map(|value| value == "1"))
        .unwrap_or(false)
}

/// The configuration kind of a device, from the capabilities the driver reports and the tags udev gives it.
#[must_use]
pub fn kind_of(device: &libinput::Device) -> DeviceKind {
    classify(Facts {
        tap_fingers: device.config_tap_finger_count(),
        touch: device.has_capability(libinput::DeviceCapability::Touch),
        keyboard: device.has_capability(libinput::DeviceCapability::Keyboard),
        pointer: device.has_capability(libinput::DeviceCapability::Pointer),
        pointing_stick: tagged(device.sysname(), "ID_INPUT_POINTINGSTICK"),
        trackball: tagged(device.sysname(), "ID_INPUT_TRACKBALL"),
    })
}

/// What the seat records about a libinput device: its name, ids, kind, capabilities and logical seat.
///
/// The number is not assigned here: [`Devices::announce`] numbers the draft, so the numbering
/// rules stay testable without a device.
///
/// A tablet pad is not given the pad capability. The toolkit's libinput source drops a pad's buttons, rings and strips
/// before they reach the queue this module reads, so announcing a pad would offer clients a device that can never
/// produce an event. The pad is a device with no capability until that source delivers its events; the mapper already
/// translates them (`TabletDeviceEvent::PadButton`, `PadRing`, `PadStrip`).
fn describe(device: &libinput::Device) -> DeviceInfo {
    let mut caps = DeviceCaps::NONE;
    for (capability, flag) in [
        (libinput::DeviceCapability::Keyboard, DeviceCaps::KEYBOARD),
        (libinput::DeviceCapability::Pointer, DeviceCaps::POINTER),
        (libinput::DeviceCapability::Touch, DeviceCaps::TOUCH),
        (libinput::DeviceCapability::Gesture, DeviceCaps::GESTURE),
        (libinput::DeviceCapability::TabletTool, DeviceCaps::TABLET_TOOL),
        (libinput::DeviceCapability::Switch, DeviceCaps::SWITCH),
    ] {
        if device.has_capability(capability) {
            caps = caps.union(flag);
        }
    }
    DeviceInfo {
        id: DeviceId::VIRTUAL,
        name: single_line(&device.name()),
        vendor: device.id_vendor(),
        product: device.id_product(),
        kind: kind_of(device),
        caps,
        seat: device.seat().logical_name().into_owned(),
        pad: PadLayout::default(),
    }
}

/// The finger count libinput reports, clamped to the range a gesture can carry.
fn fingers_of(fingers: u32) -> u8 {
    u8::try_from(fingers).unwrap_or(u8::MAX)
}

/// What one gesture sample carries besides its phase and kind.
#[derive(Clone, Copy)]
struct GestureStep {
    fingers: u8,
    delta: (f64, f64),
    scale: f64,
    angle: f64,
}

impl GestureStep {
    /// A sample that moved nothing: no fingers named, no movement, a scale of one.
    const REST: Self = Self {
        fingers: 0,
        delta: (0.0, 0.0),
        scale: 1.0,
        angle: 0.0,
    };

    /// A begin, which names its finger count and moves nothing.
    const fn begin(fingers: u8) -> Self {
        Self { fingers, ..Self::REST }
    }
}

/// The phase a gesture's end reports, given whether libinput cancelled it.
fn end_phase(cancelled: bool) -> RawTouchpadPhase {
    if cancelled {
        RawTouchpadPhase::Cancel
    } else {
        RawTouchpadPhase::End
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_taps_is_a_touchpad_whatever_else_it_says() {
        let touchpad = Facts {
            tap_fingers: 3,
            pointer: true,
            trackball: true,
            ..Facts::default()
        };
        assert_eq!(classify(touchpad), DeviceKind::Touchpad);
    }

    #[test]
    fn a_trackball_and_a_pointing_stick_are_what_udev_tags_them() {
        let pointer = Facts {
            pointer: true,
            ..Facts::default()
        };
        assert_eq!(classify(pointer), DeviceKind::Mouse, "an untagged pointer is a mouse");
        assert_eq!(
            classify(Facts {
                trackball: true,
                ..pointer
            }),
            DeviceKind::Trackball
        );
        assert_eq!(
            classify(Facts {
                pointing_stick: true,
                ..pointer
            }),
            DeviceKind::PointingStick
        );
    }

    #[test]
    fn a_keyboard_is_a_device_with_keys_and_no_pointer() {
        let keys = Facts {
            keyboard: true,
            ..Facts::default()
        };
        assert_eq!(classify(keys), DeviceKind::Keyboard);
        assert_eq!(
            classify(Facts { pointer: true, ..keys }),
            DeviceKind::Mouse,
            "a keyboard with a pointer of its own is the pointer's kind"
        );
        assert_eq!(classify(Facts { touch: true, ..keys }), DeviceKind::Touch);
    }

    #[test]
    fn a_device_with_no_capability_the_rule_knows_is_a_mouse() {
        // A drawing tablet or a lid switch. The settings layer reads the capabilities and gives it no pointer
        // section (`a_device_without_a_pointer_gets_no_pointer_setting`).
        assert_eq!(classify(Facts::default()), DeviceKind::Mouse);
    }
}
