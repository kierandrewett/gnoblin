//! XML protocol `tablet-v2` (`<protocol name="tablet_unstable_v2">`); interfaces served: `zwp_tablet_manager_v2`,
//! `zwp_tablet_seat_v2`, `zwp_tablet_v2`, `zwp_tablet_tool_v2`, `zwp_tablet_pad_v2`, `zwp_tablet_pad_group_v2`,
//! `zwp_tablet_pad_ring_v2`, `zwp_tablet_pad_strip_v2`.
//!
//! Drawing tablets: the tools a hand holds over one, and the pad beside it. Every backend delivers them as
//! [`TabletEvent`]s (`gnoblin_backend::tablet`), and this crate is what the compositor does with them.
//!
//! # Who does what
//!
//! - **Tools and tablets are Smithay's.** The manager global, the tablet seat, the tablet and tool objects and the
//!   delivery of a tool's events to the client under it are `smithay::wayland::tablet_manager`. This crate adds a
//!   tablet to the seat when a device joins (`DeviceAdded` with the tool capability), a tool when it first comes into
//!   proximity, and turns each event into one call on the tool, with the surface under the tool as its focus.
//! - **Pads are ours.** Smithay has no pad objects, so the pad, its group, rings and strips are created here for every
//!   tablet seat a client holds, and a pad's events go to the client that has the keyboard. A pad is associated with a
//!   tablet object of its own, because the pad's device and the pen's device are two devices to the backends.
//! - **A tablet seat is found after Smithay made it.** Smithay creates the `zwp_tablet_seat_v2` object inside its own
//!   request handler and keeps it out of reach, so the compositor's request tap notes the object number the client chose
//!   and [`resolve_new_seats`] looks the object up once the request is handled.
//!
//! # Failure modes first
//!
//! 1. **A tool's events reach a client that does not have it under the tool.** Smithay addresses each call to the
//!    focus passed with it, and the focus is the surface under the tool: a tool over client A's window is never seen
//!    by B: `tool_events_reach_the_client_under_the_tool` in `tests/tablet.rs`.
//! 2. **A tool is left hovering by an unplugged tablet.** The device's removal takes every tool that was over it out
//!    of proximity first: `a_removed_tablet_ends_its_tools` in `tests/tablet.rs`.
//! 3. **A locked session leaks a pen.** While locked, a tool entering or touching is ignored and only the events that
//!    end something go on ([`Host::tablets_locked`]). **No test drives it:** the lock needs an
//!    `ext-session-lock-v1` client and `tests/session_lock.rs` has the only one, with no tablet objects; the rule is
//!    the same `lock.state()` read every other input path makes.
//! 4. **A pad press goes to a client that is not typing.** PadRecord events follow the keyboard focus, so a background
//!    client cannot read the pad: `a_pad_reaches_the_client_with_the_keyboard` in `tests/tablet.rs`.
//! 5. **A pad announced to a client that is gone.** The pad table holds weak references and drops a client's
//!    objects when they die: `a_pad_of_a_client_that_left_is_forgotten` in `tests/tablet.rs`.
//! 6. **A tablet seat bound after the tablet joined misses it.** Smithay announces the tablets and tools that exist to
//!    a new seat, and the pads that exist are announced here: `a_client_that_binds_late_sees_the_pad` in
//!    `tests/tablet.rs`.
//!
//! A tool does not move the pointer: a client that binds no tablet protocol sees nothing of a tablet.

use std::collections::HashMap;

use smithay::backend::input::{ButtonState, InputTime, TabletToolCapabilities, TabletToolDescriptor, TabletToolType};
use smithay::input::tablet::tool::{
    AxisFrame, ButtonEvent, DownEvent, MotionEvent, ProximityInEvent, ProximityOutEvent, TabletToolHandle, UpEvent,
};
use smithay::input::tablet::{Tablet, TabletDescriptor, TabletSeat, TabletSeatHandler};
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::{
    zwp_tablet_manager_v2::ZwpTabletManagerV2,
    zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
    zwp_tablet_pad_ring_v2::{self, ZwpTabletPadRingV2},
    zwp_tablet_pad_strip_v2::{self, ZwpTabletPadStripV2},
    zwp_tablet_pad_v2::{self, ZwpTabletPadV2},
    zwp_tablet_seat_v2::ZwpTabletSeatV2,
    zwp_tablet_tool_v2::ZwpTabletToolV2,
    zwp_tablet_v2::ZwpTabletV2,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, Resource, Weak};
use smithay::utils::{Logical, Point as SmithayPoint, SERIAL_COUNTER};
use smithay::wayland::compositor::CompositorHandler;
use smithay::wayland::tablet_manager::{TabletManagerState, TabletSeatUserData, TabletToolUserData, TabletUserData};
use smithay::wayland::{Dispatch2, GlobalData};

use gnoblin_types::{DeviceId, OutputId, PointF};
use gnoblin_wayland_api::host::{CoreHost, OutputHost};
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use gnoblin_wayland_types::tablet::{PadLayout, TabletAxes, TabletEvent, TabletTool, ToolAxes, ToolKind};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-tablet-v2",
    interfaces: &[ServedInterface::new("zwp_tablet_manager_v2", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor.
pub trait Host:
    CoreHost
    + OutputHost
    + CompositorHandler
    + TabletSeatHandler<ToolFocus = WlSurface>
    + Dispatch<ZwpTabletV2, TabletUserData>
    + Dispatch<ZwpTabletToolV2, TabletToolUserData<Self>>
    + Dispatch<ZwpTabletPadV2, PadData>
    + Dispatch<ZwpTabletPadGroupV2, PadGroupData>
    + Dispatch<ZwpTabletPadRingV2, PadRingData>
    + Dispatch<ZwpTabletPadStripV2, PadStripData>
    + Dispatch<ZwpTabletV2, PadTabletData>
    + Sized
    + 'static
{
    /// The tablets, tools and pads.
    fn tablets(&mut self) -> &mut TabletTables<Self>;

    /// The seat's tablet half.
    fn tablet_seat(&self) -> TabletSeat<Self>;

    /// The surface under a point of the layout, and the point in its own coordinates. A locked session answers with its
    /// lock surface, if any.
    fn surface_under(&self, location: SmithayPoint<f64, Logical>) -> Option<(WlSurface, SmithayPoint<f64, Logical>)>;

    /// The surface that has the keyboard, which is where a pad's events go.
    fn keyboard_focus(&self) -> Option<WlSurface>;

    /// Whether the session is covered, so a pen starts nothing.
    fn tablets_locked(&self) -> bool;

    /// A pen pressed on a window at `location`: asks for its activation, as a pointer press does.
    fn pen_activates(&mut self, location: SmithayPoint<f64, Logical>);

    /// Remembers a serial handed to a client on a tool event, which a client may name in a grab or an activation.
    fn remember_serial(&mut self, serial: u32);
}

/// A device that joined the seat, as far as tablets care.
#[derive(Clone, Copy, Debug)]
pub struct TabletDevice<'a> {
    /// The device.
    pub id: DeviceId,
    /// Its name.
    pub name: &'a str,
    /// The USB vendor, or zero.
    pub vendor: u32,
    /// The USB product, or zero.
    pub product: u32,
    /// Whether it has the tool capability, and so is a `zwp_tablet_v2`.
    pub tool: bool,
    /// The layout of its pad, when it has the pad capability.
    pub pad: Option<PadLayout>,
}

/// What a tool the compositor has seen is doing.
struct TrackedTool<D: TabletSeatHandler + 'static> {
    handle: TabletToolHandle<D>,
    /// The tablet it is over while in proximity.
    device: Option<DeviceId>,
    /// Where it is, in the layout's space.
    location: SmithayPoint<f64, Logical>,
    /// Whether its tip is down.
    tip: bool,
}

/// One client's objects for one pad.
pub struct PadInstance {
    client: ClientId,
    pad: Weak<ZwpTabletPadV2>,
    /// The tablet object the pad is associated with, which `enter` names.
    tablet: Weak<ZwpTabletV2>,
    rings: Vec<Weak<ZwpTabletPadRingV2>>,
    strips: Vec<Weak<ZwpTabletPadStripV2>>,
    /// The surface the pad entered, while it has.
    entered: Option<Weak<WlSurface>>,
}

/// A pad the seat has and the clients it was announced to.
struct PadRecord {
    device: DeviceId,
    name: String,
    layout: PadLayout,
    instances: Vec<PadInstance>,
}

/// A tablet seat a client holds, which a pad is announced to.
struct SeatRef {
    seat: Weak<ZwpTabletSeatV2>,
}

/// Everything the compositor keeps about tablets.
pub struct TabletTables<D: TabletSeatHandler + 'static> {
    /// Kept alive so the global stays registered.
    _manager: TabletManagerState,
    tablets: HashMap<DeviceId, (TabletDescriptor, Tablet)>,
    tools: HashMap<TabletTool, TrackedTool<D>>,
    pads: Vec<PadRecord>,
    seats: Vec<SeatRef>,
    /// Tablet seat objects a client asked for whose number is known but whose object is not looked up yet.
    new_seats: Vec<(Client, u32)>,
    /// How many tablets were made up for a tool that came from a device nobody announced.
    unannounced: u32,
}

impl<D: TabletSeatHandler + 'static> std::fmt::Debug for TabletTables<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TabletTables")
            .field("tablets", &self.tablets.len())
            .field("tools", &self.tools.len())
            .field("pads", &self.pads.len())
            .finish_non_exhaustive()
    }
}

/// Creates the `zwp_tablet_manager_v2` global.
pub fn register<D>(registrar: &Registrar<'_>) -> TabletTables<D>
where
    D: GlobalDispatch<ZwpTabletManagerV2, GlobalData>
        + Dispatch<ZwpTabletManagerV2, GlobalData>
        + Dispatch<ZwpTabletSeatV2, TabletSeatUserData<D>>
        + Dispatch<ZwpTabletToolV2, TabletToolUserData<D>>
        + TabletSeatHandler
        + 'static,
{
    TabletTables {
        _manager: TabletManagerState::new::<D>(registrar.display()),
        tablets: HashMap::new(),
        tools: HashMap::new(),
        pads: Vec::new(),
        seats: Vec::new(),
        new_seats: Vec::new(),
        unannounced: 0,
    }
}

// ---- the objects a pad is made of ------------------------------------------------------------------------------------

/// User data of a pad object.
pub struct PadData;
/// User data of a pad's group.
pub struct PadGroupData;
/// User data of a pad's ring.
pub struct PadRingData;
/// User data of a pad's strip.
pub struct PadStripData;
/// User data of the tablet object a pad is associated with.
pub struct PadTabletData;

impl<D> Dispatch2<ZwpTabletPadV2, D> for PadData {
    /// A pad's requests are feedback for the pad's LEDs and the destructor, which the library handles: there is
    /// nothing to light, so nothing is done.
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpTabletPadV2,
        _request: zwp_tablet_pad_v2::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}

impl<D> Dispatch2<ZwpTabletPadGroupV2, D> for PadGroupData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpTabletPadGroupV2,
        _request: <ZwpTabletPadGroupV2 as Resource>::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}

impl<D> Dispatch2<ZwpTabletPadRingV2, D> for PadRingData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpTabletPadRingV2,
        _request: zwp_tablet_pad_ring_v2::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}

impl<D> Dispatch2<ZwpTabletPadStripV2, D> for PadStripData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpTabletPadStripV2,
        _request: zwp_tablet_pad_strip_v2::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}

impl<D> Dispatch2<ZwpTabletV2, D> for PadTabletData {
    fn request(
        &self,
        _state: &mut D,
        _client: &Client,
        _resource: &ZwpTabletV2,
        _request: <ZwpTabletV2 as Resource>::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
    }
}

/// The object number a client chose for a new object, read from what the request prints.
///
/// Smithay's `GetTabletSeat` handler initialises the new tablet seat itself and hands nothing back, and the request's
/// `New` wrapper has no accessor. The wrapper's `Debug` output names the object as `interface@number`, which is what is
/// read. A request whose print has no number yields `None` and the seat is simply not given pads: the tools and tablets
/// Smithay serves do not depend on it. `tests/tablet.rs` proves the number is found.
pub fn protocol_id_of(new: &impl std::fmt::Debug) -> Option<u32> {
    let text = format!("{new:?}");
    let at = text.find('@')?;
    let digits: String = text[at + 1..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The tablet seat a client asked for through `get_tablet_seat`, by the object number it chose. Called by the request tap
/// before Smithay handles the request.
pub fn seat_requested<D: Host>(state: &mut D, client: Option<Client>, number: Option<u32>) {
    if let (Some(client), Some(number)) = (client, number) {
        state.tablets().new_seats.push((client, number));
    }
}

/// Looks up the tablet seats requested since the last call and announces the pads that exist to each. Called by the
/// request tap after Smithay handled a request.
pub fn resolve_new_seats<D: Host>(state: &mut D, display: &DisplayHandle) {
    if state.tablets().new_seats.is_empty() {
        return;
    }
    for (client, number) in std::mem::take(&mut state.tablets().new_seats) {
        let Ok(seat) = client.object_from_protocol_id::<ZwpTabletSeatV2>(display, number) else {
            continue;
        };
        state.tablets().seats.retain(|known| known.seat.upgrade().is_ok());
        state.tablets().seats.push(SeatRef { seat: seat.downgrade() });
        let pads: Vec<(DeviceId, String, PadLayout)> = state
            .tablets()
            .pads
            .iter()
            .map(|pad| (pad.device, pad.name.clone(), pad.layout))
            .collect();
        for (device, name, layout) in pads {
            if let Some(instance) = announce_pad::<D>(display, &client, &seat, device, &name, layout)
                && let Some(pad) = state.tablets().pads.iter_mut().find(|pad| pad.device == device)
            {
                pad.instances.push(instance);
            }
        }
    }
}

/// Creates one client's objects for a pad: the tablet it is associated with, the pad, its one group, and the rings and
/// strips, each announced in the order the protocol states.
fn announce_pad<D: Host>(
    display: &DisplayHandle,
    client: &Client,
    seat: &ZwpTabletSeatV2,
    device: DeviceId,
    name: &str,
    layout: PadLayout,
) -> Option<PadInstance> {
    let version = seat.version();
    let tablet = client
        .create_resource::<ZwpTabletV2, _, D>(display, version, PadTabletData)
        .ok()?;
    seat.tablet_added(&tablet);
    tablet.name(name.to_owned());
    tablet.done();
    let pad = client
        .create_resource::<ZwpTabletPadV2, _, D>(display, version, PadData)
        .ok()?;
    seat.pad_added(&pad);
    pad.path(format!("gnoblin-tablet-pad-{device}"));
    pad.buttons(layout.buttons);
    let group = client
        .create_resource::<ZwpTabletPadGroupV2, _, D>(display, version, PadGroupData)
        .ok()?;
    pad.group(&group);
    let buttons: Vec<u8> = (0..layout.buttons).flat_map(u32::to_ne_bytes).collect();
    group.buttons(buttons);
    let mut rings = Vec::new();
    for _ in 0..layout.rings {
        let ring = client
            .create_resource::<ZwpTabletPadRingV2, _, D>(display, version, PadRingData)
            .ok()?;
        group.ring(&ring);
        rings.push(ring.downgrade());
    }
    let mut strips = Vec::new();
    for _ in 0..layout.strips {
        let strip = client
            .create_resource::<ZwpTabletPadStripV2, _, D>(display, version, PadStripData)
            .ok()?;
        group.strip(&strip);
        strips.push(strip.downgrade());
    }
    group.modes(1);
    group.done();
    pad.done();
    Some(PadInstance {
        client: client.id(),
        pad: pad.downgrade(),
        tablet: tablet.downgrade(),
        rings,
        strips,
        entered: None,
    })
}

// ---- the seat's half -------------------------------------------------------------------------------------------------

/// The Smithay type of a tool, for a kind. A finger has none: it is a pen on the wire.
fn tool_type(kind: ToolKind) -> TabletToolType {
    match kind {
        ToolKind::Pen | ToolKind::Finger => TabletToolType::Pen,
        ToolKind::Eraser => TabletToolType::Eraser,
        ToolKind::Brush => TabletToolType::Brush,
        ToolKind::Pencil => TabletToolType::Pencil,
        ToolKind::Airbrush => TabletToolType::Airbrush,
        ToolKind::Mouse => TabletToolType::Mouse,
        ToolKind::Lens => TabletToolType::Lens,
    }
}

/// The descriptor of a tool.
fn descriptor_of(tool: &TabletTool) -> TabletToolDescriptor {
    let mut capabilities = TabletToolCapabilities::empty();
    for (axis, capability) in [
        (ToolAxes::TILT, TabletToolCapabilities::TILT),
        (ToolAxes::PRESSURE, TabletToolCapabilities::PRESSURE),
        (ToolAxes::DISTANCE, TabletToolCapabilities::DISTANCE),
        (ToolAxes::ROTATION, TabletToolCapabilities::ROTATION),
        (ToolAxes::SLIDER, TabletToolCapabilities::SLIDER),
        (ToolAxes::WHEEL, TabletToolCapabilities::WHEEL),
    ] {
        if tool.axes.contains(axis) {
            capabilities |= capability;
        }
    }
    TabletToolDescriptor {
        tool_type: tool_type(tool.kind),
        hardware_serial: tool.serial,
        hardware_id_wacom: tool.hardware_id,
        capabilities,
    }
}

/// The axes a tool event changed, as Smithay's frame.
fn frame_of(axes: &TabletAxes) -> AxisFrame {
    let mut frame = AxisFrame::new();
    if let Some(pressure) = axes.pressure {
        frame = frame.pressure(pressure.clamp(0.0, 1.0));
    }
    if let Some(distance) = axes.distance {
        frame = frame.distance(distance.clamp(0.0, 1.0));
    }
    if let Some((x, y)) = axes.tilt {
        frame = frame.tilt(x, y);
    }
    if let Some(rotation) = axes.rotation {
        frame = frame.rotation(rotation);
    }
    if let Some(slider) = axes.slider {
        frame = frame.slider(slider.clamp(-1.0, 1.0));
    }
    if let Some((degrees, clicks)) = axes.wheel {
        frame = frame.wheel(degrees, clicks);
    }
    frame
}

/// A device joined the seat: a tablet with the tool capability becomes a `zwp_tablet_v2`, and a pad is announced to
/// every tablet seat that exists.
pub fn device_added<D: Host>(state: &mut D, device: &TabletDevice<'_>) {
    if device.tool {
        add_tablet(state, device.id, device.name, device.vendor, device.product);
    }
    if let Some(layout) = device.pad {
        add_pad(state, device.id, device.name, layout);
    }
}

fn add_tablet<D: Host>(state: &mut D, device: DeviceId, name: &str, vendor: u32, product: u32) {
    let descriptor = TabletDescriptor {
        name: name.to_owned(),
        // The protocol states vendor then product.
        usb_id: (vendor != 0 || product != 0).then_some((vendor, product)),
        syspath: None,
    };
    let display = state.display_handle().clone();
    let tablet = state.tablet_seat().add_wp_tablet(&display, &descriptor);
    log::debug!(area = "tablet", %device, name, "a tablet joined the seat");
    state.tablets().tablets.insert(device, (descriptor, tablet));
}

fn add_pad<D: Host>(state: &mut D, device: DeviceId, name: &str, layout: PadLayout) {
    let display = state.display_handle().clone();
    let mut instances = Vec::new();
    let tables = state.tablets();
    tables.seats.retain(|known| known.seat.upgrade().is_ok());
    for known in &tables.seats {
        let Ok(seat) = known.seat.upgrade() else {
            continue;
        };
        let Some(client) = seat.client() else {
            continue;
        };
        if let Some(instance) = announce_pad::<D>(&display, &client, &seat, device, name, layout) {
            instances.push(instance);
        }
    }
    log::debug!(area = "tablet", %device, name, "a pad joined the seat");
    tables.pads.push(PadRecord {
        device,
        name: name.to_owned(),
        layout,
        instances,
    });
}

/// A device left: its tools leave proximity first, then the tablet or pad goes.
pub fn device_removed<D: Host>(state: &mut D, id: DeviceId) {
    let time = InputTime::now();
    let over: Vec<TabletTool> = state
        .tablets()
        .tools
        .iter()
        .filter(|(_, tool)| tool.device == Some(id))
        .map(|(tool, _)| *tool)
        .collect();
    for tool in over {
        tool_out(state, id, tool, time);
    }
    if let Some((descriptor, _)) = state.tablets().tablets.remove(&id) {
        state.tablet_seat().remove_tablet(&descriptor);
    }
    let tables = state.tablets();
    if let Some(index) = tables.pads.iter().position(|pad| pad.device == id) {
        let pad = tables.pads.remove(index);
        for instance in pad.instances {
            if let Ok(resource) = instance.pad.upgrade() {
                resource.removed();
            }
            if let Ok(tablet) = instance.tablet.upgrade() {
                tablet.removed();
            }
        }
    }
}

/// The tablet a device is, made up when a tool arrives from a device nobody announced.
fn tablet_of<D: Host>(state: &mut D, device: DeviceId) -> Tablet {
    if let Some((_, tablet)) = state.tablets().tablets.get(&device) {
        return tablet.clone();
    }
    let tables = state.tablets();
    tables.unannounced += 1;
    let name = format!("Tablet {}", tables.unannounced);
    add_tablet(state, device, &name, 0, 0);
    state.tablets().tablets[&device].1.clone()
}

/// The tool a description names, added to the seat the first time it is seen.
fn tool_of<D: Host>(state: &mut D, tool: TabletTool) -> TabletToolHandle<D> {
    if let Some(known) = state.tablets().tools.get(&tool) {
        return known.handle.clone();
    }
    let display = state.display_handle().clone();
    let seat = state.tablet_seat();
    let handle = seat.add_wp_tool(state, &display, &descriptor_of(&tool));
    state.tablets().tools.insert(
        tool,
        TrackedTool {
            handle: handle.clone(),
            device: None,
            location: SmithayPoint::from((0.0, 0.0)),
            tip: false,
        },
    );
    handle
}

/// An output-local position as a point of the layout, or `None` for an output the session does not have.
fn layout_point<D: Host>(state: &D, output: OutputId, position: PointF) -> Option<SmithayPoint<f64, Logical>> {
    let rect = state.output_rect(output)?;
    Some(SmithayPoint::from((
        position.x + f64::from(rect.x),
        position.y + f64::from(rect.y),
    )))
}

/// One tablet event, from any backend.
pub fn input<D: Host>(state: &mut D, event: TabletEvent) {
    let time = InputTime::now();
    match event {
        TabletEvent::ToolIn {
            device,
            output,
            tool,
            position,
            axes,
        } => {
            if state.tablets_locked() {
                return;
            }
            let Some(location) = layout_point(state, output, position) else {
                return;
            };
            let tablet = tablet_of(state, device);
            let handle = tool_of(state, tool);
            let focus = state.surface_under(location);
            if let Some(known) = state.tablets().tools.get_mut(&tool) {
                known.device = Some(device);
                known.location = location;
                known.tip = false;
            }
            let axis = (!axes.is_empty()).then(|| frame_of(&axes));
            handle.proximity_in(
                state,
                focus,
                tablet,
                &ProximityInEvent {
                    location,
                    axis,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            handle.frame(state, time);
        }
        TabletEvent::ToolMotion {
            device: _,
            output,
            tool,
            position,
            axes,
        } => {
            if state.tablets_locked() {
                return;
            }
            let Some(location) = layout_point(state, output, position) else {
                return;
            };
            let Some(known) = state.tablets().tools.get_mut(&tool) else {
                return;
            };
            if known.device.is_none() {
                return;
            }
            known.location = location;
            let handle = known.handle.clone();
            let focus = state.surface_under(location);
            handle.motion(
                state,
                focus,
                &MotionEvent {
                    location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            if !axes.is_empty() {
                handle.axis(state, frame_of(&axes));
            }
            handle.frame(state, time);
        }
        TabletEvent::ToolTip { tool, down, .. } => {
            // A tip that lands while locked starts nothing; one that lifts is always let through.
            let locked = state.tablets_locked();
            let Some(known) = state.tablets().tools.get_mut(&tool) else {
                return;
            };
            if known.device.is_none() || known.tip == down || (down && locked) {
                return;
            }
            known.tip = down;
            let (handle, location) = (known.handle.clone(), known.location);
            let serial = SERIAL_COUNTER.next_serial();
            state.remember_serial(u32::from(serial));
            if down {
                // A pen pressed on a window asks for activation, as a pointer press does.
                state.pen_activates(location);
                handle.down(state, &DownEvent { serial, time });
            } else {
                handle.up(state, &UpEvent { serial, time });
            }
            handle.frame(state, time);
        }
        TabletEvent::ToolButton {
            tool, button, pressed, ..
        } => {
            let locked = state.tablets_locked();
            let Some(known) = state.tablets().tools.get(&tool) else {
                return;
            };
            if known.device.is_none() || (pressed && locked) {
                return;
            }
            let handle = known.handle.clone();
            handle.button(
                state,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    button,
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                    time,
                },
            );
            handle.frame(state, time);
        }
        TabletEvent::ToolOut { device, tool } => tool_out(state, device, tool, time),
        TabletEvent::PadButton {
            device,
            button,
            pressed,
        } => {
            let wire = if pressed {
                zwp_tablet_pad_v2::ButtonState::Pressed
            } else {
                zwp_tablet_pad_v2::ButtonState::Released
            };
            pad_event(state, device, time, |instance, _, time| {
                if let Ok(pad) = instance.pad.upgrade() {
                    pad.button(time, button, wire);
                }
            });
        }
        TabletEvent::PadRing { device, ring, angle } => {
            pad_event(state, device, time, |instance, _, time| {
                let Some(Ok(ring)) = instance.rings.get(ring as usize).map(Weak::upgrade) else {
                    return;
                };
                ring.source(zwp_tablet_pad_ring_v2::Source::Finger);
                match angle {
                    Some(degrees) => ring.angle(degrees),
                    None => ring.stop(),
                }
                ring.frame(time);
            });
        }
        TabletEvent::PadStrip {
            device,
            strip,
            position,
        } => {
            pad_event(state, device, time, |instance, _, time| {
                let Some(Ok(strip)) = instance.strips.get(strip as usize).map(Weak::upgrade) else {
                    return;
                };
                strip.source(zwp_tablet_pad_strip_v2::Source::Finger);
                match position {
                    // The wire states a strip as 0 to 65535.
                    Some(fraction) => strip.position((fraction.clamp(0.0, 1.0) * 65535.0) as u32),
                    None => strip.stop(),
                }
                strip.frame(time);
            });
        }
    }
}

/// Ends a tool's stay: its tip lifts and it leaves proximity.
fn tool_out<D: Host>(state: &mut D, device: DeviceId, tool: TabletTool, time: InputTime) {
    let Some(known) = state.tablets().tools.get_mut(&tool) else {
        return;
    };
    if known.device != Some(device) {
        return;
    }
    known.device = None;
    let tip = std::mem::take(&mut known.tip);
    let handle = known.handle.clone();
    if tip {
        handle.up(
            state,
            &UpEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }
    handle.proximity_out(
        state,
        &ProximityOutEvent {
            serial: SERIAL_COUNTER.next_serial(),
            time,
        },
    );
    handle.frame(state, time);
}

/// Delivers one pad event to the client that has the keyboard, entering the pad on its surface first. A locked
/// session delivers none.
fn pad_event<D: Host>(
    state: &mut D,
    device: DeviceId,
    time: InputTime,
    deliver: impl Fn(&PadInstance, &WlSurface, u32),
) {
    if state.tablets_locked() {
        return;
    }
    let focus = state.keyboard_focus();
    let Some(pad) = state.tablets().pads.iter_mut().find(|pad| pad.device == device) else {
        return;
    };
    pad.instances.retain(|instance| instance.pad.upgrade().is_ok());
    for instance in &mut pad.instances {
        let entered = instance.entered.as_ref().and_then(|surface| surface.upgrade().ok());
        let wanted = focus
            .as_ref()
            .filter(|surface| surface.client().is_some_and(|client| client.id() == instance.client));
        if entered.as_ref() != wanted {
            if let (Some(surface), Ok(pad_resource)) = (entered, instance.pad.upgrade()) {
                pad_resource.leave(u32::from(SERIAL_COUNTER.next_serial()), &surface);
            }
            instance.entered = None;
            if let (Some(surface), Ok(pad_resource), Ok(tablet)) =
                (wanted, instance.pad.upgrade(), instance.tablet.upgrade())
            {
                pad_resource.enter(u32::from(SERIAL_COUNTER.next_serial()), &tablet, surface);
                instance.entered = Some(surface.downgrade());
            }
        }
        if let Some(surface) = wanted.filter(|_| instance.entered.is_some()) {
            deliver(instance, surface, time.millis());
        }
    }
}
