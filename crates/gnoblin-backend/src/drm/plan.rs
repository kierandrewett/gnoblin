//! What the outputs should be doing, and the changes that get them there.
//!
//! This is the pure half of the DRM backend: it reads a [`Layout`] from `gnoblin-output` and
//! the kernel facts about the connectors, and answers two things — the descriptor each
//! connector is published as ([`Connector::descriptor`]) and the ordered list of kernel
//! operations that puts one output into a new state ([`ops`]). The device work itself is
//! `drm::gpu`, which does nothing but translate an [`Op`] into one ioctl.
//!
//! It exists because the alternative is arithmetic about modes, scales and rollbacks buried
//! in `unsafe`-adjacent device code where no test can reach it. Nothing here names a Smithay,
//! `drm` or udev type except the two the CRTC's gamma length comes from, and that one is a
//! `usize` in [`Connector`].
//!
//! # Order matters, so the order is fixed
//!
//! A mode set on a connector that is powered off does nothing, and a gamma table uploaded
//! after the CRTC is disabled is lost. [`ops`] therefore states one order for every change:
//!
//! 1. power on, when the output was off and is being lit;
//! 2. the mode, and then the orientation the plane presents it at;
//! 3. variable refresh, which is a connector property and a modeset on some drivers;
//! 4. the gamma table, so the first frame after a mode set is already tinted;
//! 5. power off, last, when the output is going dark.
//!
//! # Rollback
//!
//! [`Applied`] remembers the state each connector was last *confirmed* in. A proposal that
//! fails its test, or that nobody confirmed, is rolled back by asking [`Applied`] for the
//! operations that restore the recorded state — never by re-deriving it from the proposal,
//! which would restore the mode that failed.
//!
//! # Failure modes first
//!
//! 1. **A mode the connector does not accept is committed.** [`plan`] refuses a mode that is
//!    not in the connector's list ([`LayoutPlanError::UnsupportedMode`]) and a mode with no pixels
//!    ([`LayoutPlanError::DegenerateMode`]) before anything is sent: `a_mode_the_connector_does_not_
//!    drive_is_refused` and `a_degenerate_mode_is_refused`.
//! 2. **A gamma table is uploaded to a CRTC of another length.** The change carries a table
//!    built for that connector's length, so a mismatch can only come from a driver that lied
//!    about the length — and then the ioctl is still refused with both lengths named:
//!    `the_gamma_table_is_built_for_the_connectors_own_length`.
//! 3. **Rollback restores the wrong mode.** [`Applied::restore`] answers from the recorded
//!    state, so a failed proposal rolls back to the mode that was there, not the one that was
//!    refused: `a_rollback_restores_the_recorded_mode_and_not_the_proposal`.
//! 4. **An output that was off is rolled back into a lit mode.** A connector with no layout
//!    entry is [`Op::Power`] off, and a rollback of one that was off restores off:
//!    `an_output_that_was_off_is_rolled_back_into_the_dark`.
//! 5. **Variable refresh is asked of a panel that cannot.** The request is downgraded and the
//!    state says so, so the layout records what happened instead of a request that silently
//!    did nothing: `a_vrr_request_on_an_incapable_panel_is_downgraded`.
//! 6. **An output that was unplugged is written to.** Only connected connectors are planned:
//!    `a_disconnected_connector_is_not_planned`.
//! 7. **A hotplug during a modeset leaves a change for an output that is gone.** [`plan`]
//!    takes the connector list it is given, so a caller that re-plans after the hotplug never
//!    produces an operation for the removed connector:
//!    `a_replanned_hotplug_has_no_operation_for_the_removed_connector`.

use std::collections::BTreeMap;

use gnoblin_output::{DisplayMode, Edid, Layout, OutputDescriptor, VrrSupport};
use gnoblin_types::{Orientation, OutputId, Scale};

use crate::drm::gamma::{GammaError, GammaRamp};

/// Whether an output is lit.
///
/// It is a separate value from the mode because a connector keeps its mode while it is off:
/// turning a monitor off and on again is not a mode change, and a plan that modelled it as
/// one would round-trip the mode through the kernel for nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Power {
    /// The output is lit and scanning out.
    On,
    /// The output is dark; the CRTC is off and the connector's DPMS is off.
    Off,
}

/// What one output is doing.
///
/// It is the whole state of an output rather than a delta, because a delta applied twice would
/// leave the hardware depending on how many times it was delivered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutputState {
    /// The mode being driven.
    pub mode: DisplayMode,
    /// The scale the session composes at.
    pub scale: Scale,
    /// The orientation the plane presents the mode at.
    pub orientation: Orientation,
    /// Whether the connector is following the content's frame rate.
    pub vrr: bool,
    /// Whether the output is lit.
    pub power: Power,
    /// The colour temperature applied through the gamma table, `None` for the identity.
    pub kelvin: Option<u32>,
}

impl OutputState {
    /// A lit output at the mode, scale and orientation given, with no tint and no VRR.
    #[must_use]
    pub fn lit(mode: DisplayMode, scale: Scale, orientation: Orientation) -> Self {
        Self {
            mode,
            scale,
            orientation,
            vrr: false,
            power: Power::On,
            kelvin: None,
        }
    }

    /// A dark output, which keeps the mode it will return to.
    #[must_use]
    pub fn dark(mode: DisplayMode, scale: Scale, orientation: Orientation) -> Self {
        Self {
            power: Power::Off,
            ..Self::lit(mode, scale, orientation)
        }
    }
}

/// One thing the kernel is asked to do, in the order it must happen.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Op {
    /// Drive this mode. A mode set is the only operation that can fail a test on its own.
    Mode(DisplayMode),
    /// Present the mode at this orientation.
    Orientation(Orientation),
    /// Follow the content's frame rate, or stop following it.
    Vrr(bool),
    /// Apply this gamma table.
    Gamma(GammaRamp),
    /// Light the output or turn it off.
    Power(Power),
}

/// A connected connector, as the kernel and the monitor describe it.
///
/// The backend fills this from the DRM connector, its properties and its EDID; everything that
/// reads it is tested against values built by hand, which is why no field is a DRM type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Connector {
    /// The graph token the rest of the compositor keys its tables by.
    pub id: OutputId,
    /// The connector name a person sees, such as `"DP-1"`.
    pub name: String,
    /// Whether a monitor is plugged in and the kernel reports a usable connection.
    pub connected: bool,
    /// The monitor's identity, when its EDID parsed.
    pub edid: Option<Edid>,
    /// Every mode the connector accepts, the monitor's preferred one first.
    pub modes: Vec<DisplayMode>,
    /// The panel's physical extent in millimetres, zero where unknown.
    pub physical: PhysicalExtent,
    /// Whether the connector can follow the content's frame rate.
    pub vrr: VrrSupport,
    /// How many entries this connector's CRTC applies in its gamma table.
    pub gamma_length: usize,
}

/// A panel's physical extent in millimetres, zero on an axis it does not report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PhysicalExtent {
    /// Width in millimetres.
    pub width: u32,
    /// Height in millimetres.
    pub height: u32,
}

impl Connector {
    /// The descriptor the layout engine and the public output record see.
    ///
    /// Every field the monitor reported is carried over, and the two the session decides —
    /// the scales it is willing to compose at and whether the connector can vary its rate —
    /// are filled in here, because they are facts about this connector rather than about the
    /// layout.
    #[must_use]
    pub fn descriptor(&self) -> OutputDescriptor {
        let mut descriptor = match &self.edid {
            Some(edid) => OutputDescriptor::from_edid(self.id, self.name.clone(), edid),
            None => OutputDescriptor::new(self.id, self.name.clone(), self.modes.clone()),
        };
        // The mode list is the connector's, not the EDID's: a monitor whose base block
        // carries one timing still accepts the modes the kernel enumerated for it.
        descriptor.modes = self.modes.clone();
        descriptor.physical = gnoblin_output::PhysicalSize::new(self.physical.width, self.physical.height);
        descriptor.scales = self.suggested_scales();
        descriptor.vrr = self.vrr;
        descriptor
    }

    /// The scales the session offers for this connector.
    ///
    /// The identity scale is always offered, because a configuration that names no scale must
    /// always be valid. A panel dense enough to need more gets the integer scale whose logical
    /// size is nearest the 96 dpi the toolkit defaults assume, capped at 3 so a small 8K panel
    /// does not offer a scale no window can be laid out at.
    #[must_use]
    pub fn suggested_scales(&self) -> Vec<Scale> {
        let mut scales = vec![Scale::integral(1)];
        let Some(mode) = self.modes.first() else {
            return scales;
        };
        let Some(dpi) = self.dpi(*mode) else {
            return scales;
        };
        let wanted = (dpi / 96.0).round().clamp(1.0, 3.0) as u32;
        if wanted > 1 {
            scales.push(Scale::integral(wanted));
        }
        scales
    }

    /// The panel's dot pitch in dots per inch for `mode`, or [`None`] when it is unknown.
    ///
    /// The diagonal is used rather than one axis because a panel's axes are not square in
    /// general, and a monitor that reports only one axis would otherwise be measured against
    /// a zero.
    #[must_use]
    pub fn dpi(&self, mode: DisplayMode) -> Option<f64> {
        if self.physical.width == 0 || self.physical.height == 0 {
            return None;
        }
        let width_inches = f64::from(self.physical.width) / 25.4;
        let height_inches = f64::from(self.physical.height) / 25.4;
        let diagonal_pixels = f64::from(mode.width).hypot(f64::from(mode.height));
        let diagonal_inches = width_inches.hypot(height_inches);
        if diagonal_inches <= 0.0 {
            return None;
        }
        Some(diagonal_pixels / diagonal_inches)
    }

    /// The mode to drive when the layout does not name one.
    #[must_use]
    pub fn preferred_mode(&self) -> Option<DisplayMode> {
        self.modes.first().copied()
    }
}

/// What the gamma table should show, for the whole session.
///
/// Night light is one setting for every output, so the plan takes it once and every connector
/// builds its own table for its own CRTC length.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tint {
    /// The colour temperature, or [`None`] for the identity table.
    pub kelvin: Option<u32>,
}

/// One connector's part of a plan: the state it should be in, and the operations to get there.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The connector this change is for.
    pub connector: String,
    /// The state it should be in afterwards.
    pub state: OutputState,
    /// The operations to send, in order. Empty when the output is already in that state.
    pub ops: Vec<Op>,
}

impl Change {
    /// Whether this change asks the hardware for anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Why a plan was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LayoutPlanError {
    /// The connector does not accept this mode.
    UnsupportedMode {
        /// The connector.
        connector: String,
        /// The mode that was asked for.
        mode: DisplayMode,
    },
    /// The mode has no pixels on an axis.
    DegenerateMode {
        /// The connector.
        connector: String,
        /// The mode that was asked for.
        mode: DisplayMode,
    },
    /// The connector has no mode at all, so it cannot be lit.
    NoModes {
        /// The connector.
        connector: String,
    },
    /// The gamma table could not be built for this connector.
    Gamma {
        /// The connector.
        connector: String,
        /// Why.
        error: GammaError,
    },
}

impl std::fmt::Display for LayoutPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedMode { connector, mode } => write!(formatter, "{connector} does not accept {mode:?}"),
            Self::DegenerateMode { connector, mode } => {
                write!(formatter, "{connector} cannot drive {mode:?}, which covers no pixels")
            }
            Self::NoModes { connector } => write!(formatter, "{connector} reports no modes"),
            Self::Gamma { connector, error } => write!(formatter, "{connector} gamma: {error}"),
        }
    }
}

impl std::error::Error for LayoutPlanError {}

/// The plan for one layout: every connector's target state and its operations.
///
/// A connector that is not in the layout, and a connector with no monitor attached, are turned
/// off rather than left as they were: an output nobody configured must not keep showing stale
/// pixels from the previous layout.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LayoutPlan {
    /// One entry per connected connector that the layout names.
    pub changes: Vec<Change>,
    /// The connectors that should be dark, with the operations that get them there.
    pub disabled: Vec<Change>,
}

impl LayoutPlan {
    /// Every change, lit and dark, in connector order.
    pub fn all(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().chain(self.disabled.iter())
    }

    /// The number of outputs this plan lights.
    #[must_use]
    pub fn lit(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| change.state.power == Power::On)
            .count()
    }
}

/// Builds the plan that puts the hardware into `layout` from `current`.
///
/// `current` is what [`Applied`] recorded, so a console that is already correct produces
/// changes with no operations and the backend sends nothing.
///
/// # Errors
///
/// [`LayoutPlanError`] when a mode is not accepted, has no pixels, or when the gamma table cannot be
/// built for the connector's CRTC.
pub fn plan(
    connectors: &[Connector],
    layout: &Layout,
    current: &Applied,
    tint: Tint,
) -> Result<LayoutPlan, LayoutPlanError> {
    let mut changes = Vec::new();
    let mut disabled = Vec::new();
    for connector in connectors.iter().filter(|connector| connector.connected) {
        let monitor = layout.monitor_of(connector.id);
        let target = match monitor {
            Some(monitor) => {
                let mode = monitor.mode;
                if mode.width == 0 || mode.height == 0 {
                    return Err(LayoutPlanError::DegenerateMode {
                        connector: connector.name.clone(),
                        mode,
                    });
                }
                if !connector.modes.is_empty() && !connector.modes.contains(&mode) {
                    return Err(LayoutPlanError::UnsupportedMode {
                        connector: connector.name.clone(),
                        mode,
                    });
                }
                OutputState {
                    mode,
                    scale: monitor.scale,
                    orientation: monitor.orientation,
                    // A panel without variable refresh is not an error: the layout records
                    // the downgrade, and the state says what the hardware will do.
                    vrr: monitor.vrr && connector.vrr == VrrSupport::Supported,
                    power: Power::On,
                    kelvin: tint.kelvin,
                }
            }
            None => {
                let previous = current.state(&connector.name).copied();
                let mode = previous
                    .map(|state| state.mode)
                    .or_else(|| connector.preferred_mode())
                    .ok_or_else(|| LayoutPlanError::NoModes {
                        connector: connector.name.clone(),
                    })?;
                let scale = previous.map_or_else(|| Scale::integral(1), |state| state.scale);
                let orientation = previous.map_or(Orientation::Normal, |state| state.orientation);
                OutputState {
                    power: Power::Off,
                    kelvin: None,
                    ..OutputState::lit(mode, scale, orientation)
                }
            }
        };
        let ops = ops(connector, current.state(&connector.name), &target)?;
        let change = Change {
            connector: connector.name.clone(),
            state: target,
            ops,
        };
        if target.power == Power::On {
            changes.push(change);
        } else {
            disabled.push(change);
        }
    }
    Ok(LayoutPlan { changes, disabled })
}

/// The operations that take one connector from `current` to `target`.
///
/// # Errors
///
/// [`LayoutPlanError`] when the gamma table cannot be built for the connector's CRTC, which is the
/// one thing the pure layer can still refuse after a state is decided.
pub fn ops(
    connector: &Connector,
    current: Option<&OutputState>,
    target: &OutputState,
) -> Result<Vec<Op>, LayoutPlanError> {
    let mut ops = Vec::new();
    let was = current.copied();
    let lighting = target.power == Power::On && was.is_none_or(|state| state.power == Power::Off);
    if lighting {
        ops.push(Op::Power(Power::On));
    }
    if target.power == Power::On {
        let mode_changed = was.is_none_or(|state| state.mode != target.mode || state.power == Power::Off);
        if mode_changed {
            ops.push(Op::Mode(target.mode));
        }
        if was.is_none_or(|state| state.orientation != target.orientation) {
            ops.push(Op::Orientation(target.orientation));
        }
        if was.is_none_or(|state| state.vrr != target.vrr) {
            ops.push(Op::Vrr(target.vrr));
        }
        if was.is_none_or(|state| state.kelvin != target.kelvin) {
            let ramp = match target.kelvin {
                Some(kelvin) => GammaRamp::for_temperature(kelvin, connector.gamma_length),
                None => GammaRamp::identity(connector.gamma_length),
            };
            ops.push(Op::Gamma(ramp));
        }
    } else if was.is_none_or(|state| state.power == Power::On) {
        ops.push(Op::Power(Power::Off));
    }
    Ok(ops)
}

/// What each connector was last confirmed to be doing, for rollback.
///
/// The backend records a state only after the hardware accepted it, so a proposal that failed
/// halfway can never be the thing a rollback restores.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Applied {
    states: BTreeMap<String, OutputState>,
}

impl Applied {
    /// An empty record: nothing is known to be applied.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The state a connector was last confirmed in.
    #[must_use]
    pub fn state(&self, connector: &str) -> Option<&OutputState> {
        self.states.get(connector)
    }

    /// Every connector the record knows, in name order.
    pub fn connectors(&self) -> impl Iterator<Item = &str> {
        self.states.keys().map(String::as_str)
    }

    /// Records that a change was accepted by the hardware.
    pub fn record(&mut self, change: &Change) {
        self.states.insert(change.connector.clone(), change.state);
    }

    /// Records every change of a plan that the hardware accepted.
    pub fn record_plan(&mut self, plan: &LayoutPlan) {
        for change in plan.all() {
            self.record(change);
        }
    }

    /// Forgets a connector, which is what an unplug means.
    pub fn forget(&mut self, connector: &str) {
        self.states.remove(connector);
    }

    /// The operations that restore a connector to what was last confirmed.
    ///
    /// [`None`] when the record has nothing for the connector, in which case there is no
    /// previous state to restore and the caller leaves the hardware alone.
    ///
    /// # Errors
    ///
    /// [`LayoutPlanError`] when the gamma table cannot be built, which a restore of a state that was
    /// applied before cannot normally reach.
    pub fn restore(&self, connector: &Connector, after: &OutputState) -> Result<Option<Vec<Op>>, LayoutPlanError> {
        let Some(before) = self.state(&connector.name) else {
            return Ok(None);
        };
        ops(connector, Some(after), before).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_output::{LogicalMonitor, OutputConfig, Placement};
    use gnoblin_types::{Rect, Size};

    const FULL_HD: DisplayMode = DisplayMode::new(1920, 1080, 60_000);
    const FOUR_K: DisplayMode = DisplayMode::new(3840, 2160, 60_000);

    fn connector(name: &str, id: u64, modes: &[DisplayMode]) -> Connector {
        Connector {
            id: OutputId::new(id),
            name: name.to_owned(),
            connected: true,
            edid: None,
            modes: modes.to_vec(),
            physical: PhysicalExtent::default(),
            vrr: VrrSupport::Unsupported,
            gamma_length: 256,
        }
    }

    fn layout_of(monitors: Vec<LogicalMonitor>) -> Layout {
        Layout {
            primary: monitors.first().and_then(|monitor| monitor.members.first().copied()),
            monitors,
        }
    }

    fn monitor(name: &str, id: u64, mode: DisplayMode, x: i32, vrr: bool) -> LogicalMonitor {
        LogicalMonitor {
            monitor: name.to_owned(),
            rect: Rect::new(x, 0, mode.width, mode.height),
            scale: Scale::integral(1),
            orientation: Orientation::Normal,
            vrr,
            mode,
            members: vec![OutputId::new(id)],
        }
    }

    #[test]
    fn a_mode_the_connector_does_not_drive_is_refused() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FOUR_K, 0, false)]);
        let error = plan(&connectors, &layout, &Applied::new(), Tint::default()).expect_err("refused");
        assert!(matches!(error, LayoutPlanError::UnsupportedMode { .. }), "{error}");
    }

    #[test]
    fn a_degenerate_mode_is_refused() {
        let connectors = [connector("DP-1", 1, &[DisplayMode::new(0, 1080, 60_000)])];
        let layout = layout_of(vec![monitor("DP-1", 1, DisplayMode::new(0, 1080, 60_000), 0, false)]);
        let error = plan(&connectors, &layout, &Applied::new(), Tint::default()).expect_err("refused");
        assert!(matches!(error, LayoutPlanError::DegenerateMode { .. }), "{error}");
    }

    #[test]
    fn a_disconnected_connector_is_not_planned() {
        let mut unplugged = connector("DP-2", 2, &[FULL_HD]);
        unplugged.connected = false;
        let connectors = [connector("DP-1", 1, &[FULL_HD]), unplugged];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let plan = plan(&connectors, &layout, &Applied::new(), Tint::default()).expect("planned");
        assert_eq!(plan.all().count(), 1);
        assert_eq!(plan.changes[0].connector, "DP-1");
    }

    #[test]
    fn a_vrr_request_on_an_incapable_panel_is_downgraded() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, true)]);
        let plan = plan(&connectors, &layout, &Applied::new(), Tint::default()).expect("planned");
        assert!(
            !plan.changes[0].state.vrr,
            "the state records what the hardware will do"
        );
        assert!(!plan.changes[0].ops.contains(&Op::Vrr(true)));
    }

    #[test]
    fn a_capable_panel_gets_the_vrr_it_asked_for() {
        let mut capable = connector("DP-1", 1, &[FULL_HD]);
        capable.vrr = VrrSupport::Supported;
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, true)]);
        let plan = plan(&[capable], &layout, &Applied::new(), Tint::default()).expect("planned");
        assert!(plan.changes[0].state.vrr);
        assert!(plan.changes[0].ops.contains(&Op::Vrr(true)));
    }

    #[test]
    fn an_output_that_is_already_correct_gets_no_operation() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let mut applied = Applied::new();
        let first = plan(&connectors, &layout, &applied, Tint::default()).expect("planned");
        assert!(!first.changes[0].is_empty());
        applied.record_plan(&first);
        let second = plan(&connectors, &layout, &applied, Tint::default()).expect("planned");
        assert!(second.changes[0].is_empty(), "{:?}", second.changes[0].ops);
    }

    #[test]
    fn a_mode_is_set_before_the_gamma_table_and_turning_off_is_last() {
        let connectors = [connector("DP-1", 1, &[FULL_HD, FOUR_K])];
        let layout = layout_of(vec![monitor("DP-1", 1, FOUR_K, 0, false)]);
        let mut applied = Applied::new();
        applied.states.insert(
            "DP-1".to_owned(),
            OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal),
        );
        let proposal = plan(&connectors, &layout, &applied, Tint { kelvin: Some(3000) }).expect("planned");
        let ops = &proposal.changes[0].ops;
        let mode_at = ops.iter().position(|op| *op == Op::Mode(FOUR_K)).expect("a mode set");
        let gamma_at = ops
            .iter()
            .position(|op| matches!(op, Op::Gamma(_)))
            .expect("a gamma set");
        assert!(mode_at < gamma_at, "{ops:?}");

        let dark = layout_of(Vec::new());
        let off = plan(&connectors, &dark, &applied, Tint::default()).expect("planned");
        assert_eq!(off.disabled[0].ops, vec![Op::Power(Power::Off)]);
    }

    #[test]
    fn turning_an_output_on_lights_it_before_setting_its_mode() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let mut applied = Applied::new();
        applied.states.insert(
            "DP-1".to_owned(),
            OutputState::dark(FULL_HD, Scale::integral(1), Orientation::Normal),
        );
        let plan = plan(&connectors, &layout, &applied, Tint::default()).expect("planned");
        assert_eq!(
            plan.changes[0].ops.first(),
            Some(&Op::Power(Power::On)),
            "a mode set on a dark connector does nothing"
        );
    }

    #[test]
    fn a_rollback_restores_the_recorded_mode_and_not_the_proposal() {
        let connectors = [connector("DP-1", 1, &[FULL_HD, FOUR_K])];
        let mut applied = Applied::new();
        let previous = OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal);
        applied.states.insert("DP-1".to_owned(), previous);
        let proposed = OutputState::lit(FOUR_K, Scale::integral(1), Orientation::Normal);
        let restore = applied
            .restore(&connectors[0], &proposed)
            .expect("no gamma failure")
            .expect("a recorded state");
        assert_eq!(restore, vec![Op::Mode(FULL_HD)]);
        assert!(!restore.contains(&Op::Mode(FOUR_K)));
    }

    #[test]
    fn an_output_that_was_off_is_rolled_back_into_the_dark() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let mut applied = Applied::new();
        applied.states.insert(
            "DP-1".to_owned(),
            OutputState::dark(FULL_HD, Scale::integral(1), Orientation::Normal),
        );
        let lit = OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal);
        let restore = applied
            .restore(&connectors[0], &lit)
            .expect("no gamma failure")
            .expect("a recorded state");
        assert_eq!(restore, vec![Op::Power(Power::Off)]);
    }

    #[test]
    fn a_connector_with_no_record_has_nothing_to_restore() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let lit = OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal);
        assert_eq!(
            Applied::new().restore(&connectors[0], &lit),
            Ok(None),
            "an output that was never applied has no previous state"
        );
    }

    #[test]
    fn a_replanned_hotplug_has_no_operation_for_the_removed_connector() {
        let both = [connector("DP-1", 1, &[FULL_HD]), connector("DP-2", 2, &[FULL_HD])];
        let layout = layout_of(vec![
            monitor("DP-1", 1, FULL_HD, 0, false),
            monitor("DP-2", 2, FULL_HD, 1920, false),
        ]);
        let mut applied = Applied::new();
        let first = plan(&both, &layout, &applied, Tint::default()).expect("planned");
        applied.record_plan(&first);
        // The monitor on DP-2 is unplugged between the two plans.
        let remaining = [both[0].clone()];
        let replanned = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let second = plan(&remaining, &replanned, &applied, Tint::default()).expect("planned");
        assert_eq!(second.all().count(), 1);
        assert!(second.all().all(|change| change.connector != "DP-2"));
        applied.forget("DP-2");
        assert_eq!(applied.connectors().count(), 1);
    }

    #[test]
    fn the_gamma_table_is_built_for_the_connectors_own_length() {
        let mut panel = connector("eDP-1", 1, &[FULL_HD]);
        panel.gamma_length = 1024;
        let ops = ops(
            &panel,
            None,
            &OutputState::lit(FULL_HD, Scale::integral(1), Orientation::Normal),
        )
        .expect("no gamma failure");
        let ramp = ops
            .iter()
            .find_map(|op| match op {
                Op::Gamma(ramp) => Some(ramp.clone()),
                _ => None,
            })
            .expect("a gamma table");
        assert_eq!(ramp.length(), 1024);
        assert_eq!(ramp.matches(1024), Ok(()));
        assert!(ramp.matches(256).is_err());
    }

    #[test]
    fn a_tint_of_the_same_temperature_is_not_reapplied() {
        let connectors = [connector("DP-1", 1, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let tint = Tint { kelvin: Some(4000) };
        let mut applied = Applied::new();
        let first = plan(&connectors, &layout, &applied, tint).expect("planned");
        applied.record_plan(&first);
        let second = plan(&connectors, &layout, &applied, tint).expect("planned");
        assert!(second.changes[0].is_empty());
        let changed = plan(&connectors, &layout, &applied, Tint { kelvin: Some(3000) }).expect("planned");
        assert!(changed.changes[0].ops.iter().any(|op| matches!(op, Op::Gamma(_))));
    }

    #[test]
    fn a_dense_panel_suggests_a_scale_and_an_unknown_panel_only_the_identity() {
        let mut dense = connector("eDP-1", 1, &[FOUR_K]);
        dense.physical = PhysicalExtent {
            width: 344,
            height: 194,
        };
        // A 15.6 inch 4K panel is about 282 dpi, which is close to three identity pixels per
        // logical one.
        assert_eq!(dense.suggested_scales(), vec![Scale::integral(1), Scale::integral(3)]);

        let unknown = connector("DP-1", 1, &[FULL_HD]);
        assert_eq!(unknown.suggested_scales(), vec![Scale::integral(1)]);
        assert_eq!(unknown.dpi(FULL_HD), None);
    }

    #[test]
    fn a_descriptor_carries_everything_the_connector_reported() {
        let mut panel = connector("DP-1", 1, &[FULL_HD, FOUR_K]);
        panel.physical = PhysicalExtent {
            width: 600,
            height: 340,
        };
        panel.vrr = VrrSupport::Supported;
        let descriptor = panel.descriptor();
        assert_eq!(descriptor.connector, "DP-1");
        assert_eq!(descriptor.modes, vec![FULL_HD, FOUR_K]);
        assert_eq!(descriptor.preferred_mode(), Some(FULL_HD));
        assert_eq!(descriptor.vrr, VrrSupport::Supported);
        assert_eq!(descriptor.physical, gnoblin_output::PhysicalSize::new(600, 340));
        assert!(descriptor.scales.contains(&Scale::integral(1)));
    }

    #[test]
    fn a_plan_lights_only_the_outputs_the_layout_enables() {
        let connectors = [connector("DP-1", 1, &[FULL_HD]), connector("DP-2", 2, &[FULL_HD])];
        let layout = layout_of(vec![monitor("DP-1", 1, FULL_HD, 0, false)]);
        let plan = plan(&connectors, &layout, &Applied::new(), Tint::default()).expect("planned");
        assert_eq!(plan.lit(), 1);
        assert_eq!(plan.disabled.len(), 1);
        assert_eq!(plan.disabled[0].connector, "DP-2");
        assert_eq!(plan.disabled[0].state.power, Power::Off);
    }

    /// A configuration this module never reads, kept so the layout type it produces is the
    /// one `gnoblin-output` publishes rather than a local imitation.
    #[test]
    fn the_configuration_vocabulary_is_the_layout_crates_own() {
        let config = OutputConfig::enabled("DP-1");
        assert_eq!(config.placement, Some(Placement::Origin));
        assert_eq!(
            LogicalMonitor::derived_size(&monitor("DP-1", 1, FULL_HD, 0, false)),
            Size::new(1920, 1080)
        );
    }
}
