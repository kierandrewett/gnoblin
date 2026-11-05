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
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmSurface, VrrSupport as KernelVrr};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, ResourceHandles, connector, crtc,
};
use smithay::utils::DeviceFd;

use crate::drm::DrmError;
use crate::drm::gamma::crtc_length;
use crate::drm::plan::{Applied, Change, Connector, LayoutPlan, Op, OutputState, PhysicalExtent, Power};
use crate::session::Seat;
use crate::udev::{DeviceOverrideSource, Devices, Discovery};

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
    let mut done: Vec<&Change> = Vec::new();
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
    applied.record_plan(plan);
    Ok(())
}

/// Returns one connector from the state it was attempted in to the state last confirmed for it.
///
/// A connector with no confirmed state is left alone: there is nothing to return to, and the
/// plan that lit it was never recorded.
fn restore<K: Kms>(kms: &mut K, applied: &Applied, name: &str, attempted: &OutputState, failures: &mut Vec<String>) {
    let Some(connector) = kms.connector(name).cloned() else {
        return;
    };
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

/// The refresh rate of a timing in millihertz, from its pixel clock and totals.
///
/// `clock_khz` is the kernel's pixel clock; the totals include blanking, which is why the rate is
/// computed rather than read from `vrefresh`, the rounded figure the kernel also reports.
#[must_use]
pub fn refresh_mhz(clock_khz: u32, htotal: u16, vtotal: u16) -> u32 {
    let frame = u64::from(htotal) * u64::from(vtotal);
    if frame == 0 {
        return 0;
    }
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

/// One connector's kernel side: its handle, its CRTC and surface, and the timings it offers.
struct Port {
    connector: Connector,
    crtc: Option<crtc::Handle>,
    /// Each timing the connector offers, with the kernel mode it is and the plan's view of it.
    modes: Vec<(DisplayMode, Mode)>,
    surface: Option<DrmSurface>,
}

impl Port {
    fn surface(&self) -> Result<&DrmSurface, String> {
        self.surface
            .as_ref()
            .ok_or_else(|| format!("{} has no KMS surface", self.connector.name))
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
                    .crtc
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

/// An opened GPU: the seat that lent its node, the device, and the record of what was applied.
pub struct Gpu {
    hardware: Hardware,
    notifier: Option<DrmDeviceNotifier>,
    /// The CRTCs whose page flips completed, oldest first. The notifier fills it and
    /// [`Gpu::take_flips`] drains it.
    flips: Arc<Mutex<VecDeque<crtc::Handle>>>,
    applied: Applied,
    /// Declared last so the device drops before the seat that lent its node.
    seat: Seat,
}

impl fmt::Debug for Gpu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Gpu")
            .field("connectors", &self.hardware.ports.len())
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
        drop(devices);
        drop(discovery);

        let fd = seat.open_device(&node).map_err(DrmError::Session)?;
        let (mut device, notifier) = DrmDevice::new(DrmDeviceFd::new(DeviceFd::from(fd)), true)
            .map_err(|error| DrmError::Device(format!("{}: {error}", node.display())))?;
        let ports = enumerate(&mut device).map_err(DrmError::Device)?;
        Ok(Self {
            seat,
            hardware: Hardware { device, ports },
            notifier: Some(notifier),
            flips: Arc::new(Mutex::new(VecDeque::new())),
            applied: Applied::new(),
        })
    }

    /// Every connector the GPU has, connected or not, in the order the kernel lists them.
    pub fn outputs(&self) -> impl Iterator<Item = &Connector> {
        self.hardware.ports.iter().map(|port| &port.connector)
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
                    tracing::warn!(%error, "[drm] the device reported an error on its event queue");
                }
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// Releases DRM master for a VT switch away. Smithay's pause cannot fail.
    pub fn pause(&mut self) {
        self.hardware.device.pause();
    }

    /// Takes DRM master back after a VT switch returns. Connectors are disabled, so the next
    /// modeset re-enables them rather than trusting the state the switch left behind.
    ///
    /// # Errors
    ///
    /// [`DrmError::Device`] when smithay cannot re-activate the device.
    pub fn activate(&mut self) -> Result<(), DrmError> {
        self.hardware
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
        self.hardware
            .ports
            .iter()
            .find(|port| port.connector.id == output)
            .and_then(|port| port.crtc)
    }

    /// Applies a plan to the hardware, rolling back on the first refusal.
    ///
    /// # Errors
    ///
    /// [`ApplyError`] when an operation is refused; see [`apply_plan`].
    pub fn apply(&mut self, plan: &LayoutPlan) -> Result<(), ApplyError> {
        apply_plan(&mut self.hardware, &mut self.applied, plan)
    }

    /// What each connector was last confirmed to be doing.
    #[must_use]
    pub fn applied(&self) -> &Applied {
        &self.applied
    }
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
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        let offered: Vec<(DisplayMode, Mode)> = info.modes().iter().map(|mode| (display_mode(mode), *mode)).collect();
        let id = u64::try_from(ports.len() + 1).unwrap_or(u64::MAX);
        let mut port = Port {
            connector: Connector {
                id: OutputId::new(id),
                name: name.clone(),
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
            crtc: None,
            modes: offered,
            surface: None,
        };
        if matches!(info.state(), connector::State::Connected) {
            match bring_up(device, &resources, &info, &taken) {
                Ok(up) => {
                    port.connector.connected = true;
                    port.connector.vrr = up.vrr;
                    port.connector.gamma_length = up.gamma_length;
                    port.crtc = Some(up.crtc);
                    port.surface = Some(up.surface);
                    taken.push(up.crtc);
                }
                Err(reason) => tracing::warn!(connector = %name, %reason, "[drm] the connector stays disconnected"),
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
