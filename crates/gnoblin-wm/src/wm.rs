//! The window management state machine: events in, effects out.

use std::collections::BTreeMap;

use gnoblin_types::{Insets, OutputId, Rect, Size, WindowId, WorkspaceId};

use crate::activation::ActivationDecision;
use crate::drag::{self, DragConstraint};
use crate::effect::{Effect, RuleRejection};
use crate::event::{Event, Rejection, WindowSpec};
use crate::geometry::clamp_to_i32;
use crate::output::Output;
use crate::placement::{self, PlacementMode};
use crate::rules::{Rule, RuleAction};
use crate::snapping::SnapZone;
use crate::stack::{Stack, StackLayer};
use crate::window::{Window, WindowState};
use crate::workspace::Workspaces;

/// Size used for a window that asks for none.
///
/// A client that has not committed a size yet still has to be placed, so the
/// manager needs a rectangle that it can reason about. The value is small enough to
/// fit any output and large enough to receive input.
const DEFAULT_WINDOW_SIZE: Size = Size::new(800, 600);

/// How many windows the focus history remembers.
///
/// The history is only used to choose the next focus, so a short list is enough and
/// bounds the work done when a focused window closes.
const HISTORY_LIMIT: usize = 64;

/// The configuration the window manager reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WmOptions {
    /// `window_management.new_window_placement`.
    pub placement: PlacementMode,
    /// `window_management.edge_tiling`.
    pub edge_tiling: bool,
    /// `window_management.constrain_drag_to_work_area`.
    pub drag_constraint: DragConstraint,
}

impl Default for WmOptions {
    /// The compositor defaults from the configuration reference, which are the
    /// values a session gets when its configuration omits the keys.
    fn default() -> Self {
        Self {
            placement: PlacementMode::FreeSpace,
            edge_tiling: false,
            drag_constraint: DragConstraint::WorkArea,
        }
    }
}

/// Where a window belongs once it is in none of the special states.
///
/// `restore` is the floating rectangle a window returns to, so this is the single
/// definition of "a non-fullscreen window's resting place". Maximise, snapping and
/// leaving fullscreen all go through it, which is what keeps a snapped or maximised
/// window on its zone after the work area changes.
fn destination(state: WindowState, restore: Rect, work_area: Rect) -> Rect {
    if state.maximised {
        return work_area;
    }
    if let Some(zone) = state.snapped {
        return zone.rect(work_area);
    }
    restore.clamp_inside(work_area)
}

/// Where a window's rectangle belongs after an output change.
fn reshape(state: WindowState, geometry: Rect, work_area: Rect, monitor: Rect) -> Rect {
    if state.fullscreen {
        return monitor;
    }
    if state.maximised {
        return work_area;
    }
    if let Some(zone) = state.snapped {
        return zone.rect(work_area);
    }
    geometry.clamp_inside(work_area)
}

/// The stack layer a window belongs in for its state.
fn layer_of(state: WindowState) -> StackLayer {
    if state.fullscreen {
        StackLayer::Fullscreen
    } else {
        StackLayer::Normal
    }
}

/// A broken invariant reported by [`Wm::check_invariants`].
///
/// Each variant names the property that failed and the window or workspace that
/// made it fail, because the only consumer is a test or a debug assertion and the
/// message has to be enough to find the event that broke it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Broken {
    /// A workspace is declared but none is active.
    NoActiveWorkspace,
    /// The active workspace is not declared.
    ActiveWorkspaceUnknown(WorkspaceId),
    /// No workspace is declared but the manager considers one active.
    ActiveWorkspaceWhileNone,
    /// The stack and the window map disagree on how many windows exist.
    StackLength {
        /// Windows in the map.
        windows: usize,
        /// Entries in the stack.
        stack: usize,
    },
    /// A window is missing from the stack or appears more than once.
    StackCount {
        /// The window that is wrong.
        window: WindowId,
        /// How many times it appears.
        occurrences: usize,
    },
    /// A stacked window has no layer, so the order cannot be checked.
    StackLayerMissing(WindowId),
    /// A lower layer is stacked above a higher one.
    LayerOrder {
        /// The window that should be lower.
        upper: WindowId,
        /// The window that should be higher.
        lower: WindowId,
    },
    /// A window belongs to a workspace that is not declared.
    WindowWorkspaceUnknown {
        /// The window.
        window: WindowId,
        /// The workspace it claims.
        workspace: WorkspaceId,
    },
    /// A window has no output while outputs are connected.
    WindowOutputMissing(WindowId),
    /// A window has an output while none is connected.
    WindowOutputWhileNone(WindowId),
    /// A window's output is not connected.
    WindowOutputUnknown {
        /// The window.
        window: WindowId,
        /// The output it claims.
        output: OutputId,
    },
    /// A window has a degenerate rectangle.
    EmptyGeometry(WindowId),
    /// A maximised window does not fill its work area.
    MaximisedGeometry(WindowId),
    /// A fullscreen window does not fill its output.
    FullscreenGeometry(WindowId),
    /// A snapped window does not match its zone.
    SnappedGeometry(WindowId),
    /// A floating window does not overlap its work area at all.
    WindowOffWorkArea(WindowId),
    /// A transient belongs to a different workspace than its parent.
    TransientWorkspaceMismatch {
        /// The transient.
        window: WindowId,
        /// Its parent.
        parent: WindowId,
    },
    /// The focused window is not managed.
    FocusUnknown(WindowId),
    /// The focused window is hidden, so a key press would go nowhere.
    FocusHidden(WindowId),
    /// The focus history names a window that is not managed.
    HistoryUnknown(WindowId),
}

/// The window manager.
///
/// One instance per compositor. Build it with [`Wm::new`], feed it events with
/// [`Wm::apply`] and read state back through the accessors. There is no interior
/// mutability and no background work, so a caller can hold `&Wm` while it applies
/// the returned effects.
#[derive(Clone, Debug, Default)]
pub struct Wm {
    options: WmOptions,
    outputs: Vec<Output>,
    workspaces: Workspaces,
    active_workspace: Option<WorkspaceId>,
    windows: BTreeMap<WindowId, Window>,
    stack: Stack,
    focus: Option<WindowId>,
    history: Vec<WindowId>,
    rules: Vec<Rule>,
}

impl Wm {
    /// A manager with no outputs, no workspaces and no windows.
    ///
    /// Workspaces and outputs arrive as events, because they are the session's
    /// configuration and hardware rather than something policy can invent. A window
    /// that arrives before either exists is rejected with
    /// [`Rejection::NoWorkspace`] or [`Rejection::NoOutput`] instead of being
    /// managed somewhere invisible.
    #[must_use]
    pub fn new(options: WmOptions) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }

    /// The configuration in force.
    #[must_use]
    pub fn options(&self) -> WmOptions {
        self.options
    }

    /// Feeds one event in and returns the effects that follow.
    ///
    /// Effects are ordered so that a caller can apply them in sequence: state
    /// changes for the windows involved, then visibility, then stacking, then focus,
    /// then workspace and rule notifications. The function is deterministic: the
    /// same manager state and the same event always produce the same effects, and
    /// nothing in this crate reads a clock or a random source.
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::OutputConnected { output, rect } => self.output_connected(output, rect),
            Event::OutputDisconnected { output } => self.output_disconnected(output),
            Event::OutputMoved { output, rect } => match self.outputs.iter_mut().find(|entry| entry.id() == output) {
                Some(entry) => {
                    entry.set_rect(rect);
                    self.output_layout_changed(output)
                }
                None => vec![Effect::rejected(Rejection::UnknownOutput(output))],
            },
            Event::OutputReserved { output, reserved } => {
                match self.outputs.iter_mut().find(|entry| entry.id() == output) {
                    Some(entry) => {
                        entry.set_reserved(reserved);
                        self.output_layout_changed(output)
                    }
                    None => vec![Effect::rejected(Rejection::UnknownOutput(output))],
                }
            }
            Event::WindowOpened(spec) => self.window_opened(spec),
            Event::WindowClosed { window } => self.window_closed(window),
            Event::WindowTitleChanged { window, title } => self.window_title_changed(window, title),
            Event::WindowAppIdChanged { window, app_id } => self.window_app_id_changed(window, app_id),
            Event::RulesChanged { rules } => {
                // Rules apply when a window is taken into management, never to
                // windows that are already managed, so a reload cannot re-place or
                // re-maximise an existing window.
                self.rules = rules;
                Vec::new()
            }
            Event::WorkspaceAdded { workspace } => self.workspace_added(workspace),
            Event::WorkspaceRemoved { workspace } => self.workspace_removed(workspace),
            Event::WorkspaceActivated { workspace } => self.workspace_activated(workspace),
            Event::ActivationRequested { window, decision } => self.activation(window, decision),
            Event::Raise { window } => self.raise(window),
            Event::Lower { window } => self.lower(window),
            Event::Minimise { window } => self.minimise(window),
            Event::Unminimise { window } => self.unminimise(window),
            Event::ToggleMaximise { window } => self.toggle_maximise(window),
            Event::ToggleFullscreen { window } => self.toggle_fullscreen(window),
            Event::SnapRequested { window, zone } => self.snap(window, zone),
            Event::Unsnap { window } => self.unsnap(window),
            Event::MoveResize { window, geometry } => self.move_resize(window, geometry),
        }
    }

    /// The connected outputs, in connection order. The first is the primary.
    #[must_use]
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// One output.
    #[must_use]
    pub fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|output| output.id() == id)
    }

    /// The work area of an output, if it is connected.
    #[must_use]
    pub fn work_area(&self, id: OutputId) -> Option<Rect> {
        self.output(id).map(Output::work_area)
    }

    /// The declared workspaces, in position order.
    #[must_use]
    pub fn workspaces(&self) -> &[WorkspaceId] {
        self.workspaces.ids()
    }

    /// The workspace the outputs are showing, if any is declared.
    #[must_use]
    pub fn active_workspace(&self) -> Option<WorkspaceId> {
        self.active_workspace
    }

    /// Every managed window, ordered by id.
    pub fn windows(&self) -> impl Iterator<Item = &Window> + '_ {
        self.windows.values()
    }

    /// One managed window.
    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    /// The focused window.
    #[must_use]
    pub fn focused(&self) -> Option<WindowId> {
        self.focus
    }

    /// The focus history, most recently focused first.
    #[must_use]
    pub fn focus_history(&self) -> &[WindowId] {
        &self.history
    }

    /// The stacking order, topmost first.
    #[must_use]
    pub fn stack(&self) -> &[WindowId] {
        self.stack.order()
    }

    /// The configured window rules.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Whether a window would be drawn right now.
    #[must_use]
    pub fn is_visible(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.is_visible(self.active_workspace))
    }

    /// Checks every invariant the state machine promises, counting the checks it ran.
    ///
    /// The count exists so a model test can prove how much it exercised rather than
    /// only that it did not fail. Callers use this in debug assertions and in tests;
    /// a release compositor does not need it.
    ///
    /// # Errors
    ///
    /// The first broken invariant, with the window or workspace that broke it.
    pub fn check_invariants(&self) -> Result<usize, Broken> {
        let mut checks = 0_usize;

        // The active workspace must exist exactly when one is declared.
        checks += 1;
        match (self.workspaces.is_empty(), self.active_workspace) {
            (true, Some(_)) => return Err(Broken::ActiveWorkspaceWhileNone),
            (false, None) => return Err(Broken::NoActiveWorkspace),
            (true, None) => {}
            (false, Some(workspace)) => {
                checks += 1;
                if !self.workspaces.contains(workspace) {
                    return Err(Broken::ActiveWorkspaceUnknown(workspace));
                }
            }
        }

        // The stack holds every window exactly once, in layer order.
        checks += 1;
        if self.stack.len() != self.windows.len() {
            return Err(Broken::StackLength {
                windows: self.windows.len(),
                stack: self.stack.len(),
            });
        }
        for window in self.windows.keys() {
            checks += 1;
            let occurrences = self.stack.order().iter().filter(|entry| *entry == window).count();
            if occurrences != 1 {
                return Err(Broken::StackCount {
                    window: *window,
                    occurrences,
                });
            }
        }
        for pair in self.stack.order().windows(2) {
            checks += 1;
            let (upper, lower) = (pair[0], pair[1]);
            let (Some(upper_layer), Some(lower_layer)) = (self.stack.layer_of(upper), self.stack.layer_of(lower))
            else {
                return Err(Broken::StackLayerMissing(upper));
            };
            if upper_layer < lower_layer {
                return Err(Broken::LayerOrder { upper, lower });
            }
        }

        // Every window sits on one declared workspace and one usable output, and its
        // rectangle matches the state it is in.
        for window in self.windows.values() {
            checks += 1;
            if !self.workspaces.contains(window.workspace) {
                return Err(Broken::WindowWorkspaceUnknown {
                    window: window.id,
                    workspace: window.workspace,
                });
            }
            checks += 1;
            match (self.outputs.is_empty(), window.output) {
                (true, Some(_)) => return Err(Broken::WindowOutputWhileNone(window.id)),
                (false, None) => return Err(Broken::WindowOutputMissing(window.id)),
                (true, None) => {}
                (false, Some(output)) => {
                    if self.output(output).is_none() {
                        return Err(Broken::WindowOutputUnknown {
                            window: window.id,
                            output,
                        });
                    }
                }
            }
            checks += 1;
            if window.geometry.is_empty() {
                return Err(Broken::EmptyGeometry(window.id));
            }
            if let Some(output) = window.output.and_then(|id| self.output(id)) {
                let work_area = output.work_area();
                checks += 1;
                if window.state.fullscreen && window.geometry != output.rect() {
                    return Err(Broken::FullscreenGeometry(window.id));
                }
                checks += 1;
                if window.state.maximised && !window.state.fullscreen && window.geometry != work_area {
                    return Err(Broken::MaximisedGeometry(window.id));
                }
                if let Some(zone) = window.state.snapped {
                    checks += 1;
                    if !window.state.fullscreen && !window.state.maximised && window.geometry != zone.rect(work_area) {
                        return Err(Broken::SnappedGeometry(window.id));
                    }
                }
                checks += 1;
                if !window.state.fullscreen
                    && !window.state.maximised
                    && window.state.snapped.is_none()
                    && !window.geometry.intersects(work_area)
                {
                    return Err(Broken::WindowOffWorkArea(window.id));
                }
            }
            if let Some(parent) = window.transient_for
                && parent != window.id
            {
                checks += 1;
                if let Some(parent) = self.windows.get(&parent)
                    && parent.workspace != window.workspace
                {
                    return Err(Broken::TransientWorkspaceMismatch {
                        window: window.id,
                        parent: parent.id,
                    });
                }
            }
        }

        // Focus is either absent or a window that can actually receive input.
        checks += 1;
        if let Some(focused) = self.focus {
            match self.windows.get(&focused) {
                None => return Err(Broken::FocusUnknown(focused)),
                Some(window) => {
                    checks += 1;
                    if !window.is_visible(self.active_workspace) {
                        return Err(Broken::FocusHidden(focused));
                    }
                }
            }
        }

        // The history never names a window that has gone away.
        for id in &self.history {
            checks += 1;
            if !self.windows.contains_key(id) {
                return Err(Broken::HistoryUnknown(*id));
            }
        }

        Ok(checks)
    }

    /// Whether a window may take focus: managed, not minimised, on the shown
    /// workspace and on an output that still exists.
    fn is_focus_candidate(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.is_visible(self.active_workspace))
    }

    /// The output a new window belongs on: the focused window's output, or the
    /// primary output.
    fn placement_output(&self) -> Option<OutputId> {
        let focused = self
            .focus
            .and_then(|id| self.windows.get(&id))
            .and_then(|window| window.output)
            .filter(|output| self.output(*output).is_some());
        focused.or_else(|| self.outputs.first().map(Output::id))
    }

    /// Rectangles of the visible windows on one output and workspace.
    fn occupied(&self, output: OutputId, workspace: WorkspaceId, exclude: Option<WindowId>) -> Vec<Rect> {
        self.windows
            .values()
            .filter(|window| {
                window.output == Some(output)
                    && window.workspace == workspace
                    && !window.state.minimised
                    && Some(window.id) != exclude
            })
            .map(|window| window.geometry)
            .collect()
    }

    /// Records `id` as the most recently used window.
    fn remember_focus(&mut self, id: WindowId) {
        self.history.retain(|entry| *entry != id);
        self.history.insert(0, id);
        self.history.truncate(HISTORY_LIMIT);
    }

    /// Chooses focus again when the current window can no longer hold it.
    ///
    /// The most recently used window that is still visible wins, then the topmost
    /// visible window, then nothing. Nothing here can select a window that has
    /// closed, because closed windows are removed from the map and the history
    /// before this runs.
    fn refocus(&mut self) -> Vec<Effect> {
        let previous = self.focus;
        if previous.is_some_and(|id| self.is_focus_candidate(id)) {
            return Vec::new();
        }
        let candidate = self
            .history
            .iter()
            .copied()
            .find(|id| self.is_focus_candidate(*id))
            .or_else(|| {
                self.stack
                    .order()
                    .iter()
                    .copied()
                    .find(|id| self.is_focus_candidate(*id))
            });
        self.focus = candidate;
        if let Some(id) = candidate {
            self.remember_focus(id);
        }
        if previous == candidate {
            Vec::new()
        } else {
            vec![Effect::focus(previous, candidate)]
        }
    }

    /// A configure effect for a window's current state.
    fn configured(&self, id: WindowId) -> Option<Effect> {
        self.windows
            .get(&id)
            .map(|window| Effect::configure(id, window.geometry, window.state))
    }

    /// A configure effect for a window, from a rectangle the caller already has.
    fn configured_with(id: WindowId, geometry: Rect, state: WindowState) -> Vec<Effect> {
        vec![Effect::configure(id, geometry, state)]
    }

    /// A stacking effect, but only when the order actually changed.
    fn stack_changed(&self, before: &[WindowId]) -> Vec<Effect> {
        let order = self.stack.order();
        if order == before {
            Vec::new()
        } else {
            vec![Effect::stack(order.to_vec())]
        }
    }

    /// Connects an output and adopts any window that lost its output.
    ///
    /// A window can be left without an output when the last monitor is unplugged.
    /// Adopting it here is what makes the pair of events reversible: unplug, replug
    /// and the window is managed again instead of being stranded off every output.
    fn output_connected(&mut self, id: OutputId, rect: Rect) -> Vec<Effect> {
        if self.output(id).is_some() {
            return vec![Effect::rejected(Rejection::DuplicateOutput(id))];
        }
        self.outputs.push(Output::new(id, rect, Insets::ZERO));
        let Some(output) = self.output(id) else {
            return Vec::new();
        };
        let (work_area, monitor) = (output.work_area(), output.rect());
        let orphans: Vec<WindowId> = self
            .windows
            .values()
            .filter(|window| window.output.is_none())
            .map(|window| window.id)
            .collect();
        let mut effects = Vec::new();
        for window_id in orphans {
            let (geometry, state) = {
                let Some(window) = self.windows.get_mut(&window_id) else {
                    continue;
                };
                window.output = Some(id);
                window.restore = window.restore.clamp_inside(work_area);
                window.geometry = reshape(window.state, window.geometry, work_area, monitor);
                (window.geometry, window.state)
            };
            effects.push(Effect::visible(window_id, self.is_visible(window_id)));
            effects.push(Effect::configure(window_id, geometry, state));
        }
        if self.focus.is_none() {
            effects.extend(self.refocus());
        }
        effects
    }

    /// Retargets the windows of a removed output onto a surviving one.
    ///
    /// A window is never left pointing at an output that no longer exists: with a
    /// survivor it moves there and its rectangle is recomputed for its state, and
    /// without one it is unassigned and hidden until an output returns.
    fn output_disconnected(&mut self, id: OutputId) -> Vec<Effect> {
        if self.output(id).is_none() {
            return vec![Effect::rejected(Rejection::UnknownOutput(id))];
        }
        self.outputs.retain(|output| output.id() != id);
        let survivor = self
            .outputs
            .first()
            .map(|output| (output.id(), output.work_area(), output.rect()));
        let affected: Vec<WindowId> = self
            .windows
            .values()
            .filter(|window| window.output == Some(id))
            .map(|window| window.id)
            .collect();
        let mut effects = Vec::new();
        for window_id in affected {
            let (geometry, state, output) = {
                let Some(window) = self.windows.get_mut(&window_id) else {
                    continue;
                };
                match survivor {
                    Some((survivor_id, work_area, monitor)) => {
                        window.output = Some(survivor_id);
                        window.restore = window.restore.clamp_inside(work_area);
                        window.geometry = reshape(window.state, window.geometry, work_area, monitor);
                    }
                    None => window.output = None,
                }
                (window.geometry, window.state, window.output)
            };
            effects.push(Effect::visible(window_id, self.is_visible(window_id)));
            if output.is_some() {
                effects.push(Effect::configure(window_id, geometry, state));
            }
        }
        if self.focus.is_some() {
            effects.extend(self.refocus());
        }
        effects
    }

    /// Recomputes the rectangles of the windows on an output whose geometry changed.
    ///
    /// Maximised and snapped windows follow the new work area exactly, and floating
    /// windows are moved as little as possible to stay on it.
    fn output_layout_changed(&mut self, id: OutputId) -> Vec<Effect> {
        let Some(output) = self.output(id) else {
            return vec![Effect::rejected(Rejection::UnknownOutput(id))];
        };
        let (work_area, monitor) = (output.work_area(), output.rect());
        let affected: Vec<WindowId> = self
            .windows
            .values()
            .filter(|window| window.output == Some(id))
            .map(|window| window.id)
            .collect();
        let mut effects = Vec::new();
        for window_id in affected {
            let Some(window) = self.windows.get_mut(&window_id) else {
                continue;
            };
            window.restore = window.restore.clamp_inside(work_area);
            let geometry = reshape(window.state, window.geometry, work_area, monitor);
            if geometry != window.geometry {
                window.geometry = geometry;
                effects.push(Effect::configure(window_id, geometry, window.state));
            }
        }
        effects
    }

    fn window_opened(&mut self, spec: WindowSpec) -> Vec<Effect> {
        if self.windows.contains_key(&spec.id) {
            return vec![Effect::rejected(Rejection::DuplicateWindow(spec.id))];
        }
        let Some(workspace) = self.active_workspace else {
            return vec![Effect::rejected(Rejection::NoWorkspace)];
        };
        let Some(fallback_output) = self.placement_output() else {
            return vec![Effect::rejected(Rejection::NoOutput)];
        };
        let Some(fallback_area) = self.work_area(fallback_output) else {
            return vec![Effect::rejected(Rejection::NoOutput)];
        };
        // A client that asks for an empty extent still has to be placeable, so the
        // request is floored at one pixel exactly as `gnoblin-types` allows.
        let size = spec
            .requested
            .map_or(DEFAULT_WINDOW_SIZE, |rect| rect.size())
            .at_least_one();
        let parent = spec
            .transient_for
            .filter(|parent| *parent != spec.id)
            .and_then(|parent| {
                self.windows
                    .get(&parent)
                    .map(|window| (parent, window.workspace, window.output, window.geometry))
            });
        let (workspace, output, geometry, anchor) = match parent {
            Some((parent_id, parent_workspace, parent_output, parent_geometry)) => {
                // A transient stays with its parent: same workspace, same output,
                // centred over it, and above it in the stack.
                let output = parent_output
                    .filter(|output| self.output(*output).is_some())
                    .or(Some(fallback_output));
                let area = output
                    .and_then(|output| self.work_area(output))
                    .unwrap_or(fallback_area);
                let offset_x = i64::from(parent_geometry.width.saturating_sub(size.width) / 2);
                let offset_y = i64::from(parent_geometry.height.saturating_sub(size.height) / 2);
                let geometry = Rect::new(
                    clamp_to_i32(i64::from(parent_geometry.x) + offset_x),
                    clamp_to_i32(i64::from(parent_geometry.y) + offset_y),
                    size.width,
                    size.height,
                )
                .clamp_inside(area);
                (parent_workspace, output, geometry, Some(parent_id))
            }
            None => {
                let occupied = self.occupied(fallback_output, workspace, None);
                let geometry = placement::place(self.options.placement, size, fallback_area, &occupied);
                (workspace, Some(fallback_output), geometry, None)
            }
        };
        let mut window = Window {
            id: spec.id,
            app_id: spec.app_id,
            title: spec.title,
            kind: spec.kind,
            transient_for: anchor,
            workspace,
            output,
            geometry,
            restore: geometry,
            state: WindowState::default(),
            applied_rules: 0,
        };
        let mut effects = Vec::new();
        if !window.kind.is_transient() {
            effects.extend(self.apply_rules(&mut window));
        }
        let layer = layer_of(window.state);
        let window_id = window.id;
        self.windows.insert(window_id, window);
        let before = self.stack.order().to_vec();
        self.stack.insert(window_id, layer, anchor);
        if let Some(effect) = self.configured(window_id) {
            effects.push(effect);
        }
        effects.push(Effect::visible(window_id, self.is_visible(window_id)));
        effects.extend(self.stack_changed(&before));
        effects
    }

    /// Applies the configured rules to a window that is being taken into management.
    ///
    /// This is the only place rules run. Later rules override the fields they set,
    /// so the list is walked to the end, and a rule whose workspace target does not
    /// exist leaves the window where it is and reports why.
    fn apply_rules(&self, window: &mut Window) -> Vec<Effect> {
        let mut effects = Vec::new();
        for (index, rule) in self.rules.iter().enumerate() {
            if !rule.matches(window) {
                continue;
            }
            let mut rejected = false;
            for action in &rule.actions {
                match action {
                    RuleAction::PlaceOnWorkspace(selector) => match self.workspaces.resolve(*selector) {
                        Some(workspace) => window.workspace = workspace,
                        None => {
                            rejected = true;
                            effects.push(Effect::rule_rejected(
                                window.id,
                                index,
                                RuleRejection::MissingWorkspace(*selector),
                            ));
                        }
                    },
                    RuleAction::Maximise => {
                        let Some(area) = window.output.and_then(|output| self.work_area(output)) else {
                            continue;
                        };
                        if !window.state.maximised && window.state.snapped.is_none() {
                            window.restore = window.geometry;
                        }
                        window.state.maximised = true;
                        window.state.snapped = None;
                        window.geometry = area;
                    }
                }
            }
            // A rule that could not be carried out completely is not counted as
            // applied, so the counter never claims work that did not happen.
            if !rejected {
                window.applied_rules += 1;
            }
        }
        effects
    }

    fn window_closed(&mut self, id: WindowId) -> Vec<Effect> {
        if self.windows.remove(&id).is_none() {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        }
        // A dialog whose parent has gone keeps working as its own window, so drop the
        // dangling parent link. Keeping it would let a reused window id look like the
        // parent of an unrelated dialog.
        for window in self.windows.values_mut() {
            if window.transient_for == Some(id) {
                window.transient_for = None;
            }
        }
        let before = self.stack.order().to_vec();
        self.stack.remove(id);
        self.history.retain(|entry| *entry != id);
        let mut effects = self.stack_changed(&before);
        if self.focus == Some(id) {
            effects.extend(self.refocus());
        }
        effects
    }

    fn window_title_changed(&mut self, id: WindowId, title: String) -> Vec<Effect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        // Rules do not run again: placement and maximise are one-shot decisions.
        window.title = title;
        Vec::new()
    }

    fn window_app_id_changed(&mut self, id: WindowId, app_id: String) -> Vec<Effect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        window.app_id = app_id;
        Vec::new()
    }

    fn workspace_added(&mut self, id: WorkspaceId) -> Vec<Effect> {
        if !self.workspaces.add(id) {
            return vec![Effect::rejected(Rejection::DuplicateWorkspace(id))];
        }
        if self.active_workspace.is_none() {
            self.active_workspace = Some(id);
        }
        Vec::new()
    }

    fn workspace_removed(&mut self, id: WorkspaceId) -> Vec<Effect> {
        if !self.workspaces.contains(id) {
            return vec![Effect::rejected(Rejection::UnknownWorkspace(id))];
        }
        let Some(neighbour) = self.workspaces.neighbour_of(id) else {
            return vec![Effect::rejected(Rejection::LastWorkspace(id))];
        };
        let was_active = self.active_workspace == Some(id);
        self.workspaces.remove(id);
        let moved: Vec<WindowId> = self
            .windows
            .values()
            .filter(|window| window.workspace == id)
            .map(|window| window.id)
            .collect();
        for window_id in &moved {
            if let Some(window) = self.windows.get_mut(window_id) {
                window.workspace = neighbour;
            }
        }
        let mut effects = Vec::new();
        if was_active {
            self.active_workspace = Some(neighbour);
            effects.push(Effect::workspace_shown(neighbour));
        }
        for window_id in moved {
            effects.push(Effect::visible(window_id, self.is_visible(window_id)));
        }
        if was_active {
            effects.extend(self.refocus());
        }
        effects
    }

    fn workspace_activated(&mut self, id: WorkspaceId) -> Vec<Effect> {
        if !self.workspaces.contains(id) {
            return vec![Effect::rejected(Rejection::UnknownWorkspace(id))];
        }
        if self.active_workspace == Some(id) {
            return Vec::new();
        }
        let before: Vec<(WindowId, bool)> = self
            .windows
            .keys()
            .map(|window| (*window, self.is_visible(*window)))
            .collect();
        self.active_workspace = Some(id);
        let mut effects = vec![Effect::workspace_shown(id)];
        for (window, was_visible) in before {
            if self.is_visible(window) != was_visible {
                effects.push(Effect::visible(window, !was_visible));
            }
        }
        effects.extend(self.refocus());
        effects
    }

    fn activation(&mut self, id: WindowId, decision: ActivationDecision) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if !decision.permits_focus() {
            return vec![Effect::ActivationDenied { window: id }];
        }
        if !window.is_visible(self.active_workspace) {
            return vec![Effect::rejected(Rejection::NotVisible(id))];
        }
        let previous = self.focus;
        self.focus = Some(id);
        self.remember_focus(id);
        let mut effects = Vec::new();
        if decision.raises() {
            let before = self.stack.order().to_vec();
            self.stack.raise(id);
            effects.extend(self.stack_changed(&before));
        }
        if previous != Some(id) {
            effects.push(Effect::focus(previous, Some(id)));
        }
        effects
    }

    fn raise(&mut self, id: WindowId) -> Vec<Effect> {
        if !self.windows.contains_key(&id) {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        }
        let before = self.stack.order().to_vec();
        self.stack.raise(id);
        self.stack_changed(&before)
    }

    fn lower(&mut self, id: WindowId) -> Vec<Effect> {
        if !self.windows.contains_key(&id) {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        }
        let before = self.stack.order().to_vec();
        self.stack.lower(id);
        self.stack_changed(&before)
    }

    fn minimise(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.state.minimised {
            return Vec::new();
        }
        window.state.minimised = true;
        let mut effects = vec![Effect::visible(id, false)];
        if self.focus == Some(id) {
            effects.extend(self.refocus());
        }
        effects
    }

    fn unminimise(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if !window.state.minimised {
            return Vec::new();
        }
        window.state.minimised = false;
        let mut effects = vec![Effect::visible(id, self.is_visible(id))];
        if self.focus.is_none() {
            effects.extend(self.refocus());
        }
        effects
    }

    fn toggle_maximise(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.state.fullscreen {
            return vec![Effect::rejected(Rejection::NotFloating(id))];
        }
        let Some(work_area) = window.output.and_then(|output| self.work_area(output)) else {
            return vec![Effect::rejected(Rejection::NoOutput)];
        };
        let (state, geometry, restore) = (window.state, window.geometry, window.restore);
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        if state.maximised {
            window.state.maximised = false;
            window.geometry = restore.clamp_inside(work_area);
        } else {
            if state.snapped.is_none() {
                window.restore = geometry;
            }
            window.state.maximised = true;
            window.state.snapped = None;
            window.geometry = work_area;
        }
        Self::configured_with(id, window.geometry, window.state)
    }

    fn toggle_fullscreen(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        let Some(output) = window.output.and_then(|output| self.output(output)) else {
            return vec![Effect::rejected(Rejection::NoOutput)];
        };
        let (work_area, monitor) = (output.work_area(), output.rect());
        let (state, geometry) = (window.state, window.geometry);
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        if state.fullscreen {
            window.state.fullscreen = false;
            window.geometry = destination(window.state, window.restore, work_area);
        } else {
            // Only a floating window needs its rectangle remembered; a maximised or
            // snapped window already keeps the rectangle to return to.
            if !state.maximised && state.snapped.is_none() {
                window.restore = geometry;
            }
            window.state.fullscreen = true;
            window.geometry = monitor;
        }
        self.stack.set_layer(id, layer_of(window.state));
        Self::configured_with(id, window.geometry, window.state)
    }

    fn snap(&mut self, id: WindowId, zone: SnapZone) -> Vec<Effect> {
        if !self.options.edge_tiling {
            return vec![Effect::rejected(Rejection::EdgeTilingDisabled)];
        }
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.state.fullscreen {
            return vec![Effect::rejected(Rejection::NotFloating(id))];
        }
        let Some(work_area) = window.output.and_then(|output| self.work_area(output)) else {
            return vec![Effect::rejected(Rejection::NoOutput)];
        };
        let (state, geometry) = (window.state, window.geometry);
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        if !state.maximised && state.snapped.is_none() {
            window.restore = geometry;
        }
        window.state.maximised = false;
        window.state.snapped = Some(zone);
        window.geometry = zone.rect(work_area);
        Self::configured_with(id, window.geometry, window.state)
    }

    fn unsnap(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.state.fullscreen {
            // Fullscreen owns the rectangle; leaving the snap now would move the
            // window out from under the fullscreen state.
            return vec![Effect::rejected(Rejection::NotFloating(id))];
        }
        if window.state.snapped.is_none() {
            return Vec::new();
        }
        let restore = window.restore;
        let work_area = window.output.and_then(|output| self.work_area(output));
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        window.state.snapped = None;
        if let Some(work_area) = work_area {
            window.geometry = restore.clamp_inside(work_area);
        }
        Self::configured_with(id, window.geometry, window.state)
    }

    fn move_resize(&mut self, id: WindowId, geometry: Rect) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.state.fullscreen {
            return vec![Effect::rejected(Rejection::NotFloating(id))];
        }
        // A resize can ask for an empty extent; the window keeps a pixel so the
        // user can still grab it again.
        let geometry = geometry.at_least_one();
        let work_area = window.output.and_then(|output| self.work_area(output));
        let constraint = self.options.drag_constraint;
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        let geometry = match work_area {
            Some(work_area) => drag::constrain(geometry, work_area, constraint),
            None => geometry,
        };
        // Dragging a maximised or snapped window floats it: the drag is the user
        // asking for a free rectangle.
        window.state.maximised = false;
        window.state.snapped = None;
        window.geometry = geometry;
        window.restore = geometry;
        Self::configured_with(id, window.geometry, window.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_ignores_invisible_windows_of_other_workspaces() {
        // A window on another workspace is not in the way of a new window, because
        // it is not on screen.
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(2),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        wm.apply(Event::WindowOpened(
            WindowSpec::normal(WindowId::new(1), "org.example.One").with_requested(Rect::new(0, 0, 400, 300)),
        ));
        wm.apply(Event::ActivationRequested {
            window: WindowId::new(1),
            decision: ActivationDecision::Allow,
        });
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(2),
        });
        wm.apply(Event::WindowOpened(
            WindowSpec::normal(WindowId::new(2), "org.example.Two").with_requested(Rect::new(0, 0, 400, 300)),
        ));
        let Some(second) = wm.window(WindowId::new(2)) else {
            panic!("the second window is managed");
        };
        assert_eq!(second.geometry, Rect::new(0, 0, 400, 300));
    }

    #[test]
    fn the_window_count_matches_the_stack_after_a_full_lifecycle() {
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        for raw in 1..=3 {
            wm.apply(Event::WindowOpened(WindowSpec::normal(
                WindowId::new(raw),
                "org.example.App",
            )));
        }
        assert_eq!(wm.windows().count(), 3);
        assert_eq!(wm.stack().len(), 3);
        wm.apply(Event::WindowClosed {
            window: WindowId::new(2),
        });
        assert_eq!(wm.windows().count(), 2);
        assert_eq!(wm.stack().len(), 2);
        assert_eq!(wm.check_invariants().map(|_| ()), Ok(()));
    }
}
