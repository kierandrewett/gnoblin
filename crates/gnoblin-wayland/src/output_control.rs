//! Per-output gamma and power control: the host seam, and the bookkeeping the two wlr
//! protocols share.
//!
//! The compositor's protocol handlers run inside the Wayland dispatch, which holds the
//! [`State`](crate::State) but not the backend. So a request that must reach the
//! hardware is queued here and applied by [`State::apply_output_controls`] once the
//! host, which owns the backend, hands in an [`OutputControl`]. A reply that depends on
//! the hardware (a `failed` event) is sent from that same call, so a client always hears
//! the outcome of its own request.
//!
//! ## Holders
//!
//! One client holds gamma control of an output, and one client holds its power
//! object. A second request for an output that is already held gets `failed` at once,
//! as the protocol requires, and the first holder keeps the output.
//!
//! ## What the host reports
//!
//! [`OutputControls`] says what the backend can honour. The globals are created when
//! the session starts and removed at once when the backend reports no support, so a
//! client never binds a control the compositor cannot apply.

use std::collections::HashMap;

use gnoblin_backend::{Backend, BackendError, GammaRamp, OutputControls};
use gnoblin_types::OutputId;
use smithay::reexports::wayland_server::backend::GlobalId;
use wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1;
use wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::{Mode, ZwlrOutputPowerV1};

use crate::State;

use crate::log;

/// The host side of per-output controls.
///
/// The composition root implements it over the backend; [`NoOutputControl`] is the answer
/// for a host with no backend controls.
pub trait OutputControl {
    /// What this host can control.
    fn controls(&self) -> OutputControls;

    /// The gamma table size of `output`, if it has one.
    fn gamma_size(&self, output: OutputId) -> Option<u32>;

    /// Loads a gamma ramp onto `output`.
    fn set_gamma(&mut self, output: OutputId, ramp: GammaRamp<'_>) -> Result<(), BackendError>;

    /// Puts `output`'s gamma back to the identity ramp.
    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError>;

    /// Switches `output` on or off.
    fn set_power(&mut self, output: OutputId, on: bool) -> Result<(), BackendError>;
}

/// A host that controls no output: every request is refused as unsupported.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOutputControl;

impl OutputControl for NoOutputControl {
    fn controls(&self) -> OutputControls {
        OutputControls::default()
    }

    fn gamma_size(&self, _output: OutputId) -> Option<u32> {
        None
    }

    fn set_gamma(&mut self, output: OutputId, _ramp: GammaRamp<'_>) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }

    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }

    fn set_power(&mut self, output: OutputId, _on: bool) -> Result<(), BackendError> {
        Err(BackendError::Unsupported(output))
    }
}

/// The client holding an output's gamma control.
pub(crate) struct GammaHolder {
    /// The `zwlr_gamma_control_v1` object of the holder.
    pub(crate) object: ZwlrGammaControlV1,
    /// Whether the holder asked for a ramp, so releasing it must restore the original.
    requested: bool,
}

/// A request the protocol handlers queued for the host.
enum PendingOp {
    /// Load three tables, one per channel.
    SetGamma { output: OutputId, tables: [Vec<u16>; 3] },
    /// Restore the original gamma of an output.
    RestoreGamma { output: OutputId },
    /// Switch an output's power.
    SetPower { output: OutputId, on: bool },
}

/// The gamma and power state of every output, and the requests waiting for the host.
#[derive(Default)]
pub(crate) struct OutputControlState {
    /// What the host reported at its last application.
    controls: OutputControls,
    /// The gamma table size of each output the host reported one for.
    gamma_sizes: HashMap<OutputId, u32>,
    /// The power state each output was last switched to. An output not listed is on.
    power: HashMap<OutputId, bool>,
    /// The client holding each output's gamma control.
    gamma: HashMap<OutputId, GammaHolder>,
    /// The client holding each output's power object.
    power_holders: HashMap<OutputId, ZwlrOutputPowerV1>,
    /// Requests waiting for the next application.
    pending: Vec<PendingOp>,
    /// The `zwlr_gamma_control_manager_v1` global, while it is advertised.
    pub(crate) gamma_global: Option<GlobalId>,
    /// The `zwlr_output_power_manager_v1` global, while it is advertised.
    pub(crate) power_global: Option<GlobalId>,
}

impl OutputControlState {
    /// The state of a new session, holding the globals it created.
    pub(crate) fn new(gamma_global: Option<GlobalId>, power_global: Option<GlobalId>) -> Self {
        Self {
            gamma_global,
            power_global,
            ..Self::default()
        }
    }

    /// The gamma table size of `output`, as the host last reported it.
    pub(crate) fn gamma_size(&self, output: OutputId) -> Option<u32> {
        self.gamma_sizes.get(&output).copied()
    }

    /// The power state `output` was last switched to.
    pub(crate) fn power(&self, output: OutputId) -> bool {
        self.power.get(&output).copied().unwrap_or(true)
    }

    /// Whether the host can switch an output's power at all.
    pub(crate) fn power_supported(&self) -> bool {
        self.controls.power
    }

    /// Whether the client holding gamma control of `output` is `object`.
    pub(crate) fn gamma_holder_is(&self, output: OutputId, object: &ZwlrGammaControlV1) -> bool {
        self.gamma.get(&output).is_some_and(|holder| holder.object == *object)
    }

    /// Whether `object` is the power object holding `output`.
    pub(crate) fn power_holder_is(&self, output: OutputId, object: &ZwlrOutputPowerV1) -> bool {
        self.power_holders.get(&output).is_some_and(|holder| holder == object)
    }

    /// Whether `output` has no gamma holder.
    pub(crate) fn gamma_free(&self, output: OutputId) -> bool {
        !self.gamma.contains_key(&output)
    }

    /// Whether `output` has no power holder.
    pub(crate) fn power_free(&self, output: OutputId) -> bool {
        !self.power_holders.contains_key(&output)
    }

    /// Records `object` as the gamma holder of `output`.
    pub(crate) fn hold_gamma(&mut self, output: OutputId, object: ZwlrGammaControlV1) {
        self.gamma.insert(
            output,
            GammaHolder {
                object,
                requested: false,
            },
        );
    }

    /// Records `object` as the power holder of `output`.
    pub(crate) fn hold_power(&mut self, output: OutputId, object: ZwlrOutputPowerV1) {
        self.power_holders.insert(output, object);
    }

    /// Queues a gamma ramp for `output` from the holder `object`.
    pub(crate) fn queue_gamma(&mut self, output: OutputId, tables: [Vec<u16>; 3]) {
        if let Some(holder) = self.gamma.get_mut(&output) {
            holder.requested = true;
        }
        self.pending.push(PendingOp::SetGamma { output, tables });
    }

    /// Queues a power change for `output`.
    pub(crate) fn queue_power(&mut self, output: OutputId, on: bool) {
        self.pending.push(PendingOp::SetPower { output, on });
    }

    /// Removes the gamma holder of `object`, queueing a restore if it asked for a ramp.
    pub(crate) fn release_gamma(&mut self, object: &ZwlrGammaControlV1) {
        let Some(output) = self
            .gamma
            .iter()
            .find(|(_, holder)| holder.object == *object)
            .map(|(output, _)| *output)
        else {
            return;
        };
        if let Some(holder) = self.gamma.remove(&output)
            && holder.requested
        {
            self.pending.push(PendingOp::RestoreGamma { output });
        }
    }

    /// Removes the power holder of `object`. The power state it set stays.
    pub(crate) fn release_power(&mut self, object: &ZwlrOutputPowerV1) {
        self.power_holders.retain(|_, holder| holder != object);
    }

    /// Forgets everything about an output the session no longer has.
    pub(crate) fn remove_output(&mut self, output: OutputId) {
        self.gamma_sizes.remove(&output);
        self.power.remove(&output);
        self.pending.retain(|op| match op {
            PendingOp::SetGamma { output: queued, .. }
            | PendingOp::RestoreGamma { output: queued }
            | PendingOp::SetPower { output: queued, .. } => *queued != output,
        });
    }
}

impl State {
    /// Learns what the host can control, and removes the globals it cannot honour.
    ///
    /// Called once at startup, before any client connects, so an unsupported control
    /// is never advertised.
    pub(crate) fn start_output_controls(&mut self, control: &mut dyn OutputControl) {
        self.refresh_output_controls(control);
        if !self.output_controls.controls.gamma
            && let Some(global) = self.output_controls.gamma_global.take()
        {
            self.display_handle.remove_global::<State>(global);
        }
        if !self.output_controls.controls.power
            && let Some(global) = self.output_controls.power_global.take()
        {
            self.display_handle.remove_global::<State>(global);
        }
    }

    /// Applies the requests the protocols queued, in the order they arrived.
    ///
    /// A backend refusal is reported to the client that asked, as `failed`; the
    /// session keeps running, because a display request the hardware refuses is the
    /// client's to handle.
    pub(crate) fn apply_output_controls(&mut self, control: &mut dyn OutputControl) {
        if self.output_controls.pending.is_empty() {
            return;
        }
        for op in std::mem::take(&mut self.output_controls.pending) {
            match op {
                PendingOp::SetGamma { output, tables } => {
                    let ramp = GammaRamp {
                        red: &tables[0],
                        green: &tables[1],
                        blue: &tables[2],
                    };
                    if let Err(error) = control.set_gamma(output, ramp) {
                        log::warn!(%error, "[gamma] the backend refused the ramp");
                        self.fail_gamma(output);
                    }
                }
                PendingOp::RestoreGamma { output } => {
                    if let Err(error) = control.restore_gamma(output) {
                        log::warn!(%error, "[gamma] the backend could not restore the ramp");
                    }
                }
                PendingOp::SetPower { output, on } => match control.set_power(output, on) {
                    Ok(()) => self.power_changed(output, on),
                    Err(error) => {
                        log::warn!(%error, "[power] the backend refused the power change");
                        if let Some(holder) = self.output_controls.power_holders.get(&output) {
                            holder.failed();
                        }
                    }
                },
            }
        }
    }

    /// Takes the host's table sizes and power state at face value for the next request.
    pub(crate) fn refresh_output_controls(&mut self, control: &dyn OutputControl) {
        self.output_controls.controls = control.controls();
        self.output_controls.gamma_sizes = self
            .outputs
            .iter()
            .filter_map(|entry| control.gamma_size(entry.backend).map(|size| (entry.backend, size)))
            .collect();
    }

    /// Tells the gamma holder of `output` that its ramp was refused.
    fn fail_gamma(&mut self, output: OutputId) {
        if let Some(holder) = self.output_controls.gamma.get(&output) {
            holder.object.failed();
        }
    }

    /// Records a power change the backend applied, and tells the holder when it moved.
    fn power_changed(&mut self, output: OutputId, on: bool) {
        let previous = self.output_controls.power(output);
        self.output_controls.power.insert(output, on);
        if previous != on
            && let Some(holder) = self.output_controls.power_holders.get(&output)
        {
            holder.mode(if on { Mode::On } else { Mode::Off });
        }
    }

    /// An output the session removed: its holders are told `failed` and let go.
    pub(crate) fn output_controls_removed(&mut self, output: OutputId) {
        if let Some(holder) = self.output_controls.gamma.remove(&output) {
            holder.object.failed();
        }
        if let Some(holder) = self.output_controls.power_holders.remove(&output) {
            holder.failed();
        }
        self.output_controls.remove_output(output);
    }
}

/// The backend as the host's [`OutputControl`]: each control is the backend's own method.
pub(crate) struct BackendControl<'a>(pub(crate) &'a mut dyn Backend<State>);

impl OutputControl for BackendControl<'_> {
    fn controls(&self) -> OutputControls {
        self.0.output_controls()
    }

    fn gamma_size(&self, output: OutputId) -> Option<u32> {
        self.0.gamma_size(output)
    }

    fn set_gamma(&mut self, output: OutputId, ramp: GammaRamp<'_>) -> Result<(), BackendError> {
        self.0.set_gamma(output, ramp)
    }

    fn restore_gamma(&mut self, output: OutputId) -> Result<(), BackendError> {
        self.0.restore_gamma(output)
    }

    fn set_power(&mut self, output: OutputId, on: bool) -> Result<(), BackendError> {
        self.0.set_power(output, on)
    }
}
