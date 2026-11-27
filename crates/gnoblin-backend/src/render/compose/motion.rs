//! Motion: the shell-played animations, the window tweens and the visual bell's flash.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields.

use super::*;

/// How long a placement, move or other visual change takes to reach its target.
#[cfg(test)]
pub(super) const MOTION_MS: u32 = 200;

#[cfg(test)]
impl Composer {
    /// Sets the desktop's window motion, as the std library does in a session: these tests are about what the compositor's own motion does,
    /// and a composer nobody set motion on lands every change at once ([`MotionPolicy::default`]).
    pub(super) fn set_stock_motion(&mut self) {
        self.setting_set(Setting::Motion(MotionPolicy {
            map_fade_ms: MOTION_MS,
            unmap_fade_ms: MOTION_MS,
            retarget_ms: MOTION_MS,
        }));
    }
}

/// How long a visual bell's flash takes to fade out, in milliseconds.
pub(super) const FLASH_MS: u32 = 150;

/// The most an animation moves on the first loop turn after it starts: one frame at 60 Hz.
pub(super) const FIRST_STEP_MS: u32 = 16;

/// The alpha a visual bell's flash starts at, before it fades.
pub(super) const FLASH_PEAK: f32 = 0.5;

/// One visual bell flash in progress.
///
/// It is a white rectangle drawn over everything on the output, fading out over
/// [`FLASH_MS`]. The composer keeps no pixels for it: it is described afresh in every
/// frame until it ends, so the damage it leaves behind is what the next frame paints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Flash {
    /// The rectangle flashed, in global logical coordinates, or `None` for every output
    /// in full.
    pub(super) area: Option<Rect>,
    /// How long the flash has been running.
    pub(super) elapsed_ms: u32,
    /// Whether motion is reduced: the flash is drawn once at its peak and then ends with the frame
    /// that draws it, instead of fading.
    pub(super) reduced: bool,
}

/// A timed `set` of the shell that has tracks running: one for a number, three for a `rotation` or a `scale`.
#[derive(Clone, Debug)]
pub(super) struct ShellPlay {
    /// The end it reports when its last track is over; `completed` is cleared by any track that did not reach its target.
    pub(super) end: ShellTrackEnd,
    /// How many of its tracks have not ended.
    pub(super) live: usize,
}

impl Composer {
    /// Sets whether property changes land at once (`true`) or are animated (`false`).
    ///
    /// It applies to changes started after the call; a track already running finishes.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Starts a visual bell's flash over `window`, or over every output when `window` is `None`
    /// or not one the composer holds.
    ///
    /// A flash already running is replaced. With reduced motion the flash is drawn once at its
    /// peak and then ends with the frame that draws it; otherwise it fades out over [`FLASH_MS`].
    pub fn flash_window(&mut self, window: Option<WindowId>) {
        let area = window
            .and_then(|window| self.windows.get(&window))
            .map(|entry| entry.geometry);
        let flash = Flash {
            area,
            elapsed_ms: 0,
            reduced: self.reduced_motion,
        };
        if let Some(old) = self.flash.replace(flash) {
            self.damage_flash(old);
        }
        self.damage_flash(flash);
    }

    /// Advances every running animation by `elapsed_ms` and forgets the ones that ended.
    ///
    /// It also moves a visual bell's flash on, so the next frame shows its fade.
    pub fn advance_motion(&mut self, elapsed_ms: u32) {
        self.advance_motion_capping_new(elapsed_ms, u32::MAX);
    }

    /// [`advance_motion`](Self::advance_motion) for the loop's turn: an animation started since the previous turn
    /// moves by at most one frame ([`FIRST_STEP_MS`]) on this one.
    ///
    /// The loop reports the time since its previous turn, and after an idle stretch that is up to 100 ms that passed
    /// before the request which started the animation was handled, so the animation's clock starts at its request
    /// instead of that stretch and a fade lasts its full duration.
    pub fn advance_motion_from_turn(&mut self, elapsed_ms: u32) {
        self.advance_motion_capping_new(elapsed_ms, FIRST_STEP_MS);
    }

    fn advance_motion_capping_new(&mut self, elapsed_ms: u32, first_step_ms: u32) {
        self.view_clock_ms = self.view_clock_ms.saturating_add(u64::from(elapsed_ms));
        self.rearm_shaders();
        self.motion
            .advance_capping_new(&mut self.scene, elapsed_ms, first_step_ms);
        self.reap_motion_ends();
        self.advance_flash(elapsed_ms, first_step_ms);
    }

    /// Forgets the tracks the player ended, finishing a closing window's fade and noting the end of each shell write.
    pub(super) fn reap_motion_ends(&mut self) {
        for end in self.motion.take_ended() {
            let owner = self
                .tracks
                .iter()
                .find(|(_, track)| **track == end.track)
                .map(|(key, _)| *key);
            self.tracks.retain(|_, track| *track != end.track);
            self.shell_tracks.retain(|_, track| *track != end.track);
            self.note_shell_track_end(&end);
            if let Some((window, VisualProperty::Opacity)) = owner
                && let Some(on_end) = self
                    .windows
                    .get(&window)
                    .and_then(|entry| entry.closing.as_ref())
                    .map(|closing| closing.on_end)
            {
                self.finish_fade(window, on_end);
            }
        }
    }

    /// Applies what the shell's writes to animated properties and to `tween` nodes asked of the tracks, in the
    /// order the writes applied. A timed `set` plays a track to its target for each property it moves (three
    /// axes for a `rotation` or a `scale`); with reduced motion on, or for a key the scene cannot tween, the target
    /// lands at once instead, so a shell that forgot to shorten its durations still gets instant changes. A write
    /// to a property ends the track running on it either way.
    ///
    /// Reduced motion is read when the write is applied: a track already running when it turns on
    /// finishes, and the next write lands at once.
    pub fn apply_shell_motion(&mut self, motion: Vec<Motion>) {
        for item in motion {
            match item {
                Motion::Settle { node, key } => {
                    for property in animated_properties(&key) {
                        self.end_shell_track(node, *property);
                    }
                }
                Motion::Play(animation) => self.play_shell_animation(&animation),
                Motion::Control { node, control } => self.control_shell_group(node, control),
            }
        }
        // The ends this caused (a track superseded or cancelled by a write) are known now, not at the next turn.
        self.reap_motion_ends();
    }

    /// How many tracks the shell's timed writes have running.
    #[must_use]
    pub fn shell_tracks_running(&self) -> usize {
        self.shell_tracks.len()
    }

    /// Ends the track the shell's timed `set` started on `node`'s `property`, leaving the value where it is.
    pub(super) fn end_shell_track(&mut self, node: NodeId, property: VisualProperty) {
        if let Some(track) = self.shell_tracks.remove(&(node, property)) {
            let _ = self.motion.cancel(track);
        }
    }

    /// Plays one timed `set`, or lands its target at once when it cannot be played (reduced motion, a duration
    /// that rounds to nothing, a key the scene cannot tween). Either way the write ends once, and the shell is told.
    pub(super) fn play_shell_animation(&mut self, animation: &Animation) {
        let properties = animated_properties(&animation.key);
        for property in properties {
            self.end_shell_track(animation.node, *property);
        }
        if !self.reduced_motion && self.start_shell_tracks(animation, properties) {
            return;
        }
        let landed = land_at_once(&mut self.scene, animation);
        if let Err(code) = &landed {
            log::warn!(
                area = "render",
                node = %animation.node,
                key = %animation.key,
                code = code.as_str(),
                "an animated set could not land: the node keeps its value"
            );
        }
        self.shell_ends.push(ShellTrackEnd {
            node: animation.node,
            key: animation.key.clone(),
            group: animation.group,
            completed: landed.is_ok(),
        });
    }

    /// The value each of `properties` ends at for a timed `set`: a number for the one property of a number key, one
    /// axis each for a three-axis list. `None` for any other pairing, so nothing is started half way.
    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn shell_targets(
        animation: &Animation,
        properties: &[VisualProperty],
    ) -> Option<Vec<(VisualProperty, f32)>> {
        match (&animation.value, properties) {
            (TreeValue::Double(target), [property]) => Some(vec![(*property, *target as f32)]),
            (TreeValue::ListDouble(axes), properties) if axes.len() == properties.len() && !properties.is_empty() => {
                Some(
                    properties
                        .iter()
                        .copied()
                        .zip(axes.iter().map(|axis| *axis as f32))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// Starts the tracks of a timed `set`, one for each of `properties`. `false` when the duration rounds to
    /// nothing or the scene refuses a track, in which case none is left running and the caller lands the target at
    /// once.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(super) fn start_shell_tracks(&mut self, animation: &Animation, properties: &[VisualProperty]) -> bool {
        let Some(targets) = Self::shell_targets(animation, properties) else {
            return false;
        };
        let duration_ms = (animation.seconds * 1000.0).round() as u32;
        let delay_ms = (animation.delay * 1000.0).round() as u32;
        let Ok(curve) = SampledCurve::new(animation.samples.clone()) else {
            return false;
        };
        if duration_ms == 0 {
            return false;
        }
        let mut started: Vec<(VisualProperty, TrackId)> = Vec::with_capacity(targets.len());
        for (property, target) in targets {
            let mut spec = TrackSpec::new(animation.node, property, PropertyValue::number(target));
            spec.duration_ms = duration_ms;
            spec.delay_ms = delay_ms;
            match self.motion.start_sampled(&mut self.scene, spec, curve.clone()) {
                Ok(track) => started.push((property, track)),
                Err(_) => {
                    // The ones already running are stopped; their ends are not the shell's, no play holds them yet.
                    for (_, track) in started {
                        let _ = self.motion.cancel(track);
                    }
                    return false;
                }
            }
        }
        // The node keeps the target as its value, so a snapshot read mid-track agrees with the change the
        // shell's mirror was told.
        let _ = self
            .scene
            .set_value(animation.node, &animation.key, animation.value.clone());
        let play = self.next_play;
        self.next_play = self.next_play.saturating_add(1);
        let group = animation.group.filter(|group| self.scene.contains(*group));
        let paused = group.is_some_and(|group| self.scene.value(group, "paused") == Some(&TreeValue::Bool(true)));
        let reverse = group.is_some_and(|group| self.scene.value(group, "reverse") == Some(&TreeValue::Bool(true)));
        for (property, track) in &started {
            self.shell_tracks.insert((animation.node, *property), *track);
            self.track_plays.insert(*track, play);
            if let Some(group) = group {
                self.groups.entry(group).or_default().insert(*track);
            }
            // A track that joins a held or reversed group starts that way: the group's state is the shell's last
            // word on it, whatever order the writes came in.
            if paused {
                let _ = self.motion.pause(*track);
            }
            if reverse {
                let _ = self.motion.set_reverse(*track, true);
            }
        }
        self.shell_plays.insert(
            play,
            ShellPlay {
                end: ShellTrackEnd {
                    node: animation.node,
                    key: animation.key.clone(),
                    group,
                    completed: true,
                },
                live: started.len(),
            },
        );
        true
    }

    /// Moves, holds or turns the tracks that joined the `tween` node `group`.
    pub(super) fn control_shell_group(&mut self, group: NodeId, control: TweenControl) {
        let tracks: Vec<TrackId> = self
            .groups
            .get(&group)
            .map(|tracks| tracks.iter().copied().collect())
            .unwrap_or_default();
        for track in tracks {
            // A track that ended between the write and here is gone from the player and the group is tidied by
            // its end; the refusal is that, not a fault.
            let _ = match control {
                TweenControl::Progress(progress) => self.motion.seek(&mut self.scene, track, progress),
                TweenControl::Paused(true) => self.motion.pause(track),
                TweenControl::Paused(false) => self.motion.resume(track),
                TweenControl::Reverse(reverse) => self.motion.set_reverse(track, reverse),
            };
        }
    }

    /// Notes that a player track ended: when it was the last of its timed `set`, the shell is owed the end.
    pub(super) fn note_shell_track_end(&mut self, end: &TrackEnd) {
        let Some(play) = self.track_plays.remove(&end.track) else {
            return;
        };
        let Some(entry) = self.shell_plays.get_mut(&play) else {
            return;
        };
        if let Some(group) = entry.end.group
            && let Some(tracks) = self.groups.get_mut(&group)
        {
            tracks.remove(&end.track);
            if tracks.is_empty() {
                self.groups.remove(&group);
            }
        }
        entry.live = entry.live.saturating_sub(1);
        entry.end.completed &= end.reason == TrackEndReason::Completed;
        if entry.live == 0
            && let Some(done) = self.shell_plays.remove(&play)
        {
            self.shell_ends.push(done.end);
        }
    }

    /// Takes the ends of the shell's timed `set`s since the last call, in the order they ended, for the tree to
    /// publish as `track_ended`.
    pub fn take_shell_track_ends(&mut self) -> Vec<ShellTrackEnd> {
        std::mem::take(&mut self.shell_ends)
    }

    /// Whether a visual bell's flash is still on screen, so the caller keeps frames coming.
    #[must_use]
    pub fn flash_running(&self) -> bool {
        self.flash.is_some()
    }

    /// Moves the flash on by `elapsed_ms`, damaging the area it covers so the next frame repaints
    /// it. A reduced-motion flash does not move: it is ended by the frame that draws it, in
    /// [`Composer::end_frame`].
    pub(super) fn advance_flash(&mut self, elapsed_ms: u32, first_step_ms: u32) {
        let Some(mut flash) = self.flash else {
            return;
        };
        if flash.reduced {
            return;
        }
        self.damage_flash(flash);
        // A flash that has not moved yet was started this turn, after the idle stretch the turn reports.
        let elapsed_ms = if flash.elapsed_ms == 0 {
            elapsed_ms.min(first_step_ms)
        } else {
            elapsed_ms
        };
        flash.elapsed_ms = flash.elapsed_ms.saturating_add(elapsed_ms);
        if flash.elapsed_ms >= FLASH_MS {
            self.flash = None;
        } else {
            self.flash = Some(flash);
        }
    }

    /// Damages the area a flash covers: every output in full when it has no area.
    pub(super) fn damage_flash(&mut self, flash: Flash) {
        match flash.area {
            Some(area) => self.scene.damage_rect(area),
            None => {
                let outputs: Vec<OutputId> = self.scene.outputs().keys().copied().collect();
                for output in outputs {
                    self.scene.damage_output(output);
                }
            }
        }
    }

    /// The alpha a flash is drawn at: its peak, fading to nothing over [`FLASH_MS`] unless motion
    /// is reduced.
    pub(super) fn flash_alpha(flash: Flash) -> f32 {
        if flash.reduced {
            return FLASH_PEAK;
        }
        let remaining = FLASH_MS.saturating_sub(flash.elapsed_ms);
        FLASH_PEAK * (remaining as f32 / FLASH_MS as f32)
    }

    /// Starts an animation towards each property's target that differs from the node's value, when `animate` is set
    /// and the compositor's own motion plays (motion is not reduced and the shell did not turn it off). A property
    /// starts from its current value, so a superseded animation continues from where it is. Otherwise the target is
    /// written at once. The animation takes the policy's `retarget_ms`.
    pub(super) fn retarget(&mut self, window: WindowId, animate: bool) {
        self.retarget_over(window, animate, self.motion_policy.retarget_ms);
    }

    /// [`Composer::retarget`] over `duration_ms`; zero lands the target at once.
    pub(super) fn retarget_over(&mut self, window: WindowId, animate: bool, duration_ms: u32) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };

        let immediate = !animate || !self.builtin_motion_plays() || duration_ms == 0;
        let mut landing = Vec::new();
        let target = self.placed_target(entry);
        for property in VisualProperty::ALL {
            let Some(value) = property.get(&target) else {
                continue;
            };
            self.cancel_track(window, property);
            if self.scene.property(node, property).ok() == Some(value) {
                continue;
            }
            if immediate {
                landing.push((property, value));
                continue;
            }
            let spec = TrackSpec {
                from: TrackStart::Current,
                duration_ms,
                easing: Easing::EaseOutCubic,
                ..TrackSpec::new(node, property, value)
            };
            if let Ok(track) = self.motion.start(&mut self.scene, spec) {
                self.tracks.insert((window, property), track);
            }
        }
        self.land(node, &landing);
    }

    /// Writes several properties of one node as a single transform change.
    ///
    /// The scene damages the node's before and after bounds on every write, so writing
    /// the properties one at a time would damage each intermediate pose: a move from
    /// (20, 30) to (200, 300) would also damage (200, 30). A property a value refuses is
    /// left as it was, as `Scene::set_property` leaves it.
    pub(super) fn land(&mut self, node: NodeId, changes: &[(VisualProperty, PropertyValue)]) {
        let Some(mut candidate) = self.scene.transform(node).copied() else {
            return;
        };
        let mut changed = false;
        for &(property, value) in changes {
            let mut next = candidate;
            if property.apply(&mut next, value).is_ok() {
                candidate = next;
                changed = true;
            }
        }
        if changed {
            let _ = self.scene.set_transform(node, candidate);
        }
    }

    /// Cancels a window property's animation, leaving the value where it is.
    pub(super) fn cancel_track(&mut self, window: WindowId, property: VisualProperty) {
        if let Some(track) = self.tracks.remove(&(window, property)) {
            let _ = self.motion.cancel(track);
        }
    }

    /// Cancels every animation of a window.
    pub(super) fn cancel_tracks(&mut self, window: WindowId) {
        let keys: Vec<(WindowId, VisualProperty)> = self.tracks.keys().filter(|key| key.0 == window).copied().collect();
        for (owner, property) in keys {
            self.cancel_track(owner, property);
        }
    }
}
