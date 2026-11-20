//! The frame-cost watchdog of the user shader pass, as the composer holds it (`design/modular-framework.md` 37.2 and 37.10).
//!
//! The policy is `gnoblin_render_frame::shader::watchdog`; this is the glue. A renderer records what each shader source cost on the frame it
//! drew (`OutputFrame::shader_timings`), the backend tells the host the draw is over ([`Composer::shader_drawn`]), and the composer
//! feeds the report to the watchdog and logs what it decides. The next frame leaves a switched-off source out of its layers
//! ([`Composer::compose`] calls `OutputFrame::drop_shader_passes`), and a node's `skipped` names the pass with the reason.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields.

use std::time::Duration;

use gnoblin_render::ShaderKey;
use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_render_frame::shader::SHADER_CACHE_ENTRIES;
use gnoblin_render_frame::shader::watchdog::{Cause, Ruling, ShaderRefusals, Watchdog};

use super::*;

/// The watchdog and what the renderers refused to build, for the composer.
#[derive(Debug, Default)]
pub(super) struct ShaderWatch {
    /// The strikes and refusals.
    pub(super) watchdog: Watchdog,
    /// The sources a renderer could not build a program for, with its reason. Not a strike: the node's `skipped` names the pass with
    /// the reason, and the source is tried again whenever a renderer is made. It is bounded, because a shell can write sources
    /// without end and each one a driver refuses would add an entry.
    pub(super) unbuildable: BTreeMap<ShaderKey, String>,
}

impl Composer {
    /// The sources refused for the session, shared with the tree's write path: a write of a refused source is answered `bad-value`.
    #[must_use]
    pub fn shader_refusals(&self) -> ShaderRefusals {
        self.shaders.watchdog.refusals().clone()
    }

    /// Replaces the refusal list with the one the tree's write path holds. The session's host calls it once, before any frame.
    pub fn set_shader_refusals(&mut self, refusals: ShaderRefusals) {
        self.shaders.watchdog.set_refusals(refusals);
    }

    /// Takes what the renderer recorded about `frame`'s shader passes and acts on it: a source over budget for three frames in a row, or
    /// in the frame of a device loss, takes a strike and is left out of the frames that follow; three strikes refuse it for the session.
    /// The backend calls it after every draw, whether the draw succeeded or not (`Host::drawn`).
    pub fn shader_drawn(&mut self, frame: &mut OutputFrame) {
        let report = frame.shader_timings().take();
        for (key, reason) in &report.refused {
            if self.shaders.unbuildable.len() >= SHADER_CACHE_ENTRIES * 4 {
                self.shaders.unbuildable.clear();
            }
            if self.shaders.unbuildable.get(key) != Some(reason) {
                log::warn!(
                    area = "render",
                    hash = %format_args!("{:016x}", key.get()),
                    %reason,
                    "the renderer could not build a shader pass; it is not drawn"
                );
                self.shaders.unbuildable.insert(*key, reason.clone());
            }
        }
        let drawn = if report.lost { frame.shader_keys() } else { Vec::new() };
        let mut struck = Vec::new();
        let now = self.shader_now();
        for event in self.shaders.watchdog.observe(now, &drawn, &report) {
            match event {
                Ruling::Struck { key, strikes, cause } => {
                    struck.push(key);
                    let why = match cause {
                        Cause::OverBudget(elapsed) => format!(
                            "{} ms in a frame, over the {} ms budget for {} frames in a row",
                            elapsed.as_millis(),
                            gnoblin_render_frame::shader::SHADER_FRAME_BUDGET.as_millis(),
                            gnoblin_render_frame::shader::FRAMES_OVER_BUDGET
                        ),
                        Cause::DeviceLost => "the device was lost while the frame held it".to_owned(),
                    };
                    log::warn!(
                        area = "render",
                        hash = %format_args!("{:016x}", key.get()),
                        strikes,
                        %why,
                        "a shader pass took a strike and is switched off; its node draws without it"
                    );
                }
                Ruling::Refused { key } => {
                    log::warn!(
                        area = "render",
                        hash = %format_args!("{:016x}", key.get()),
                        "a shader source is refused for the session after repeated strikes; writes of it answer bad-value"
                    );
                }
            }
        }
        if !struck.is_empty() {
            // The pass is out of the frames from now on, but a node that is not repainted keeps the pixels the slow pass drew:
            // the nodes that carry a struck source are repainted without it. The frame that was just drawn is left without it too,
            // for a backend that draws it again.
            let nodes: Vec<NodeId> = self
                .scene
                .effect_nodes()
                .filter(|node| {
                    self.scene.effects(*node).is_some_and(|effects| {
                        effects
                            .passes()
                            .iter()
                            .any(|pass| matches!(pass, Pass::Shader(shader) if struck.contains(&shader.key())))
                    })
                })
                .collect();
            for node in nodes {
                if let Some(bounds) = self.scene.absolute_bounds(node) {
                    self.scene.damage_rect(bounds);
                }
            }
            self.drop_switched_shaders(frame);
        }
    }

    /// The time the watchdog counts its cool-offs in: the composer's own clock, which the session advances with the time that passed.
    fn shader_now(&self) -> Duration {
        Duration::from_millis(self.view_clock_ms)
    }

    /// Leaves out of `frame` the shader passes the watchdog has switched off.
    pub(super) fn drop_switched_shaders(&self, frame: &mut OutputFrame) {
        let now = self.shader_now();
        frame.drop_shader_passes(|key| self.shaders.watchdog.switched(key, now).is_some());
    }

    /// The reason a node's shader pass `key` is not drawn on top of the renderer's own limits, if the watchdog or a renderer has one.
    pub(super) fn shader_skip_reason(&self, key: ShaderKey) -> Option<String> {
        self.shaders
            .watchdog
            .switched(key, self.shader_now())
            .map(|switched| switched.reason().to_owned())
            .or_else(|| self.shaders.unbuildable.get(&key).cloned())
    }
}
