//! What `zwlr_output_management_v1` reports for each output, and how a configuration a client asked for is applied.
//!
//! The protocol handler runs inside the Wayland dispatch, which holds the [`State`](crate::State) and, once the loop
//! hands it in, the backend. [`State::output_heads`] is the snapshot a client reads: every mode the backend lists for
//! the output, the one in use, the layout position and whether the output is on. A configuration is not applied here:
//! [`State::apply_head_requests`] turns each head's request into a change to the output's profile and runs them
//! through [`State::apply_output_changes`], the engine a shell's `screen.apply` and the IPC's `output.apply` use, so
//! all three get the same checks, the same test before the first output is driven, and the same rollback.

use gnoblin_backend::Backend;
use gnoblin_output::{DisplayMode, ProfileChange};
use gnoblin_types::{OutputId, Scale};

use crate::State;

/// One mode a head can be driven in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeInfo {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Refresh rate in millihertz, as the protocol reports it.
    pub refresh_mhz: i32,
    /// Whether the protocol marks this mode as preferred. The backend reports its one mode as preferred.
    pub preferred: bool,
}

/// What the host reports for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadInfo {
    /// The stable identity the host uses for this output.
    pub id: OutputId,
    /// The connector name the backend knows this output by.
    pub name: String,
    /// The description the protocol reports. The backend sets it to the connector name.
    pub description: String,
    /// The manufacturer string. The backend reports `Gnoblin`.
    pub make: String,
    /// The model string. The backend reports the connector name.
    pub model: String,
    /// The serial number. The backend reports none, so this is empty.
    pub serial: String,
    /// Physical size in millimetres.
    pub physical_mm: (i32, i32),
    /// Every mode the head can be driven in. [`HeadInfo::current`] indexes into this list.
    pub modes: Vec<ModeInfo>,
    /// Index into `modes` of the mode in use, if any.
    pub current: Option<usize>,
    /// Whether the output is on. The backend reports every output as enabled.
    pub enabled: bool,
    /// Top-left corner in the layout. `(0, 0)` when the layout has no position for the output.
    pub position: (i32, i32),
    /// `wl_output` transform value.
    pub transform: i32,
    /// Scale factor reported to clients, read from the backend's current scale.
    pub scale: f64,
    /// Whether adaptive sync is on. The backend always reports `false`.
    pub adaptive_sync: bool,
}

/// A mode a client asked for, either one the head lists or a custom one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeRequest {
    /// Requested width in pixels.
    pub width: i32,
    /// Requested height in pixels.
    pub height: i32,
    /// Requested refresh rate in millihertz.
    pub refresh_mhz: i32,
}

/// What a configuration asks for one head. A `None` field keeps the head's current value.
///
/// A request is read against the head as it is now and laid over the output's profile, so a request that names what
/// the head already has changes nothing. A transform other than `0` and adaptive sync are refused.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadRequest {
    /// The head this request configures.
    pub head: OutputId,
    /// Whether the head should be on. A configuration must leave at least one head on.
    pub enabled: bool,
    /// The mode to drive the head in, or `None` to keep the current one. A mode the backend does not list is refused.
    pub mode: Option<ModeRequest>,
    /// The position to move the head to, or `None` to keep it.
    pub position: Option<(i32, i32)>,
    /// The `wl_output` transform to apply, or `None` to keep it. Any value but `0` is refused.
    pub transform: Option<i32>,
    /// The scale to apply, or `None` to keep it.
    pub scale: Option<f64>,
    /// Whether adaptive sync should be on, or `None` to keep it. `true` is refused.
    pub adaptive_sync: Option<bool>,
}

impl HeadRequest {
    /// The change to the output's profile this request makes against `head`, the head as it is now.
    ///
    /// A field that names what the head already has changes nothing, so a client that sends its whole configuration
    /// back does not turn every output's automatic placement into a stored one. A transform other than normal and
    /// adaptive sync are refused as the shell's `apply` refuses them: nothing draws a rotation and nothing drives
    /// variable refresh.
    ///
    /// # Errors
    ///
    /// Why the request cannot be made, naming the head.
    fn to_change(&self, head: &HeadInfo) -> Result<ProfileChange, String> {
        let mut change = ProfileChange::default();
        if self.enabled != head.enabled {
            change.enabled = Some(self.enabled);
        }
        if let Some(mode) = self.mode {
            let (Ok(width), Ok(height), Ok(refresh)) = (
                u32::try_from(mode.width),
                u32::try_from(mode.height),
                u32::try_from(mode.refresh_mhz),
            ) else {
                return Err(format!("{}: the requested mode is not a mode", head.name));
            };
            let requested = DisplayMode::new(width, height, refresh);
            let current = head.current.and_then(|index| head.modes.get(index));
            let is_current = current.is_some_and(|current| {
                current.width == mode.width && current.height == mode.height && current.refresh_mhz == mode.refresh_mhz
            });
            if !is_current {
                change.mode = Some(Some(requested));
            }
        }
        if let Some(position) = self.position
            && position != head.position
        {
            change.position = Some(Some(position));
        }
        if self.transform.is_some_and(|transform| transform != 0) {
            return Err(format!("{}: transforms are not supported", head.name));
        }
        if self.adaptive_sync == Some(true) {
            return Err(format!("{}: adaptive sync is not supported", head.name));
        }
        if let Some(scale) = self.scale {
            if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
                return Err(format!("{}: scale {scale} is out of range", head.name));
            }
            let target = Scale::from_percent((scale * 100.0).round() as u32);
            if (target.as_f64() - head.scale).abs() > f64::EPSILON {
                change.scale = Some(Some(target));
            }
        }
        Ok(change)
    }
}

impl State {
    /// The layout position of each output, for the heads the host publishes.
    pub(crate) fn output_positions(&self) -> Vec<(OutputId, (i32, i32))> {
        self.output_ids()
            .into_iter()
            .filter_map(|id| self.output_rect(id).map(|rect| (id, (rect.x, rect.y))))
            .collect()
    }

    /// The outputs as the protocol reports them: every mode the backend can drive each in, the one it is driven in
    /// now, its place in the layout and whether the desktop gives it one. A mirror is on; an output that is off is not.
    pub(crate) fn output_heads(&self, backend: &dyn Backend<State>) -> Vec<HeadInfo> {
        let positions = self.output_positions();
        backend
            .outputs()
            .iter()
            .map(|output| {
                let mut offered = backend.output_modes(output.id);
                // The mode in use is always one of the modes a head lists, whatever the backend's own list says.
                if !offered.contains(&output.mode) {
                    offered.insert(0, output.mode);
                }
                let modes: Vec<ModeInfo> = offered
                    .iter()
                    .enumerate()
                    .map(|(index, mode)| ModeInfo {
                        width: i32::try_from(mode.width).unwrap_or(i32::MAX),
                        height: i32::try_from(mode.height).unwrap_or(i32::MAX),
                        refresh_mhz: i32::try_from(mode.refresh_mhz).unwrap_or(i32::MAX),
                        // A backend lists the monitor's preferred mode first.
                        preferred: index == 0,
                    })
                    .collect();
                let position = positions
                    .iter()
                    .find(|(id, _)| *id == output.id)
                    .map_or((0, 0), |(_, position)| *position);
                HeadInfo {
                    id: output.id,
                    name: output.connector.clone(),
                    description: output.connector.clone(),
                    make: "Gnoblin".to_owned(),
                    model: output.connector.clone(),
                    serial: String::new(),
                    physical_mm: (0, 0),
                    current: offered.iter().position(|mode| *mode == output.mode),
                    modes,
                    enabled: self
                        .output_policy
                        .shadow(output.id)
                        .is_none_or(|shadow| shadow.source.is_some()),
                    position,
                    transform: 0,
                    scale: output.scale.as_f64(),
                    adaptive_sync: false,
                }
            })
            .collect()
    }

    /// Applies the heads a `zwlr_output_configuration_v1` asked for, or only checks them when `test_only` is set, through
    /// the same engine as a shell's `screen.apply` ([`State::apply_output_changes`]).
    ///
    /// `Ok` means the whole configuration was accepted and, when not a test, is in effect. `Err` means nothing was left
    /// changed: the engine put back whatever it had driven.
    pub(crate) fn apply_head_requests(
        &mut self,
        backend: &mut dyn Backend<State>,
        requests: &[HeadRequest],
        test_only: bool,
    ) -> Result<(), String> {
        if requests.iter().all(|request| !request.enabled) {
            return Err("a configuration must leave at least one output enabled".to_owned());
        }
        let heads = self.output_heads(backend);
        let mut changes = Vec::with_capacity(requests.len());
        for request in requests {
            let Some(head) = heads.iter().find(|head| head.id == request.head) else {
                return Err("the configuration names a head this host does not drive".to_owned());
            };
            changes.push((request.head, request.to_change(head)?));
        }
        self.apply_output_changes(backend, &changes, test_only)
    }
}

/// One output change in a configuration: the mode and scale to move it from and to. Rollback restores the mode
/// and the scale the head had.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OutputChange {
    /// The head being changed.
    pub(crate) id: OutputId,
    /// The mode the head is driven in after the change.
    pub(crate) mode: DisplayMode,
    /// The mode the head had before the change.
    pub(crate) before_mode: DisplayMode,
    /// The scale the head had before the change.
    pub(crate) before: Scale,
    /// The scale the head moves to.
    pub(crate) after: Scale,
}

/// A configuration that could not be applied. The output named is the first one refused; the
/// rest are listed so the message can say what was restored and what was left changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyFailure {
    /// The output whose change was refused.
    pub(crate) output: OutputId,
    /// Why the backend refused that change.
    pub(crate) reason: String,
    /// Earlier outputs that were rolled back to their previous scale.
    pub(crate) restored: Vec<OutputId>,
    /// Earlier outputs whose rollback was itself refused and so keep their new scale.
    pub(crate) stuck: Vec<OutputId>,
}

impl ApplyFailure {
    /// The message a client sees, naming the output by `name`.
    pub(crate) fn describe(&self, name: impl Fn(OutputId) -> String) -> String {
        let mut message = format!("{}: the backend refused the change: {}", name(self.output), self.reason);
        if !self.stuck.is_empty() {
            let names: Vec<String> = self.stuck.iter().map(|id| name(*id)).collect();
            message.push_str("; these outputs could not be restored and keep their new state: ");
            message.push_str(&names.join(", "));
        } else if !self.restored.is_empty() {
            message.push_str("; the earlier outputs were restored to their previous state");
        }
        message
    }
}

/// Applies `steps` in order. When one is refused, the earlier steps are rolled back in reverse
/// order, so the last change made is the first undone. A rollback that is refused is recorded
/// as stuck rather than hidden, because the client must be told which output kept its change.
pub(crate) fn apply_in_order(
    steps: &[OutputChange],
    mut configure: impl FnMut(OutputId, DisplayMode, Scale) -> Result<(), String>,
) -> Result<(), ApplyFailure> {
    for (index, step) in steps.iter().enumerate() {
        if let Err(reason) = configure(step.id, step.mode, step.after) {
            let mut restored = Vec::new();
            let mut stuck = Vec::new();
            for done in steps[..index].iter().rev() {
                match configure(done.id, done.before_mode, done.before) {
                    Ok(()) => restored.push(done.id),
                    Err(_) => stuck.push(done.id),
                }
            }
            return Err(ApplyFailure {
                output: step.id,
                reason,
                restored,
                stuck,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use gnoblin_output::DisplayMode;
    use gnoblin_types::{OutputId, Scale};

    use super::{ApplyFailure, OutputChange, apply_in_order};

    fn step(id: u64, from: u32, to: u32) -> OutputChange {
        OutputChange {
            id: OutputId::new(id),
            mode: DisplayMode::new(1920, 1080, 60_000),
            before_mode: DisplayMode::new(1920, 1080, 60_000),
            before: Scale::from_percent(from),
            after: Scale::from_percent(to),
        }
    }

    /// Runs `apply_in_order` and records every call as `(output, scale percent)`, so a test can
    /// check the forward changes and the rollback order.
    fn run(
        steps: &[OutputChange],
        refuse: impl Fn(OutputId, u32) -> bool,
    ) -> (Result<(), ApplyFailure>, Vec<(u64, u32)>) {
        let mut calls = Vec::new();
        let result = apply_in_order(steps, |id, _mode, scale| {
            let percent = (scale.as_f64() * 100.0).round() as u32;
            calls.push((id.get(), percent));
            if refuse(id, percent) {
                Err("the mode is not offered".to_owned())
            } else {
                Ok(())
            }
        });
        (result, calls)
    }

    #[test]
    fn every_change_is_applied_in_order_when_nothing_refuses() {
        let steps = [step(1, 100, 200), step(2, 100, 150)];
        let (result, calls) = run(&steps, |_, _| false);
        assert_eq!(result, Ok(()));
        assert_eq!(calls, vec![(1, 200), (2, 150)]);
    }

    #[test]
    fn a_refusal_on_the_second_output_names_it_and_restores_the_first() {
        let steps = [step(1, 100, 200), step(2, 100, 150)];
        // Output 2 refuses its new scale; output 1 must go back to 100%.
        let (result, calls) = run(&steps, |id, percent| id.get() == 2 && percent == 150);
        let failure = result.expect_err("the second output refused");
        assert_eq!(failure.output, OutputId::new(2));
        assert_eq!(failure.reason, "the mode is not offered");
        assert!(failure.stuck.is_empty(), "the restore of output 1 was accepted");
        assert_eq!(calls, vec![(1, 200), (2, 150), (1, 100)]);
    }

    #[test]
    fn rollback_undoes_the_changes_in_reverse_order() {
        let steps = [step(1, 100, 200), step(2, 100, 150), step(3, 100, 300)];
        let (result, calls) = run(&steps, |id, _| id.get() == 3);
        assert_eq!(result.expect_err("the third output refused").output, OutputId::new(3));
        // The refused change is the last one tried, so the restores run 2 then 1.
        assert_eq!(calls, vec![(1, 200), (2, 150), (3, 300), (2, 100), (1, 100)]);
    }

    #[test]
    fn a_restore_that_is_refused_is_reported_as_stuck() {
        let steps = [step(1, 100, 200), step(2, 100, 150), step(3, 100, 300)];
        // Output 3 refuses, and so does the restore of output 2: output 2 is left at 150%.
        let (result, _) = run(&steps, |id, percent| {
            (id.get() == 3) || (id.get() == 2 && percent == 100)
        });
        let failure = result.expect_err("output 3 refused");
        assert_eq!(failure.output, OutputId::new(3));
        assert_eq!(failure.stuck, vec![OutputId::new(2)]);
    }

    #[test]
    fn the_message_names_the_output_by_connector_and_says_what_was_restored() {
        let steps = [step(1, 100, 200), step(2, 100, 150)];
        let (result, _) = run(&steps, |id, _| id.get() == 2);
        let failure = result.expect_err("output 2 refused");
        let names = |id: OutputId| {
            if id.get() == 2 {
                "DP-2".to_owned()
            } else {
                "DP-1".to_owned()
            }
        };
        assert_eq!(
            failure.describe(names),
            "DP-2: the backend refused the change: the mode is not offered; \
             the earlier outputs were restored to their previous state"
        );
    }

    #[test]
    fn the_message_names_an_output_left_at_its_new_state() {
        let steps = [step(1, 100, 200), step(2, 100, 150), step(3, 100, 300)];
        let (result, _) = run(&steps, |id, percent| {
            (id.get() == 3) || (id.get() == 2 && percent == 100)
        });
        let failure = result.expect_err("output 3 refused");
        let names = |id: OutputId| format!("DP-{}", id.get());
        assert_eq!(
            failure.describe(names),
            "DP-3: the backend refused the change: the mode is not offered; \
             these outputs could not be restored and keep their new state: DP-2"
        );
    }
}
