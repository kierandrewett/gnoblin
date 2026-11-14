//! Output policy: what a shell asked of the outputs (`screen.apply`), the profiles that keep it across a hotplug, and
//! the outputs the desktop gives no place of their own (a mirror, or one that is off).
//!
//! # The model
//!
//! The desktop is one global coordinate space. Every output that has a place in it (a *real* output) owns a rectangle
//! there, and a window straddling two of them is one window drawn in two pieces, so the extended desktop already is
//! the span: there is no separate span renderer, and a pointer that leaves one rectangle enters the next.
//!
//! An output with no place is a *shadow*. It keeps its screen node (a shell can still find it and change it back) but
//! is withdrawn from the scene, the pointer's reach and `wl_output`. A shadow either mirrors a real output (it shows
//! that output's finished picture, scaled to fit its own panel and letterboxed when the aspect differs) or is off (it
//! shows black). The mirror is a copy of pixels, not a second layout position, so the pair is one monitor for the
//! pointer and for windows.
//!
//! # Applying
//!
//! `apply` is transactional. The request is read, laid over the output's profile, and the whole candidate layout is
//! resolved first, so an overlap, a gap or a mirror of a missing output is refused by name before hardware is
//! touched. Only then are the modes and scales driven, in order; if the backend refuses one, the earlier ones are
//! put back and the last good arrangement, which was never replaced, stays. Only a fully driven candidate becomes the
//! profile.

use std::collections::{BTreeMap, BTreeSet};

use gnoblin_backend::{Backend, Output};
use gnoblin_output::{
    DisplayMode, Layout, LayoutConfig, LayoutError, OutputDescriptor, OutputProfile, OutputSighting, ProfileChange,
    ProfileStore, compute,
};
use gnoblin_render::transaction::TreeCall;
use gnoblin_render_frame::frame::{Layer, OutputFrame, Overlay};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{Orientation, OutputId, Rect, Size};

use crate::output_config::{OutputChange, apply_in_order};
use crate::{State, log};

/// An output the desktop gives no place of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shadow {
    /// The output.
    pub(crate) output: OutputId,
    /// The real output it shows a copy of, or `None` when it is off.
    pub(crate) source: Option<OutputId>,
    /// The output's own logical size, which its picture is fitted into.
    pub(crate) logical: Size,
    /// The mode the output is driven in.
    pub(crate) mode: DisplayMode,
    /// The scale the output composes at.
    pub(crate) scale: gnoblin_types::Scale,
}

/// An `apply` call waiting for the loop, which holds the backend.
#[derive(Debug)]
struct PendingApply {
    token: u64,
    output: Option<OutputId>,
    text: String,
}

/// What the session remembers about outputs between layouts.
#[derive(Default)]
pub(crate) struct OutputPolicy {
    /// The profiles, kept for hardware that is not connected as well as for hardware that is.
    pub(crate) profiles: ProfileStore,
    /// Outputs with no place of their own, in connection order.
    shadows: Vec<Shadow>,
    /// The connector and EDID identity of every connected output, for the screen nodes.
    names: BTreeMap<OutputId, (String, String)>,
    /// Calls the loop has not run yet.
    applies: Vec<PendingApply>,
    /// The picture a mirror shows in its next frame.
    pictures: BTreeMap<OutputId, Image>,
    /// Shadows whose next frame must be drawn whatever their source did, because they are new or changed.
    stale: BTreeSet<OutputId>,
}

impl OutputPolicy {
    /// Records the connector and identity of each connected output.
    pub(crate) fn remember(&mut self, connected: &[&Output]) {
        self.names = connected
            .iter()
            .map(|output| {
                (
                    output.id,
                    (output.connector.clone(), output.identity.clone().unwrap_or_default()),
                )
            })
            .collect();
    }

    /// The connector and identity of a connected output.
    pub(crate) fn name_of(&self, output: OutputId) -> Option<&(String, String)> {
        self.names.get(&output)
    }

    /// The shadows as they stand.
    pub(crate) fn shadows(&self) -> &[Shadow] {
        &self.shadows
    }

    /// Replaces the shadows, marking each new or changed one stale so it draws its next frame.
    pub(crate) fn set_shadows(&mut self, shadows: Vec<Shadow>) {
        for shadow in &shadows {
            if !self.shadows.contains(shadow) {
                self.stale.insert(shadow.output);
            }
        }
        self.stale
            .retain(|output| shadows.iter().any(|shadow| shadow.output == *output));
        self.pictures
            .retain(|output, _| shadows.iter().any(|shadow| shadow.output == *output));
        self.shadows = shadows;
    }

    /// The shadow an output is, if it is one.
    pub(crate) fn shadow(&self, output: OutputId) -> Option<Shadow> {
        self.shadows.iter().find(|shadow| shadow.output == output).copied()
    }

    /// Whether a shadow has to draw this iteration: its source drew, or it is new or changed.
    pub(crate) fn shadow_due(&self, shadow: Shadow, source_drew: bool) -> bool {
        self.stale.contains(&shadow.output) || (shadow.source.is_some() && source_drew)
    }

    /// Hands a mirror the picture its next frame shows.
    pub(crate) fn set_picture(&mut self, output: OutputId, picture: Image) {
        self.pictures.insert(output, picture);
    }
}

/// The rectangle a picture of `source` pixels takes in a panel of `target` logical pixels: scaled to fit, keeping its
/// aspect, centred. The bars are what the output draws black around it.
fn fit(source: Size, target: Size) -> Rect {
    if source.is_empty() || target.is_empty() {
        return Rect::new(0, 0, target.width, target.height);
    }
    let (source_width, source_height) = (u64::from(source.width), u64::from(source.height));
    let (target_width, target_height) = (u64::from(target.width), u64::from(target.height));
    // Whichever side runs out first sets the scale; rounding the other side to the nearest pixel keeps a letterbox
    // symmetric to within one pixel and never larger than the panel.
    let (width, height) = if target_width * source_height <= target_height * source_width {
        (
            target_width,
            ((source_height * target_width + source_width / 2) / source_width).clamp(1, target_height),
        )
    } else {
        (
            ((source_width * target_height + source_height / 2) / source_height).clamp(1, target_width),
            target_height,
        )
    };
    let (width, height) = (width as u32, height as u32);
    Rect::new(
        ((target.width - width) / 2) as i32,
        ((target.height - height) / 2) as i32,
        width,
        height,
    )
}

/// The layout the profiles describe for `connected`, or the plain row when they do not resolve.
///
/// A stored arrangement that no longer resolves (a monitor swapped for a bigger one under an absolute position) must
/// not leave the session without a desktop, so it is reported and the row stands until a new `apply` replaces it.
pub(crate) fn plan(
    descriptors: &[OutputDescriptor],
    sightings: &[OutputSighting<'_>],
    profiles: &ProfileStore,
) -> Option<(Layout, LayoutConfig)> {
    let config = profiles.layout(sightings);
    match compute(descriptors, &config) {
        Ok(layout) => Some((layout, config)),
        Err(error) => {
            log::warn!(
                area = "output",
                %error,
                "the stored output arrangement does not resolve; showing the default row"
            );
            let order: Vec<String> = sightings.iter().map(|seen| seen.connector.to_owned()).collect();
            let row = LayoutConfig::row_over(&order);
            match compute(descriptors, &row) {
                Ok(layout) => Some((layout, row)),
                Err(error) => {
                    log::warn!(area = "output", %error, "the default row was refused; keeping the previous layout");
                    None
                }
            }
        }
    }
}

/// The sighting of a backend output.
fn sighting(output: &Output) -> OutputSighting<'_> {
    OutputSighting {
        connector: &output.connector,
        identity: output.identity.as_deref(),
    }
}

/// The key of a request that a layout refusal belongs to, so the error can say where to look.
fn attribute(error: &LayoutError, change: &ProfileChange) -> Option<&'static str> {
    match error {
        LayoutError::NoEnabledOutputs | LayoutError::PrimaryNotEnabled(_) => Some("enabled"),
        LayoutError::MirrorOfMirror { .. }
        | LayoutError::MirrorSourceUnavailable { .. }
        | LayoutError::MirrorOverridesSource(_) => Some("mirror_of"),
        LayoutError::ModeNotSupported { .. } => Some("mode"),
        LayoutError::ScaleNotSupported { .. } => Some("scale"),
        _ if change.position.is_some() => Some("position"),
        _ if change.scale.is_some() => Some("scale"),
        _ if change.mode.is_some() => Some("mode"),
        _ => None,
    }
}

/// The mode and scale an output is driven at under `profile`: what it names, and what the backend already drives
/// for anything it leaves open. A mirror keeps what it drives, because it shows its source's picture at its own size.
fn wanted(output: &Output, profile: Option<&OutputProfile>) -> (DisplayMode, gnoblin_types::Scale) {
    match profile {
        Some(profile) if profile.mirror_of.is_none() => (
            profile.mode.unwrap_or(output.mode),
            profile.scale.unwrap_or(output.scale),
        ),
        _ => (output.mode, output.scale),
    }
}

impl State {
    /// Takes the shell's `apply` calls on screen nodes, to be run by the loop once it holds the backend.
    pub(crate) fn queue_output_applies(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let output = self.output_of_node(call.node);
            self.output_policy.applies.push(PendingApply {
                token: call.token,
                output,
                text: call.text.expose().to_owned(),
            });
        }
    }

    /// The output a screen node stands for, real or shadow.
    fn output_of_node(&self, node: gnoblin_types::NodeId) -> Option<OutputId> {
        self.output_policy
            .names
            .keys()
            .copied()
            .find(|output| self.composer.screen_node(*output) == Some(node))
    }

    /// Runs the queued `apply` calls against the backend and answers each: an empty error is success, anything else
    /// is why the arrangement was kept as it was.
    pub(crate) fn run_output_applies(&mut self, backend: &mut dyn Backend<State>) {
        if self.output_policy.applies.is_empty() {
            return;
        }
        for pending in std::mem::take(&mut self.output_policy.applies) {
            let result = match pending.output {
                Some(output) => self.apply_output(backend, output, &pending.text),
                None => Err("the output is not connected".to_owned()),
            };
            match result {
                Ok(()) => self.tree.answer(pending.token, "ok".to_owned(), String::new()),
                Err(reason) => {
                    log::info!(area = "output", %reason, "an output configuration was refused");
                    self.tree.answer(pending.token, String::new(), reason);
                }
            }
        }
    }

    /// Applies one request to one output through the transactional path described in the module docs.
    fn apply_output(&mut self, backend: &mut dyn Backend<State>, output: OutputId, text: &str) -> Result<(), String> {
        let change = ProfileChange::parse(text).map_err(|error| error.to_string())?;
        if change
            .orientation
            .is_some_and(|orientation| orientation != Orientation::Normal)
        {
            // The renderers draw an output upright only; accepting a rotation would report one the pixels do not show.
            return Err("transform: this compositor draws only `normal`".to_owned());
        }
        let outputs: Vec<Output> = backend.outputs().to_vec();
        let target = outputs
            .iter()
            .find(|candidate| candidate.id == output)
            .ok_or_else(|| "the output is not connected".to_owned())?;
        let sightings: Vec<OutputSighting<'_>> = outputs.iter().map(sighting).collect();
        let next = self
            .output_policy
            .profiles
            .find(sighting(target))
            .cloned()
            .unwrap_or_else(|| OutputProfile::new(&target.connector, target.identity.clone()))
            .with(&change);
        let mut candidate = self.output_policy.profiles.clone();
        if let Some(source) = &next.mirror_of {
            let named = outputs.iter().find(|candidate| &candidate.connector == source);
            let Some(named) = named.filter(|named| named.id != output) else {
                return Err(format!("mirror_of: {source} is not another connected output"));
            };
            let source_profile = candidate.find(sighting(named));
            if source_profile.is_some_and(|profile| !profile.enabled) {
                return Err(format!("mirror_of: {source} is off"));
            }
            if source_profile.is_some_and(|profile| profile.mirror_of.is_some()) {
                return Err(format!("mirror_of: {source} is itself a mirror"));
            }
        }
        candidate.keep(next);

        // The layout is resolved for the modes and scales the candidate would drive, so a refusal here costs the
        // hardware nothing.
        let descriptors: Vec<OutputDescriptor> = outputs
            .iter()
            .map(|candidate_output| {
                let (mode, scale) = wanted(candidate_output, candidate.find(sighting(candidate_output)));
                let mut descriptor =
                    OutputDescriptor::new(candidate_output.id, candidate_output.connector.clone(), vec![mode]);
                descriptor.scales = vec![scale];
                descriptor
            })
            .collect();
        if let Err(error) = compute(&descriptors, &candidate.layout(&sightings)) {
            return Err(match attribute(&error, &change) {
                Some(key) => format!("{key}: {error}"),
                None => error.to_string(),
            });
        }

        let steps: Vec<OutputChange> = outputs
            .iter()
            .filter_map(|candidate_output| {
                let (mode, scale) = wanted(candidate_output, candidate.find(sighting(candidate_output)));
                (mode != candidate_output.mode || scale != candidate_output.scale).then_some(OutputChange {
                    id: candidate_output.id,
                    mode,
                    before_mode: candidate_output.mode,
                    before: candidate_output.scale,
                    after: scale,
                })
            })
            .collect();
        if let Err(failure) = apply_in_order(&steps, |id, mode, scale| {
            backend
                .configure_output(id, mode, scale)
                .map_err(|error| error.to_string())
        }) {
            let message = failure.describe(|id| {
                outputs
                    .iter()
                    .find(|candidate_output| candidate_output.id == id)
                    .map_or_else(|| format!("output {}", id.get()), |found| found.connector.clone())
            });
            let key = if change.mode.is_some() { "mode" } else { "scale" };
            return Err(format!("{key}: {message}"));
        }
        self.output_policy.profiles = candidate;
        self.sync_outputs(backend.outputs());
        Ok(())
    }

    /// Settles the outputs after the backend reported a change: puts back the mode and scale a profile holds for each
    /// output that has just connected, then recomputes the layout.
    ///
    /// Only a newly connected output is driven: one that has been here keeps what it is driven at, so a change made
    /// elsewhere (the output management protocol) is not undone by an unrelated hotplug. A mode the backend refuses
    /// leaves that output at what it drives, and the layout falls back to the row if the profile then fails to resolve.
    pub(crate) fn settle_outputs(&mut self, backend: &mut dyn Backend<State>) {
        let outputs: Vec<Output> = backend.outputs().to_vec();
        let steps: Vec<OutputChange> = outputs
            .iter()
            .filter(|candidate| {
                self.outputs.iter().all(|entry| entry.backend != candidate.id)
                    && self.output_policy.shadow(candidate.id).is_none()
            })
            .filter_map(|candidate| {
                let profile = self.output_policy.profiles.find(sighting(candidate));
                let (mode, scale) = wanted(candidate, profile);
                (mode != candidate.mode || scale != candidate.scale).then_some(OutputChange {
                    id: candidate.id,
                    mode,
                    before_mode: candidate.mode,
                    before: candidate.scale,
                    after: scale,
                })
            })
            .collect();
        for step in steps {
            if let Err(failure) = apply_in_order(&[step], |id, mode, scale| {
                backend
                    .configure_output(id, mode, scale)
                    .map_err(|error| error.to_string())
            }) {
                log::warn!(area = "output", reason = %failure.reason, "a stored mode could not be restored");
            }
        }
        self.sync_outputs(backend.outputs());
    }

    /// Whether an output is one the desktop gives no place of its own.
    pub(crate) fn is_shadow(&self, output: OutputId) -> bool {
        self.output_policy.shadow(output).is_some()
    }

    /// The shadows, for the frame loop.
    pub(crate) fn shadow_outputs(&self) -> Vec<Shadow> {
        self.output_policy.shadows().to_vec()
    }

    /// Whether a shadow has to draw this iteration.
    pub(crate) fn shadow_due(&self, shadow: Shadow, source_drew: bool) -> bool {
        self.output_policy.shadow_due(shadow, source_drew)
    }

    /// Gives a mirror the picture of its source for its next frame.
    pub(crate) fn set_mirror_picture(&mut self, output: OutputId, picture: Image) {
        self.output_policy.set_picture(output, picture);
    }

    /// Describes the frame of a shadow: the source's picture fitted into the panel, or black when it is off.
    ///
    /// Returns whether there is something to draw. A mirror with no picture yet draws nothing and stays due.
    pub(crate) fn shadow_frame(&mut self, shadow: Shadow, frame: &mut OutputFrame) -> bool {
        frame.clear();
        let bounds = Rect::new(0, 0, shadow.logical.width, shadow.logical.height);
        if shadow.source.is_some() {
            let Some(picture) = self.output_policy.pictures.remove(&shadow.output) else {
                return false;
            };
            let Some(overlay) = Overlay::new(fit(picture.size(), shadow.logical), &picture) else {
                return false;
            };
            frame.push_layer(Layer::Overlay(overlay));
        }
        frame.push_damage(bounds);
        self.output_policy.stale.remove(&shadow.output);
        self.last_frame = Some((shadow.output, frame.damage().to_vec()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_of_the_same_size_fills_the_panel() {
        assert_eq!(fit(Size::new(64, 48), Size::new(64, 48)), Rect::new(0, 0, 64, 48));
        // A HiDPI source and panel at the same logical size are still a whole-panel picture.
        assert_eq!(fit(Size::new(128, 96), Size::new(64, 48)), Rect::new(0, 0, 64, 48));
    }

    #[test]
    fn a_wider_panel_gets_bars_at_the_sides() {
        // 4:3 into 16:9: the height is the limit, the picture is centred.
        assert_eq!(fit(Size::new(64, 48), Size::new(128, 72)), Rect::new(16, 0, 96, 72));
    }

    #[test]
    fn a_taller_panel_gets_bars_above_and_below() {
        assert_eq!(fit(Size::new(64, 48), Size::new(64, 64)), Rect::new(0, 8, 64, 48));
    }

    #[test]
    fn a_picture_never_leaves_the_panel_whatever_the_rounding() {
        for source in [(1, 1), (3, 7), (1920, 1080), (7, 3), (1366, 768)] {
            for target in [(1, 1), (5, 3), (64, 48), (1279, 721)] {
                let rect = fit(Size::new(source.0, source.1), Size::new(target.0, target.1));
                assert!(rect.x >= 0 && rect.y >= 0, "{source:?} into {target:?}: {rect:?}");
                assert!(
                    rect.x as u32 + rect.width <= target.0,
                    "{source:?} into {target:?}: {rect:?}"
                );
                assert!(
                    rect.y as u32 + rect.height <= target.1,
                    "{source:?} into {target:?}: {rect:?}"
                );
            }
        }
    }
}
