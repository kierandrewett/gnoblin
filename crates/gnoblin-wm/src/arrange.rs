//! Per-workspace layout arrangement: the bridge between the wm and a
//! [`LayoutEngine`] (design/rust-compositor.md, D17).
//!
//! Each declared workspace owns one engine, created by the factory given to
//! [`crate::Wm::set_layout_engines`]. The wanted input for a workspace is its
//! tiled set on its output and that output's work area. Whenever the wanted
//! input differs from the last submitted one, the wm submits a new run; a
//! [`crate::Event::Frame`] collects finished runs with a deadline. The layout
//! never writes window state or geometry: a settled `Float` effect is applied
//! through the same path as [`crate::Event::SetFloating`].
//!
//! With no factory installed, nothing in this module runs and the wm behaves as
//! before.
//!
//! ## Failure modes
//!
//! - No factory installed: no layout, no submits, no effects.
//! - Empty tiled set: nothing is submitted and the last placements are dropped.
//!   The windows that left tiling are reported by their own `Configure` effects.
//! - Invalid parameters (empty or out-of-bounds output or window size): the engine
//!   is not called; [`crate::Effect::LayoutFailed`] with `invalid-params` is sent
//!   once when the layout was previously submitted. The last good placements stay
//!   in force.
//! - Engine still running at the deadline: [`crate::layout::Collected::Pending`];
//!   the last good placements stay in force and no effect is sent.
//! - Engine error, timeout, disabled layout, wedged worker, or an invalid reply
//!   (refused by [`crate::layout::settle`]): [`crate::Effect::LayoutFailed`] with
//!   the failure's reason; the last good placements stay in force. The run is
//!   retried when the wanted input next changes.
//! - Late or stale generation: discarded by [`crate::layout::Slot`] and reported as
//!   pending; it never reaches the wm.
//! - Inputs change while a run is in flight: the run is superseded by a new
//!   submit. A result for the old inputs is never emitted.
//! - Result for a workspace that no longer exists or has no submitted input: dropped.
//! - Output removed: the layout moves to the first surviving output and is
//!   re-arranged against its work area. With no output left, the layout waits for
//!   an output and submits nothing.
//! - Output added while a workspace has no output: the layout takes the new output.
//! - Workspace removed: its layout and engine are dropped.
//! - Workspace switch: every workspace keeps its engine and its arrangement; only
//!   visibility changes.
//! - A settled `Float` effect is applied with `set_floating(window, true)` when the
//!   window is still tiled on that workspace; the placement of a floated window is
//!   not emitted.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use gnoblin_types::{OutputId, RectF, WindowId, WorkspaceId};

use crate::effect::Effect;
use crate::layout::{
    Collected, Job, Layout, LayoutEngine, LayoutFailure, LayoutWindow, Params, ParamsError, Placement, Request,
    RequestKind, Slot, Transform3,
};
use crate::output::Output;
use crate::window::{Window, WindowState};

/// Builds one engine for each workspace.
pub(crate) type Factory = Box<dyn FnMut() -> Box<dyn LayoutEngine>>;

/// Every workspace's layout state, and the factory that makes engines.
#[derive(Default)]
pub(crate) struct Layouts {
    factory: Option<Factory>,
    by_workspace: BTreeMap<WorkspaceId, WorkspaceLayout>,
}

/// One workspace's engine and the parameters of its latest submitted run.
struct WorkspaceLayout {
    output: Option<OutputId>,
    slot: Slot,
    /// The parameters of the latest submitted run, `None` when nothing is tiled or
    /// the input was refused.
    submitted: Option<Params>,
    /// Where the last good placements put each window, for directional focus.
    /// Empty when nothing is tiled or the layout has no placements.
    placed: BTreeMap<WindowId, RectF>,
}

/// Whether a window takes part in tiling.
pub(crate) fn is_tiled(state: &WindowState) -> bool {
    !state.floating && !state.minimised && !state.fullscreen && !state.maximised && state.snapped.is_none()
}

/// How a geometry request was answered.
pub(crate) enum RequestVerdict {
    /// The layout answered; its placements are in force.
    Handled(Layout),
    /// The layout declined or did not answer in time, or nothing is tiled on the
    /// workspace; the built-in logic applies.
    Fallthrough,
    /// The layout failed or its reply was refused; the last good placements stay.
    Failed(LayoutFailure),
    /// The workspace's tiled input cannot be used.
    InvalidParams,
}

/// The rectangle a placement occupies, or `None` when its transform is a raw matrix.
fn placed_rect(placement: &Placement) -> Option<RectF> {
    match placement.transform {
        Transform3::Trs { position, .. } => Some(RectF {
            x: position.x,
            y: position.y,
            width: placement.width,
            height: placement.height,
        }),
        Transform3::Matrix(_) => None,
    }
}

impl WorkspaceLayout {
    /// Keeps the rectangles of a settled layout's placements.
    fn remember(&mut self, layout: &Layout) {
        self.placed = layout
            .placements
            .iter()
            .filter_map(|placement| placed_rect(placement).map(|rect| (placement.window, rect)))
            .collect();
    }
}

/// The parameters a workspace's layout needs right now, or `None` when there is
/// nothing to arrange.
pub(crate) fn tiled_params(
    workspace: WorkspaceId,
    output: Option<OutputId>,
    windows: &BTreeMap<WindowId, Window>,
    outputs: &[Output],
) -> Result<Option<Params>, ParamsError> {
    let Some(output) = output else {
        return Ok(None);
    };
    let Some(work_area) = outputs.iter().find(|entry| entry.id() == output).map(Output::work_area) else {
        return Ok(None);
    };
    let tiled: Vec<LayoutWindow> = windows
        .values()
        .filter(|window| window.workspace == workspace && window.output == Some(output) && is_tiled(&window.state))
        .map(|window| LayoutWindow {
            id: window.id,
            width: f64::from(window.geometry.width),
            height: f64::from(window.geometry.height),
        })
        .collect();
    if tiled.is_empty() {
        return Ok(None);
    }
    Params::new(f64::from(work_area.width), f64::from(work_area.height), tiled).map(Some)
}

impl fmt::Debug for Layouts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Layouts")
            .field("enabled", &self.is_enabled())
            .field("workspaces", &self.by_workspace.len())
            .finish()
    }
}

impl Layouts {
    /// Whether a factory is installed.
    pub(crate) fn is_enabled(&self) -> bool {
        self.factory.is_some()
    }

    /// Installs `factory` and drops every existing layout and engine.
    pub(crate) fn install(&mut self, factory: Factory) {
        self.factory = Some(factory);
        self.by_workspace.clear();
    }

    /// Creates a layout for each declared workspace, drops the layouts of removed
    /// workspaces, and submits a run for each workspace whose wanted input changed.
    ///
    /// `wanted` gives the parameters for a workspace on its current output. An
    /// error means the input cannot be used: nothing is submitted, and the failure
    /// is reported once until the input changes.
    pub(crate) fn sync(
        &mut self,
        workspaces: &[WorkspaceId],
        first_output: Option<OutputId>,
        wanted: impl Fn(WorkspaceId, Option<OutputId>) -> Result<Option<Params>, ParamsError>,
    ) -> Vec<Effect> {
        let Some(factory) = self.factory.as_mut() else {
            return Vec::new();
        };
        self.by_workspace.retain(|id, _| workspaces.contains(id));
        for id in workspaces {
            self.by_workspace.entry(*id).or_insert_with(|| WorkspaceLayout {
                output: first_output,
                slot: Slot::new(factory()),
                submitted: None,
                placed: BTreeMap::new(),
            });
        }
        let mut effects = Vec::new();
        for (id, layout) in &mut self.by_workspace {
            let params = match wanted(*id, layout.output) {
                Ok(params) => params,
                Err(_) => {
                    layout.placed.clear();
                    if layout.submitted.take().is_some() {
                        effects.push(Effect::LayoutFailed {
                            workspace: *id,
                            reason: "invalid-params",
                        });
                    }
                    continue;
                }
            };
            if params == layout.submitted {
                continue;
            }
            match params {
                Some(params) => {
                    layout.slot.submit(Job::Arrange(params.clone()));
                    layout.submitted = Some(params);
                }
                None => {
                    layout.submitted = None;
                    layout.placed.clear();
                }
            }
        }
        effects
    }

    /// Collects every run that finished before `deadline`. A run whose workspace
    /// has nothing submitted any more is dropped.
    pub(crate) fn collect(&mut self, deadline: Instant) -> Vec<(WorkspaceId, Result<Layout, LayoutFailure>)> {
        let mut finished = Vec::new();
        for (id, layout) in &mut self.by_workspace {
            match layout.slot.collect(deadline) {
                Collected::Idle | Collected::Pending | Collected::Declined => {}
                Collected::Settled(settled) => {
                    if layout.submitted.is_some() {
                        layout.remember(&settled);
                        finished.push((*id, Ok(settled)));
                    }
                }
                Collected::Failed(failure) => {
                    if layout.submitted.is_some() {
                        finished.push((*id, Err(failure)));
                    }
                }
            }
        }
        finished
    }

    /// Gives layouts that have no output the newly connected `output`.
    pub(crate) fn output_connected(&mut self, output: OutputId) {
        for entry in self.by_workspace.values_mut().filter(|entry| entry.output.is_none()) {
            entry.output = Some(output);
        }
    }

    /// Moves the layouts of a removed output onto `survivor`, or leaves them
    /// without an output when none is left.
    pub(crate) fn output_disconnected(&mut self, removed: OutputId, survivor: Option<OutputId>) {
        for entry in self
            .by_workspace
            .values_mut()
            .filter(|entry| entry.output == Some(removed))
        {
            entry.output = survivor;
        }
    }

    /// The last good placed rectangle of `window` on `workspace`, if the layout
    /// placed it.
    pub(crate) fn placed(&self, workspace: WorkspaceId, window: WindowId) -> Option<RectF> {
        self.by_workspace.get(&workspace)?.placed.get(&window).copied()
    }

    /// Offers one geometry request to `workspace`'s engine and waits until
    /// `deadline`. `wanted` gives the tiled input on the workspace's output after
    /// the change, as [`Layouts::sync`] does.
    pub(crate) fn offer(
        &mut self,
        workspace: WorkspaceId,
        kind: RequestKind,
        deadline: Instant,
        wanted: impl Fn(Option<OutputId>) -> Result<Option<Params>, ParamsError>,
    ) -> RequestVerdict {
        let Some(layout) = self.by_workspace.get_mut(&workspace) else {
            return RequestVerdict::Fallthrough;
        };
        let params = match wanted(layout.output) {
            Ok(Some(params)) => params,
            Ok(None) => {
                layout.submitted = None;
                layout.placed.clear();
                return RequestVerdict::Fallthrough;
            }
            Err(_) => {
                layout.submitted = None;
                layout.placed.clear();
                return RequestVerdict::InvalidParams;
            }
        };
        layout.slot.submit_request(Request {
            params: params.clone(),
            kind,
        });
        match layout.slot.collect(deadline) {
            Collected::Settled(settled) => {
                layout.submitted = Some(params);
                layout.remember(&settled);
                RequestVerdict::Handled(settled)
            }
            Collected::Failed(failure) => {
                layout.submitted = Some(params);
                RequestVerdict::Failed(failure)
            }
            Collected::Declined | Collected::Idle => {
                layout.submitted = None;
                RequestVerdict::Fallthrough
            }
            Collected::Pending => {
                layout.slot.cancel();
                layout.submitted = None;
                RequestVerdict::Fallthrough
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maximised_or_floating_window_is_not_tiled() {
        let tiled = WindowState::default();
        assert!(is_tiled(&tiled));
        assert!(!is_tiled(&WindowState {
            floating: true,
            ..tiled
        }));
        assert!(!is_tiled(&WindowState {
            maximised: true,
            ..tiled
        }));
        assert!(!is_tiled(&WindowState {
            fullscreen: true,
            ..tiled
        }));
        assert!(!is_tiled(&WindowState {
            minimised: true,
            ..tiled
        }));
    }
}
