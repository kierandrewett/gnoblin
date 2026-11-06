//! The GPU behind the hardware backend: the seat, the primary node, its connectors and their
//! KMS surfaces, and the apply path that turns a [`LayoutPlan`] into kernel operations.
//!
//! The split is the one [`super`] describes. [`apply_plan`] is the pure part: it walks the
//! plan, and when an operation is refused it restores the connectors it touched in reverse
//! order through [`Applied::restore`]. It is generic over [`Kms`], so the ordering is tested
//! with a recorder. [`Hardware`] is the one implementation that talks to smithay, and it has
//! no device to test against here, so everything it does is untested on hardware.
//!
//! Staging is what smithay's surface API does without a framebuffer: a mode or VRR change is
//! staged on the surface and the atomic commit happens with the first frame that carries a
//! plane, which is the render path that comes after this slice. A power-off is committed at
//! once, because [`DrmSurface::clear`] is itself a commit.

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gnoblin_output::{DisplayMode, Edid, VrrSupport};
use gnoblin_types::{Orientation, OutputId};
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmSurface, PlaneConfig, PlaneState, VrrSupport as KernelVrr,
};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, ResourceHandles, connector, crtc, framebuffer,
};
use smithay::utils::{Buffer, DeviceFd, Physical, Rectangle, Size, Transform};

use crate::drm::DrmError;
use crate::drm::gamma::{GammaError, GammaRamp, crtc_length};
use crate::drm::plan::{
    Applied, Change, Connector, Hotplug, LayoutPlan, Op, OutputState, PhysicalExtent, Power, Seen, hotplug,
};
use crate::session::Seat;
use crate::udev::{DeviceChange, DeviceOverrideSource, Devices, Discovery, DiscoveryEvent, DrmNode};

/// Why a plan could not be applied, and what it took to put the connectors back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyError {
    /// The connector whose operation was refused, or the one the plan named but the GPU does not have.
    pub connector: String,
    /// What the kernel, or the plan, said.
    pub reason: String,
    /// Restore operations that were refused too. Non-empty means the hardware is in a state the
    /// record does not describe, and the caller must not trust it.
    pub rollback_failures: Vec<String>,
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} refused the layout: {}", self.connector, self.reason)?;
        if !self.rollback_failures.is_empty() {
            write!(
                formatter,
                "; the rollback was also refused: {}",
                self.rollback_failures.join("; ")
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for ApplyError {}

/// What the kernel side of a set of connectors can do with one operation.
///
/// [`apply_plan`] is written against this trait so that its rollback ordering is tested without
/// a device. [`Hardware`] is the implementation that drives smithay.
pub trait Kms {
    /// The connector the name refers to, if the GPU has it.
    fn connector(&self, name: &str) -> Option<&Connector>;

    /// Asks the connector to take one operation.
    ///
    /// # Errors
    ///
    /// The reason the operation was refused. The connector is then in a state the caller must
    /// restore.
    fn apply(&mut self, connector: &str, op: &Op) -> Result<(), String>;
}

/// Applies every change of a plan, in plan order, and records the plan only if all of it held.
///
/// On the first refused operation the connector that refused is restored to what [`Applied`]
/// last confirmed for it, then every connector that already took its change is restored in
/// reverse order. The record is not touched on failure, so a later plan is computed against the
/// state the hardware really had.
///
/// # Errors
///
/// [`ApplyError`] naming the connector that refused, and any restore that was refused as well.
pub fn apply_plan<K: Kms>(kms: &mut K, applied: &mut Applied, plan: &LayoutPlan) -> Result<(), ApplyError> {
    // Connectors that already accepted their change, so a later refusal can undo them.
    let mut done: Vec<&Change> = Vec::new();
    // Ops for one connector run in order and stop at the first refusal. Connectors run in
    // plan order, so a refusal on one leaves earlier connectors to be rolled back below.
    for change in plan.all() {
        let outcome = if kms.connector(&change.connector).is_some() {
            change.ops.iter().try_for_each(|op| kms.apply(&change.connector, op))
        } else {
            Err(format!("the GPU has no connector named {}", change.connector))
        };
        if let Err(reason) = outcome {
            let mut rollback_failures = Vec::new();
            restore(kms, applied, &change.connector, &change.state, &mut rollback_failures);
            for earlier in done.iter().rev() {
                restore(kms, applied, &earlier.connector, &earlier.state, &mut rollback_failures);
            }
            return Err(ApplyError {
                connector: change.connector.clone(),
                reason,
                rollback_failures,
            });
        }
        done.push(change);
    }
    // Only a plan that fully held becomes the record. Recording a partial plan would make
    // the next diff assume hardware state that was rolled back.
    applied.record_plan(plan);
    Ok(())
}

/// Returns one connector from the state it was attempted in to the state last confirmed for it.
///
/// A connector with no confirmed state is left alone: there is nothing to return to, and the
/// plan that lit it was never recorded.
// Restore failures are collected, not returned early: every connector still gets its
// rollback attempt, and the caller sees the full list in `ApplyError::rollback_failures`.
fn restore<K: Kms>(kms: &mut K, applied: &Applied, name: &str, attempted: &OutputState, failures: &mut Vec<String>) {
    let Some(connector) = kms.connector(name).cloned() else {
        return;
    };
    // `applied.restore` decides what to write back from the record. `Ok(None)` means the
    // record has nothing confirmed for this connector, so there is nothing to undo.
    match applied.restore(&connector, attempted) {
        Ok(None) => {}
        Ok(Some(ops)) => {
            failures.extend(
                ops.iter()
                    .filter_map(|op| kms.apply(name, op).err().map(|reason| format!("{name}: {reason}"))),
            );
        }
        Err(error) => failures.push(format!("{name}: {error}")),
    }
}

/// Why a gamma or power request could not be applied to an output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// The GPU has no connector with that name.
    NoConnector(String),
    /// The table does not fit the CRTC, or the CRTC has no table.
    Gamma(GammaError),
    /// The kernel refused the operation.
    Refused(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConnector(name) => write!(formatter, "the GPU has no connector named {name}"),
            Self::Gamma(error) => write!(formatter, "{error}"),
            Self::Refused(reason) => write!(formatter, "the kernel refused the change: {reason}"),
        }
    }
}

impl std::error::Error for ControlError {}

/// Loads `ramp` onto the CRTC of connector `name`.
///
/// The table is checked against the CRTC's length before the kernel is asked, so a table of the
/// wrong length is a [`ControlError::Gamma`] and never reaches the device.
///
/// # Errors
///
/// [`ControlError`] naming the connector, the length, or the refusal.
pub fn set_gamma<K: Kms>(kms: &mut K, name: &str, ramp: &GammaRamp) -> Result<(), ControlError> {
    let connector = kms
        .connector(name)
        .ok_or_else(|| ControlError::NoConnector(name.to_owned()))?;
    // Length is checked here, not in the kernel, so a bad table never costs a device
    // round-trip. A zero-length CRTC table is caught by `GammaRamp::matches` too.
    ramp.matches(connector.gamma_length).map_err(ControlError::Gamma)?;
    kms.apply(name, &Op::Gamma(ramp.clone())).map_err(ControlError::Refused)
}

/// Puts the identity ramp back on the CRTC of connector `name`, at the CRTC's own length.
///
/// # Errors
///
/// [`ControlError`]: no such connector, a CRTC with no table, or a refusal.
pub fn restore_gamma<K: Kms>(kms: &mut K, name: &str) -> Result<(), ControlError> {
    let length = kms
        .connector(name)
        .ok_or_else(|| ControlError::NoConnector(name.to_owned()))?
        .gamma_length;
    if length == 0 {
        // A CRTC that reports no gamma table cannot be restored to identity, so the call fails
        // instead of writing an empty ramp that the kernel would reject with a less useful error.
        return Err(ControlError::Gamma(GammaError::Degenerate));
    }
    kms.apply(name, &Op::Gamma(GammaRamp::identity(length)))
        .map_err(ControlError::Refused)
}

/// Switches connector `name` on or off, recording the new power state when the plan that lit it
/// is known.
///
/// This is one operation, so a refusal leaves the connector as the kernel left it and the record
/// untouched.
///
/// # Errors
///
/// [`ControlError`]: no such connector or a refusal.
pub fn set_power<K: Kms>(kms: &mut K, applied: &mut Applied, name: &str, on: bool) -> Result<(), ControlError> {
    kms.connector(name)
        .ok_or_else(|| ControlError::NoConnector(name.to_owned()))?;
    let power = if on { Power::On } else { Power::Off };
    // Recorded only after the kernel accepted the change: a refused power-off must not look applied.
    kms.apply(name, &Op::Power(power)).map_err(ControlError::Refused)?;
    if let Some(current) = applied.state(name).copied() {
        let state = OutputState { power, ..current };
        applied.record(&Change {
            connector: name.to_owned(),
            state,
            ops: vec![Op::Power(power)],
        });
    }
    Ok(())
}

/// The refresh rate of a timing in millihertz, from its pixel clock and totals.
///
/// `clock_khz` is the kernel's pixel clock; the totals include blanking, which is why the rate is
/// computed rather than read from `vrefresh`, the rounded figure the kernel also reports.
#[must_use]
pub fn refresh_mhz(clock_khz: u32, htotal: u16, vtotal: u16) -> u32 {
    // The product is taken in u64 so no kernel timing can overflow; the division rounds down,
    // which matches the rounding the kernel uses for `vrefresh`.
    let frame = u64::from(htotal) * u64::from(vtotal);
    if frame == 0 {
        return 0;
    }
    // A timing whose totals do not fit in u32 millihertz reports 0 rather than wrapping.
    u32::try_from(u64::from(clock_khz) * 1_000_000 / frame).unwrap_or(0)
}

/// The kernel's timing as the mode the plan speaks.
#[must_use]
pub fn display_mode(mode: &Mode) -> DisplayMode {
    let (width, height) = mode.size();
    DisplayMode::new(
        u32::from(width),
        u32::from(height),
        refresh_mhz(mode.clock(), mode.hsync().2, mode.vsync().2),
    )
}

/// One connector's kernel side: its handle, what it is bound to, and the timings it offers.
struct Port {
    connector: Connector,
    /// The connector's handle, which a re-read of the connector needs.
    handle: connector::Handle,
    state: PortState,
    /// Each timing the connector offers, with the kernel mode it is and the plan's view of it.
    modes: Vec<(DisplayMode, Mode)>,
}

/// What a connector is bound to on the GPU. A bound connector has its CRTC and the surface that
/// drives it; an unbound one has neither, and its CRTC is free for another connector.
enum PortState {
    /// Nothing drives the connector.
    Unbound,
    /// The connector is driven on `crtc` through `surface`.
    Bound { crtc: crtc::Handle, surface: DrmSurface },
}

impl PortState {
    fn crtc(&self) -> Option<crtc::Handle> {
        match self {
            Self::Unbound => None,
            Self::Bound { crtc, .. } => Some(*crtc),
        }
    }

    /// The surface of the connector `name`, or why there is none.
    fn surface(&self, name: &str) -> Result<&DrmSurface, String> {
        match self {
            Self::Unbound => Err(format!("{name} has no KMS surface")),
            Self::Bound { surface, .. } => Ok(surface),
        }
    }
}

impl Port {
    fn crtc(&self) -> Option<crtc::Handle> {
        self.state.crtc()
    }

    fn surface(&self) -> Result<&DrmSurface, String> {
        self.state.surface(&self.connector.name)
    }
}

/// Every connector of the GPU, and the device they hang off.
pub struct Hardware {
    device: DrmDevice,
    ports: Vec<Port>,
}

impl Hardware {
    fn port(&self, name: &str) -> Result<&Port, String> {
        self.ports
            .iter()
            .find(|port| port.connector.name == name)
            .ok_or_else(|| format!("the GPU has no connector named {name}"))
    }
}

impl Kms for Hardware {
    fn connector(&self, name: &str) -> Option<&Connector> {
        self.ports
            .iter()
            .find(|port| port.connector.name == name)
            .map(|port| &port.connector)
    }

    fn apply(&mut self, name: &str, op: &Op) -> Result<(), String> {
        let port = self.port(name)?;
        match op {
            Op::Mode(mode) => {
                let (_, kernel) = port
                    .modes
                    .iter()
                    .find(|(offered, _)| offered == mode)
                    .ok_or_else(|| format!("{name} does not offer the mode {mode:?}"))?;
                port.surface()?.use_mode(*kernel).map_err(|error| error.to_string())
            }
            Op::Orientation(Orientation::Normal) => Ok(()),
            Op::Orientation(other) => Err(format!(
                "rotation to {other:?} is not built yet: outputs are not rotated until the frame path lands"
            )),
            Op::Vrr(enabled) => port.surface()?.use_vrr(*enabled).map_err(|error| error.to_string()),
            Op::Gamma(ramp) => {
                ramp.matches(port.connector.gamma_length)
                    .map_err(|error| error.to_string())?;
                let crtc = port
                    .crtc()
                    .ok_or_else(|| format!("{name} has no CRTC to take a gamma table"))?;
                let (red, green, blue) = ramp.channels();
                self.device
                    .device_fd()
                    .set_gamma(crtc, red, green, blue)
                    .map_err(|error| error.to_string())
            }
            Op::Power(Power::On) => Ok(()),
            Op::Power(Power::Off) => port.surface()?.clear().map_err(|error| error.to_string()),
        }
    }
}

/// The primary GPU went away, so nothing on this session can be driven any more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceLost {
    /// The node that was removed, such as `/dev/dri/card0`.
    pub node: PathBuf,
}

impl fmt::Display for DeviceLost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the primary GPU {} was removed", self.node.display())
    }
}

impl std::error::Error for DeviceLost {}

/// One thing [`Gpu::sync_hardware`] found changed since it last looked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HardwareChange {
    /// A connector gained a monitor and is driven now.
    OutputAdded(OutputId),
    /// A connector lost its monitor, so its output is gone.
    OutputRemoved(OutputId),
    /// The primary GPU was removed.
    DeviceLost(DeviceLost),
    /// A change that cannot be applied safely. The connector is left as it was.
    Refused {
        /// The connector the change was for.
        connector: String,
        /// Why it was refused.
        reason: String,
    },
}

/// Whether the primary GPU is still driven. Losing it is sticky: the live hardware is dropped at
/// the moment of loss, so nothing can be driven on that device again.
enum GpuState<H> {
    /// The device is open and its connectors are known.
    Live(H),
    /// The primary GPU was removed; the record names the node and the reason.
    Lost(DeviceLost),
}

impl<H> GpuState<H> {
    fn live(&self) -> Option<&H> {
        match self {
            Self::Live(hardware) => Some(hardware),
            Self::Lost(_) => None,
        }
    }

    fn live_mut(&mut self) -> Option<&mut H> {
        match self {
            Self::Live(hardware) => Some(hardware),
            Self::Lost(_) => None,
        }
    }

    fn lost(&self) -> Option<&DeviceLost> {
        match self {
            Self::Live(_) => None,
            Self::Lost(lost) => Some(lost),
        }
    }
}

/// An opened GPU: the seat that lent its node, the device, and the record of what was applied.
pub struct Gpu {
    state: GpuState<Hardware>,
    /// The device's file descriptor, handed out by [`Gpu::device_fd`] whatever the state.
    device_fd: DrmDeviceFd,
    notifier: Option<DrmDeviceNotifier>,
    /// The CRTCs whose page flips completed, oldest first. The notifier fills it and
    /// [`Gpu::take_flips`] drains it.
    flips: Arc<Mutex<VecDeque<crtc::Handle>>>,
    applied: Applied,
    /// The udev monitor that reports the device and its connectors changing. Drained by
    /// [`Gpu::sync_hardware`].
    discovery: Discovery,
    /// The GPUs this session knows about, and which one it drives.
    devices: Devices,
    /// Declared last so the device drops before the seat that lent its node.
    seat: Seat,
}

impl fmt::Debug for Gpu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Gpu")
            .field(
                "connectors",
                &self.state.live().map_or(0, |hardware| hardware.ports.len()),
            )
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Opens the seat, the primary GPU and every connector on it.
    ///
    /// A connector that is connected but has no CRTC, or whose surface cannot be created, is
    /// listed as disconnected: the session runs without it instead of failing.
    ///
    /// # Errors
    ///
    /// [`DrmError`] naming the step that failed: the seat, udev, the GPU choice, the device, or the
    /// GPU's resource list.
    pub fn open(seat_name: &str) -> Result<Self, DrmError> {
        let mut seat = Seat::open().map_err(DrmError::Session)?;
        if seat.name() != seat_name {
            return Err(DrmError::Seat(format!(
                "the session's seat is {}, not {seat_name}",
                seat.name()
            )));
        }
        let discovery = Discovery::new(seat.name()).map_err(|error| DrmError::Discovery(error.to_string()))?;
        let devices = Devices::new(discovery.snapshot(), &DeviceOverrideSource::read()).map_err(DrmError::NoGpu)?;
        let node: PathBuf = devices.primary().to_path_buf();

        let fd = seat.open_device(&node).map_err(DrmError::Session)?;
        let (mut device, notifier) = DrmDevice::new(DrmDeviceFd::new(DeviceFd::from(fd)), true)
            .map_err(|error| DrmError::Device(format!("{}: {error}", node.display())))?;
        let ports = enumerate(&mut device).map_err(DrmError::Device)?;
        let device_fd = device.device_fd().clone();
        Ok(Self {
            seat,
            state: GpuState::Live(Hardware { device, ports }),
            device_fd,
            notifier: Some(notifier),
            flips: Arc::new(Mutex::new(VecDeque::new())),
            applied: Applied::new(),
            discovery,
            devices,
        })
    }

    /// Every connector the GPU has, connected or not, in the order the kernel lists them.
    pub fn outputs(&self) -> impl Iterator<Item = &Connector> {
        self.state
            .live()
            .into_iter()
            .flat_map(|hardware| hardware.ports.iter().map(|port| &port.connector))
    }

    /// The primary GPU's removal, once it has happened.
    #[must_use]
    pub fn device_lost(&self) -> Option<&DeviceLost> {
        self.state.lost()
    }

    /// Applies what the udev monitor reported since the last call and says what it changed.
    ///
    /// A connector that gained or lost its monitor is brought up or taken down in place, and the
    /// returned changes say so. A change that cannot be applied safely is refused by name and the
    /// connector is left as it was. Losing the primary GPU is sticky: from then on this returns
    /// nothing, and [`Gpu::device_lost`] names the node.
    pub fn sync_hardware(&mut self) -> Vec<HardwareChange> {
        let mut changes = Vec::new();
        if self.state.lost().is_some() {
            return changes;
        }
        for event in self.discovery.take_events() {
            match event {
                DiscoveryEvent::Added(path) => {
                    self.devices.added(DrmNode::read(&path));
                }
                DiscoveryEvent::Removed(path) => {
                    if self.devices.removed(&path) == DeviceChange::PrimaryLost {
                        let lost = DeviceLost { node: path };
                        changes.push(HardwareChange::DeviceLost(lost.clone()));
                        self.state = GpuState::Lost(lost);
                        break;
                    }
                }
                DiscoveryEvent::Changed(path) => {
                    if self.devices.is_primary(&path) {
                        self.reprobe(&mut changes);
                    }
                }
            }
        }
        changes
    }

    /// Re-reads every connector the GPU drives and applies what changed since the last look.
    ///
    /// When the kernel cannot be read the whole pass is skipped: an unreadable connector would
    /// otherwise look unplugged.
    fn reprobe(&mut self, changes: &mut Vec<HardwareChange>) {
        let GpuState::Live(Hardware { device, ports }) = &mut self.state else {
            return;
        };
        let resources = match device.resource_handles() {
            Ok(resources) => resources,
            Err(error) => {
                gnoblin_log::warn_in!("backend", %error, "[drm] the GPU's resources could not be re-read; the change is skipped");
                return;
            }
        };
        let mut infos = Vec::with_capacity(ports.len());
        for port in ports.iter() {
            match device.get_connector(port.handle, true) {
                Ok(info) => infos.push(info),
                Err(error) => {
                    gnoblin_log::warn_in!("backend", connector = %port.connector.name, %error, "[drm] the connector could not be re-read; the change is skipped");
                    return;
                }
            }
        }
        let known: Vec<Connector> = ports.iter().map(|port| port.connector.clone()).collect();
        let seen: Vec<Seen> = infos
            .iter()
            .zip(ports.iter())
            .map(|(info, port)| Seen {
                name: port.connector.name.clone(),
                connected: matches!(info.state(), connector::State::Connected),
                modes: info.modes().iter().map(display_mode).collect(),
            })
            .collect();
        for change in hotplug(&known, &seen) {
            match change {
                Hotplug::Unplugged(name) => {
                    let Some(port) = ports.iter_mut().find(|port| port.connector.name == name) else {
                        continue;
                    };
                    port.connector.connected = false;
                    port.connector.vrr = VrrSupport::Unsupported;
                    port.connector.gamma_length = 0;
                    port.state = PortState::Unbound;
                    self.applied.forget(&name);
                    changes.push(HardwareChange::OutputRemoved(port.connector.id));
                }
                Hotplug::Plugged(name) => {
                    let Some(index) = ports.iter().position(|port| port.connector.name == name) else {
                        continue;
                    };
                    let taken: Vec<crtc::Handle> = ports.iter().filter_map(|port| port.crtc()).collect();
                    let id = ports[index].connector.id;
                    let handle = ports[index].handle;
                    let info = &infos[index];
                    let mut port = port_of(device, handle, info, id);
                    match bring_up_port(&mut port, device, &resources, info, &taken) {
                        Ok(()) => {
                            ports[index] = port;
                            changes.push(HardwareChange::OutputAdded(id));
                        }
                        Err(reason) => changes.push(HardwareChange::Refused {
                            connector: name,
                            reason,
                        }),
                    }
                }
                Hotplug::ModesChanged(name) => changes.push(HardwareChange::Refused {
                    connector: name,
                    reason: "its modes changed while it is driven, and a live output is not re-planned yet".to_owned(),
                }),
            }
        }
    }

    /// The seat that lent the GPU's node. It must outlive the device: dropping it revokes the node.
    #[must_use]
    pub fn seat(&self) -> &Seat {
        &self.seat
    }

    /// Registers the seat's event source with the loop, so VT switches reach
    /// [`Seat::take_events`], and the device's notifier, so page flips reach [`Gpu::take_flips`].
    ///
    /// # Errors
    ///
    /// The loop refuses one of the sources.
    pub fn register<S: 'static>(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        self.seat.register(loop_handle)?;
        self.discovery.register(loop_handle)?;
        let Some(notifier) = self.notifier.take() else {
            return Ok(());
        };
        let flips = Arc::clone(&self.flips);
        loop_handle
            .insert_source(notifier, move |event, _, _data| match event {
                DrmEvent::VBlank(crtc) => {
                    let mut queue = flips.lock().unwrap_or_else(|error| error.into_inner());
                    queue.push_back(crtc);
                }
                DrmEvent::Error(error) => {
                    gnoblin_log::warn_in!("backend", %error, "[drm] the device reported an error on its event queue");
                }
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// Releases DRM master for a VT switch away. Smithay's pause cannot fail.
    pub fn pause(&mut self) {
        if let Some(hardware) = self.state.live_mut() {
            hardware.device.pause();
        }
    }

    /// Takes DRM master back after a VT switch returns. Connectors are disabled, so the next
    /// modeset re-enables them rather than trusting the state the switch left behind.
    ///
    /// # Errors
    ///
    /// [`DrmError::Device`] when smithay cannot re-activate the device.
    pub fn activate(&mut self) -> Result<(), DrmError> {
        let Some(hardware) = self.state.live_mut() else {
            return Err(DrmError::Device("the primary GPU was removed".to_owned()));
        };
        hardware
            .device
            .activate(true)
            .map_err(|error| DrmError::Device(error.to_string()))
    }

    /// The CRTCs whose page flips completed since the last call, oldest first.
    pub fn take_flips(&self) -> Vec<crtc::Handle> {
        let mut queue = self.flips.lock().unwrap_or_else(|error| error.into_inner());
        queue.drain(..).collect()
    }

    /// The CRTC that scans out `output`, which is how a flip completion names its output.
    #[must_use]
    pub fn crtc_of(&self, output: OutputId) -> Option<crtc::Handle> {
        self.state
            .live()
            .and_then(|hardware| hardware.ports.iter().find(|port| port.connector.id == output))
            .and_then(|port| port.crtc())
    }

    /// The formats the primary plane of `crtc` can scan out, with their modifiers.
    #[must_use]
    pub fn plane_formats(&self, crtc: crtc::Handle) -> Option<smithay::backend::allocator::format::FormatSet> {
        let hardware = self.state.live()?;
        let port = hardware.ports.iter().find(|port| port.crtc() == Some(crtc))?;
        port.surface().ok().map(|surface| surface.plane_info().formats.clone())
    }

    /// The device node's file descriptor, which buffers and framebuffers are created on.
    #[must_use]
    pub fn device_fd(&self) -> DrmDeviceFd {
        self.device_fd.clone()
    }

    /// Queues a page flip on `crtc` that scans `framebuffer` out over the whole mode.
    ///
    /// The flip completes at the next vblank, which the notifier reports through [`Gpu::take_flips`].
    ///
    /// # Errors
    ///
    /// The reason the kernel refused the flip, or that no output is scanned out by `crtc`.
    pub fn queue_flip(
        &self,
        crtc: crtc::Handle,
        framebuffer: framebuffer::Handle,
        size: Size<i32, Physical>,
    ) -> Result<(), String> {
        let hardware = self
            .state
            .live()
            .ok_or_else(|| format!("the primary GPU was removed, so CRTC {crtc:?} cannot flip"))?;
        let port = hardware
            .ports
            .iter()
            .find(|port| port.crtc() == Some(crtc))
            .ok_or_else(|| format!("no output is scanned out by CRTC {crtc:?}"))?;
        let surface = port.surface()?;
        let plane = PlaneState {
            handle: surface.plane(),
            config: Some(PlaneConfig {
                src: Rectangle::<f64, Buffer>::from_size((f64::from(size.w), f64::from(size.h)).into()),
                dst: Rectangle::from_size(size),
                transform: Transform::Normal,
                alpha: 1.0,
                damage_clips: None,
                fb: framebuffer,
                fence: None,
            }),
        };
        surface.page_flip([plane], true).map_err(|error| error.to_string())
    }

    /// Applies a plan to the hardware, rolling back on the first refusal.
    ///
    /// # Errors
    ///
    /// [`ApplyError`] when an operation is refused; see [`apply_plan`].
    pub fn apply(&mut self, plan: &LayoutPlan) -> Result<(), ApplyError> {
        let hardware = match &mut self.state {
            GpuState::Live(hardware) => hardware,
            GpuState::Lost(lost) => return lost_refusal(plan, lost),
        };
        apply_plan(hardware, &mut self.applied, plan)
    }

    /// The number of entries in `output`'s gamma table, if the GPU has the output.
    #[must_use]
    pub fn gamma_length(&self, output: OutputId) -> Option<usize> {
        self.state
            .live()
            .and_then(|hardware| hardware.ports.iter().find(|port| port.connector.id == output))
            .map(|port| port.connector.gamma_length)
    }

    /// The live hardware, and the name of the connector `output` is on it.
    ///
    /// # Errors
    ///
    /// [`ControlError::NoConnector`] when the GPU is lost or has no connector for `output`.
    fn locate(state: &mut GpuState<Hardware>, output: OutputId) -> Result<(&mut Hardware, String), ControlError> {
        let hardware = state
            .live_mut()
            .ok_or_else(|| ControlError::NoConnector(output.to_string()))?;
        let name = hardware
            .ports
            .iter()
            .find(|port| port.connector.id == output)
            .map(|port| port.connector.name.clone())
            .ok_or_else(|| ControlError::NoConnector(output.to_string()))?;
        Ok((hardware, name))
    }

    /// Loads `ramp` onto the CRTC that scans out `output`.
    ///
    /// # Errors
    ///
    /// [`ControlError`] from [`set_gamma`], or [`ControlError::NoConnector`] for an unknown output.
    pub fn set_gamma(&mut self, output: OutputId, ramp: &GammaRamp) -> Result<(), ControlError> {
        let (hardware, name) = Self::locate(&mut self.state, output)?;
        set_gamma(hardware, &name, ramp)
    }

    /// Puts the identity ramp back on the CRTC that scans out `output`.
    ///
    /// # Errors
    ///
    /// [`ControlError`] from [`restore_gamma`], or [`ControlError::NoConnector`] for an unknown output.
    pub fn restore_gamma(&mut self, output: OutputId) -> Result<(), ControlError> {
        let (hardware, name) = Self::locate(&mut self.state, output)?;
        restore_gamma(hardware, &name)
    }

    /// Switches `output` on or off.
    ///
    /// # Errors
    ///
    /// [`ControlError`] from [`set_power`], or [`ControlError::NoConnector`] for an unknown output.
    pub fn set_power(&mut self, output: OutputId, on: bool) -> Result<(), ControlError> {
        let (hardware, name) = Self::locate(&mut self.state, output)?;
        set_power(hardware, &mut self.applied, &name, on)
    }

    /// What each connector was last confirmed to be doing.
    #[must_use]
    pub fn applied(&self) -> &Applied {
        &self.applied
    }
}

/// The refusal for a plan applied after the primary GPU was removed: nothing is driven any more.
fn lost_refusal(plan: &LayoutPlan, lost: &DeviceLost) -> Result<(), ApplyError> {
    Err(ApplyError {
        connector: plan
            .all()
            .next()
            .map(|change| change.connector.clone())
            .unwrap_or_default(),
        reason: format!("the primary GPU at {} was removed", lost.node.display()),
        rollback_failures: Vec::new(),
    })
}

/// The name a person sees the connector by, such as `DP-1`.
fn connector_name(info: &connector::Info) -> String {
    format!("{}-{}", info.interface().as_str(), info.interface_id())
}

/// Describes one connector the kernel reports, as a port nothing drives yet.
fn port_of(device: &DrmDevice, handle: connector::Handle, info: &connector::Info, id: OutputId) -> Port {
    let offered: Vec<(DisplayMode, Mode)> = info.modes().iter().map(|mode| (display_mode(mode), *mode)).collect();
    Port {
        handle,
        connector: Connector {
            id,
            name: connector_name(info),
            connected: false,
            edid: read_edid(device, handle),
            modes: offered.iter().map(|(mode, _)| *mode).collect(),
            physical: info
                .size()
                .map_or_else(PhysicalExtent::default, |(width, height)| PhysicalExtent {
                    width,
                    height,
                }),
            vrr: VrrSupport::Unsupported,
            gamma_length: 0,
        },
        state: PortState::Unbound,
        modes: offered,
    }
}

/// Brings a described connector up on a free CRTC, recording its CRTC, surface and capabilities.
fn bring_up_port(
    port: &mut Port,
    device: &mut DrmDevice,
    resources: &ResourceHandles,
    info: &connector::Info,
    taken: &[crtc::Handle],
) -> Result<(), String> {
    let up = bring_up(device, resources, info, taken)?;
    port.connector.connected = true;
    port.connector.vrr = up.vrr;
    port.connector.gamma_length = up.gamma_length;
    port.state = PortState::Bound {
        crtc: up.crtc,
        surface: up.surface,
    };
    Ok(())
}

/// Describes every connector and brings each connected one up on a CRTC of its own.
fn enumerate(device: &mut DrmDevice) -> Result<Vec<Port>, String> {
    let resources = device
        .resource_handles()
        .map_err(|error| format!("the GPU's resources could not be read: {error}"))?;
    let mut taken: Vec<crtc::Handle> = Vec::new();
    let mut ports = Vec::new();
    for handle in resources.connectors().to_vec() {
        let info = device
            .get_connector(handle, false)
            .map_err(|error| format!("connector {handle:?} could not be read: {error}"))?;
        let id = OutputId::new(u64::try_from(ports.len() + 1).unwrap_or(u64::MAX));
        let mut port = port_of(device, handle, &info, id);
        if matches!(info.state(), connector::State::Connected) {
            match bring_up_port(&mut port, device, &resources, &info, &taken) {
                Ok(()) => taken.extend(port.crtc()),
                Err(reason) => {
                    gnoblin_log::warn_in!("backend", connector = %port.connector.name, %reason, "[drm] the connector stays disconnected");
                }
            }
        }
        ports.push(port);
    }
    Ok(ports)
}

/// What a connected connector needs to be driven.
struct Up {
    crtc: crtc::Handle,
    surface: DrmSurface,
    vrr: VrrSupport,
    gamma_length: usize,
}

/// Finds a free CRTC for a connected connector and creates its surface on the mode it shows now,
/// or on its preferred mode when it shows none.
fn bring_up(
    device: &mut DrmDevice,
    resources: &ResourceHandles,
    info: &connector::Info,
    taken: &[crtc::Handle],
) -> Result<Up, String> {
    let crtc = choose_crtc(device, resources, info, taken)
        .map_err(|error| format!("its encoders could not be read: {error}"))?
        .ok_or_else(|| "no free CRTC can drive it".to_owned())?;
    let crtc_info = device
        .get_crtc(crtc)
        .map_err(|error| format!("CRTC {crtc:?} could not be read: {error}"))?;
    let mode = crtc_info
        .mode()
        .or_else(|| {
            info.modes()
                .iter()
                .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                .or_else(|| info.modes().first())
                .copied()
        })
        .ok_or_else(|| "it offers no modes".to_owned())?;
    let surface = device
        .create_surface(crtc, mode, &[info.handle()])
        .map_err(|error| format!("its surface could not be created: {error}"))?;
    let vrr = match surface.vrr_supported(info.handle()) {
        Ok(KernelVrr::Supported) => VrrSupport::Supported,
        // Enabling VRR on this panel needs a modeset the apply path does not do yet.
        Ok(KernelVrr::RequiresModeset | KernelVrr::NotSupported) | Err(_) => VrrSupport::Unsupported,
    };
    Ok(Up {
        crtc,
        surface,
        vrr,
        gamma_length: crtc_length(&crtc_info),
    })
}

/// The first CRTC an encoder of the connector can use that no other connector has taken.
fn choose_crtc(
    device: &DrmDevice,
    resources: &ResourceHandles,
    info: &connector::Info,
    taken: &[crtc::Handle],
) -> io::Result<Option<crtc::Handle>> {
    for encoder in info.encoders() {
        let encoder = device.get_encoder(*encoder)?;
        let candidates = encoder
            .crtc()
            .into_iter()
            .chain(resources.filter_crtcs(encoder.possible_crtcs()));
        if let Some(found) = candidates.into_iter().find(|crtc| !taken.contains(crtc)) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The connector's EDID, when the kernel exposes one that parses. A connector without one is
/// described without it, which the plan accepts.
fn read_edid(device: &DrmDevice, handle: connector::Handle) -> Option<Edid> {
    let properties = device.get_properties(handle).ok()?;
    for (property, value) in properties {
        let info = device.get_property(property).ok()?;
        if info.name().to_str() != Ok("EDID") {
            continue;
        }
        let blob = info.value_type().convert_value(value).as_blob()?;
        let bytes = device.get_property_blob(blob).ok()?;
        return Edid::parse(&bytes).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use gnoblin_types::Scale;

    use super::*;
    use crate::drm::plan;

    #[test]
    fn a_lost_gpu_has_no_live_hardware_and_names_its_node() {
        let mut live: GpuState<u8> = GpuState::Live(7);
        assert_eq!(live.live(), Some(&7));
        assert_eq!(live.live_mut().copied(), Some(7));
        assert!(live.lost().is_none());

        let node = PathBuf::from("/dev/dri/card0");
        let mut lost: GpuState<u8> = GpuState::Lost(DeviceLost { node: node.clone() });
        assert!(lost.live().is_none());
        assert!(lost.live_mut().is_none());
        assert_eq!(lost.lost().map(|lost| lost.node.clone()), Some(node));
    }

    #[test]
    fn an_unbound_port_has_no_crtc_and_no_surface() {
        assert_eq!(PortState::Unbound.crtc(), None);
        assert_eq!(
            PortState::Unbound.surface("DP-1").err(),
            Some("DP-1 has no KMS surface".to_owned())
        );
    }

    const FULL_HD: DisplayMode = DisplayMode::new(1920, 1080, 60_000);
    const FOUR_K: DisplayMode = DisplayMode::new(3840, 2160, 60_000);

    /// A recorder that refuses the operations whose call indices are listed.
    struct Recorder {
        connectors: Vec<Connector>,
        refuse: Vec<usize>,
        calls: usize,
        log: Vec<(String, Op)>,
    }

    impl Recorder {
        fn new(connectors: Vec<Connector>, refuse: Vec<usize>) -> Self {
            Self {
                connectors,
                refuse,
                calls: 0,
                log: Vec::new(),
            }
        }
    }

    impl Kms for Recorder {
        fn connector(&self, name: &str) -> Option<&Connector> {
            self.connectors.iter().find(|connector| connector.name == name)
        }

        fn apply(&mut self, connector: &str, op: &Op) -> Result<(), String> {
            let call = self.calls;
            self.calls += 1;
            if self.refuse.contains(&call) {
                return Err("refused".to_owned());
            }
            self.log.push((connector.to_owned(), op.clone()));
            Ok(())
        }
    }

    fn connector(name: &str, id: u64) -> Connector {
        Connector {
            id: OutputId::new(id),
            name: name.to_owned(),
            connected: true,
            edid: None,
            modes: vec![FULL_HD, FOUR_K],
            physical: PhysicalExtent::default(),
            vrr: VrrSupport::Supported,
            gamma_length: 4,
        }
    }

    fn state(mode: DisplayMode, vrr: bool) -> OutputState {
        let mut state = OutputState::lit(mode, Scale::integral(1), Orientation::Normal);
        state.vrr = vrr;
        state
    }

    fn plan_of(changes: Vec<Change>) -> LayoutPlan {
        LayoutPlan {
            changes,
            disabled: Vec::new(),
        }
    }

    fn change_from(connector: &Connector, from: Option<&OutputState>, to: OutputState) -> Change {
        Change {
            connector: connector.name.clone(),
            ops: plan::ops(connector, from, &to).expect("ops for a valid state"),
            state: to,
        }
    }

    #[test]
    fn refresh_is_computed_from_pixel_clock_and_totals() {
        assert_eq!(refresh_mhz(148_500, 2200, 1125), 60_000);
        assert_eq!(refresh_mhz(594_000, 4400, 2250), 60_000);
        assert_eq!(refresh_mhz(0, 2200, 1125), 0);
        assert_eq!(refresh_mhz(148_500, 0, 1125), 0);
    }

    #[test]
    fn a_plan_is_recorded_only_once_every_connector_took_it() {
        let a = connector("DP-1", 1);
        let b = connector("DP-2", 2);
        let mut kms = Recorder::new(vec![a.clone(), b.clone()], Vec::new());
        let mut applied = Applied::new();
        let plan = plan_of(vec![
            change_from(&a, None, state(FULL_HD, false)),
            change_from(&b, None, state(FOUR_K, false)),
        ]);

        apply_plan(&mut kms, &mut applied, &plan).expect("every operation holds");

        assert_eq!(applied.state("DP-1"), Some(&state(FULL_HD, false)));
        assert_eq!(applied.state("DP-2"), Some(&state(FOUR_K, false)));
        assert_eq!(kms.log.first().map(|(name, _)| name.as_str()), Some("DP-1"));
        assert!(
            kms.log
                .iter()
                .any(|(name, op)| name == "DP-2" && *op == Op::Mode(FOUR_K))
        );
    }

    #[test]
    fn a_gamma_table_of_another_length_is_refused_before_the_kernel_sees_it() {
        let mut kms = Recorder::new(vec![connector("DP-1", 1)], Vec::new());
        assert_eq!(
            set_gamma(&mut kms, "DP-1", &GammaRamp::identity(2)),
            Err(ControlError::Gamma(GammaError::Length { table: 2, hardware: 4 }))
        );
        assert!(kms.log.is_empty());
    }

    #[test]
    fn a_table_of_the_crtc_length_reaches_the_kernel_once() {
        let mut kms = Recorder::new(vec![connector("DP-1", 1)], Vec::new());
        set_gamma(&mut kms, "DP-1", &GammaRamp::identity(4)).expect("the CRTC's own length");
        assert!(matches!(&kms.log[..], [(name, Op::Gamma(_))] if name.as_str() == "DP-1"));
    }

    #[test]
    fn a_refused_gamma_is_named_not_swallowed() {
        let mut kms = Recorder::new(vec![connector("DP-1", 1)], vec![0]);
        assert_eq!(
            set_gamma(&mut kms, "DP-1", &GammaRamp::identity(4)),
            Err(ControlError::Refused("refused".to_owned()))
        );
    }

    #[test]
    fn restoring_gamma_writes_the_identity_at_the_crtc_length() {
        let mut kms = Recorder::new(vec![connector("DP-1", 1)], Vec::new());
        restore_gamma(&mut kms, "DP-1").expect("restore");
        assert!(matches!(&kms.log[..], [(_, Op::Gamma(ramp))] if ramp.length() == 4));
    }

    #[test]
    fn a_power_change_is_recorded_only_when_the_kernel_takes_it() {
        let a = connector("DP-1", 1);
        let mut applied = Applied::new();
        applied.record_plan(&plan_of(vec![change_from(&a, None, state(FULL_HD, false))]));

        let mut refusing = Recorder::new(vec![a.clone()], vec![0]);
        assert_eq!(
            set_power(&mut refusing, &mut applied, "DP-1", false),
            Err(ControlError::Refused("refused".to_owned()))
        );
        assert_eq!(applied.state("DP-1").map(|state| state.power), Some(Power::On));

        let mut kms = Recorder::new(vec![a], Vec::new());
        set_power(&mut kms, &mut applied, "DP-1", false).expect("power off");
        assert_eq!(applied.state("DP-1").map(|state| state.power), Some(Power::Off));
        assert!(matches!(&kms.log[..], [(_, Op::Power(Power::Off))]));
    }

    #[test]
    fn an_unknown_connector_is_named_for_gamma_and_power() {
        let mut kms = Recorder::new(vec![connector("DP-1", 1)], Vec::new());
        let mut applied = Applied::new();
        assert_eq!(
            set_power(&mut kms, &mut applied, "HDMI-9", true),
            Err(ControlError::NoConnector("HDMI-9".to_owned()))
        );
        assert_eq!(
            restore_gamma(&mut kms, "HDMI-9"),
            Err(ControlError::NoConnector("HDMI-9".to_owned()))
        );
    }

    #[test]
    fn a_refusal_restores_the_failed_connector_then_the_earlier_ones_in_reverse() {
        let a = connector("DP-1", 1);
        let b = connector("DP-2", 2);
        let before = state(FULL_HD, false);
        let mut applied = Applied::new();
        applied.record_plan(&plan_of(vec![
            change_from(&a, None, before),
            change_from(&b, None, before),
        ]));
        // Calls: 0 A's mode, 1 A's VRR, 2 B's mode, 3 B's VRR (refused).
        let mut kms = Recorder::new(vec![a.clone(), b.clone()], vec![3]);
        let next = state(FOUR_K, true);
        let plan = plan_of(vec![
            change_from(&a, Some(&before), next),
            change_from(&b, Some(&before), next),
        ]);

        let error = apply_plan(&mut kms, &mut applied, &plan).expect_err("the VRR operation is refused");

        assert_eq!(error.connector, "DP-2");
        assert_eq!(error.reason, "refused");
        assert!(error.rollback_failures.is_empty());
        assert_eq!(
            kms.log,
            vec![
                ("DP-1".to_owned(), Op::Mode(FOUR_K)),
                ("DP-1".to_owned(), Op::Vrr(true)),
                ("DP-2".to_owned(), Op::Mode(FOUR_K)),
                ("DP-2".to_owned(), Op::Mode(FULL_HD)),
                ("DP-2".to_owned(), Op::Vrr(false)),
                ("DP-1".to_owned(), Op::Mode(FULL_HD)),
                ("DP-1".to_owned(), Op::Vrr(false)),
            ]
        );
        assert_eq!(applied.state("DP-1"), Some(&before));
        assert_eq!(applied.state("DP-2"), Some(&before));
    }

    #[test]
    fn a_plan_naming_an_absent_connector_rolls_back_what_came_before_it() {
        let a = connector("DP-1", 1);
        let ghost = connector("HDMI-9", 9);
        let before = state(FULL_HD, false);
        let mut applied = Applied::new();
        applied.record_plan(&plan_of(vec![change_from(&a, None, before)]));
        let mut kms = Recorder::new(vec![a.clone()], Vec::new());
        let next = state(FOUR_K, false);
        let plan = plan_of(vec![
            change_from(&a, Some(&before), next),
            change_from(&ghost, None, next),
        ]);

        let error = apply_plan(&mut kms, &mut applied, &plan).expect_err("HDMI-9 is not on this GPU");

        assert_eq!(error.connector, "HDMI-9");
        assert!(error.reason.contains("no connector named HDMI-9"));
        assert_eq!(kms.log.last(), Some(&("DP-1".to_owned(), Op::Mode(FULL_HD))));
        assert_eq!(applied.state("DP-1"), Some(&before));
    }

    #[test]
    fn a_refused_rollback_is_reported_and_the_rest_still_runs() {
        let a = connector("DP-1", 1);
        let b = connector("DP-2", 2);
        let before = state(FULL_HD, false);
        let mut applied = Applied::new();
        applied.record_plan(&plan_of(vec![
            change_from(&a, None, before),
            change_from(&b, None, before),
        ]));
        // Calls: 0 A's mode, 1 B's mode, 2 B's VRR (refused), 3 B's restore of its mode (refused).
        let mut kms = Recorder::new(vec![a.clone(), b.clone()], vec![2, 3]);
        let next = state(FOUR_K, true);
        let plan = plan_of(vec![
            change_from(&a, Some(&before), next),
            change_from(&b, Some(&before), next),
        ]);

        let error = apply_plan(&mut kms, &mut applied, &plan).expect_err("the VRR operation is refused");

        assert_eq!(error.rollback_failures, vec!["DP-2: refused".to_owned()]);
        assert_eq!(kms.log.last().map(|(name, _)| name.as_str()), Some("DP-1"));
    }

    #[test]
    fn the_record_is_untouched_when_a_plan_fails() {
        let a = connector("DP-1", 1);
        let mut applied = Applied::new();
        let mut kms = Recorder::new(vec![a.clone()], vec![0]);
        let plan = plan_of(vec![change_from(&a, None, state(FOUR_K, false))]);

        apply_plan(&mut kms, &mut applied, &plan).expect_err("the first operation is refused");

        assert_eq!(applied.connectors().count(), 0);
    }
}
