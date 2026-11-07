//! The window management state machine: events in, effects out.
//!
//! [`Wm`] holds every fact the policy needs: the outputs and their work areas, the
//! declared workspaces and which one is active, each managed [`Window`] with its
//! geometry and [`WindowState`], the stack, the focus and its history, and the
//! pointer-focus timer. [`Wm::apply`] is the only way to change any of it. It takes
//! one [`Event`], mutates the state in one step and returns the [`Effect`]s the
//! caller must carry out. The readers only borrow state, and nothing reads a
//! clock: time arrives as a [`Tick`] on the events that need it.
//!
//! # Window states
//!
//! A window's state is a set of flags in [`WindowState`], not one enum, because the
//! flags compose. A window can be minimised while fullscreen, and fullscreen can
//! sit over maximise or a snap, which it returns to when it leaves fullscreen.
//! Maximising a snapped window clears its snap. Each transition has one of three
//! shapes:
//!
//! - **Enter a state** (maximise, fullscreen, snap) remembers the floating
//!   rectangle in `restore` when the window is not already snapped (and, for
//!   fullscreen, not maximised), then moves the window to the state's rectangle.
//!   Minimise changes no geometry.
//! - **Leave a state** (unmaximise, unfullscreen, unsnap) recomputes the rectangle
//!   from the remaining flags through [`destination`], so the window lands where its
//!   other states say, not where it was before the toggle.
//! - **Change the work area or output** recomputes every rectangle through
//!   [`reshape`], which applies the same precedence: fullscreen covers the monitor,
//!   maximise covers the work area, a snap takes its zone, and a floating window is
//!   clamped inside the work area.
//!
//! That precedence is why a fullscreen window survives a workspace switch and a
//! snapped window follows its zone when a panel changes its reservation.
//!
//! # Focus and stacking
//!
//! Focus is one window or none. When the focused window is minimised or closed,
//! focus moves to the most recently used window in the history that is still
//! visible, so focus does not land on a window the user cannot see. The stack
//! ([`Stack`]) is updated by the same events. Fullscreen windows sit in the
//! fullscreen layer above all normal windows, and [`layer_of`] is the only place a
//! state chooses a layer.
//!
//! # Layouts
//!
//! Layouts never run inside [`Wm::apply`]. A state change submits layout work, the
//! result is collected on a later [`Event::Frame`] with a deadline, and a slow or
//! wedged engine leaves the last good placements in force rather than blocking the
//! frame. [`crate::arrange`] owns that protocol. This module only applies settled
//! results.
//!
//! # Units
//!
//! Rectangles, sizes and insets are logical pixels in the compositor's global
//! space, the units `gnoblin-types` uses. Physical pixels are a renderer concern
//! and never reach this module. Time is a [`Tick`], one millisecond of the caller's
//! clock. `auto_raise_delay` is in milliseconds, the same unit.

use std::collections::BTreeMap;
use std::time::Instant;

use gnoblin_layers::{Band, LayerHandle, LayerProvider, LayerRegistry, LayerScope, LayerSpec, LocalName};
use gnoblin_types::{Direction, Insets, OutputId, Rect, RectF, Size, Tick, WindowId, WorkspaceId};

use crate::activation::{ActivationDecision, NewWindowFocus};
use crate::arrange::{self, Layouts};
use crate::drag::{self, DragConstraint};
use crate::effect::{Effect, RuleRejection};
use crate::event::{Event, Rejection, WindowSpec};
use crate::geometry::clamp_to_i32;
use crate::layout::{
    Layout, LayoutCommand, LayoutEffect, LayoutEngine, LayoutKind, Placement, RequestKind, Transform3, WindowPlacement,
};
use crate::output::Output;
use crate::placement::{self, PlacementMode};
use crate::pointer_focus::{self, FocusMode, PointerFocus};
use crate::rules::{Rule, RuleAction};
use crate::snapping::SnapZone;
use crate::stack::{Stack, StackLayer};
use crate::tiled::{TiledEdges, tiled_edges};
use crate::window::{Window, WindowKind, WindowState};
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
    /// `window_management.focus_new_windows`.
    pub new_window_focus: NewWindowFocus,
    /// `window_management.focus_mode`.
    pub focus_mode: FocusMode,
    /// `window_management.focus_change_on_pointer_rest`.
    pub focus_change_on_pointer_rest: bool,
    /// `window_management.raise_on_click`.
    pub raise_on_click: bool,
    /// `window_management.auto_raise`.
    pub auto_raise: bool,
    /// `window_management.auto_raise_delay`, in milliseconds.
    pub auto_raise_delay: u64,
}

impl Default for WmOptions {
    /// The compositor defaults from the configuration reference, which are the
    /// values a session gets when its configuration omits the keys.
    fn default() -> Self {
        Self {
            placement: PlacementMode::FreeSpace,
            edge_tiling: false,
            drag_constraint: DragConstraint::WorkArea,
            new_window_focus: NewWindowFocus::Prevent,
            focus_mode: FocusMode::Click,
            focus_change_on_pointer_rest: false,
            raise_on_click: true,
            auto_raise: false,
            auto_raise_delay: 500,
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
/// Why a layout command was not sent to a layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandError {
    /// The workspace has no layout engine.
    UnknownLayout,
    /// The layout does not know the command's name.
    Refused,
}

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
#[derive(Debug, Default)]
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
    layouts: Layouts,
    /// Events held back until the next [`Event::Frame`] because a layout is
    /// installed and the event either changes tiling or follows one that does.
    /// They are applied in arrival order so that layouts see every tiling change.
    deferred: Vec<Event>,
    /// Pointer position and any rest focus waiting on it.
    pointer: PointerFocus,
    /// A window the pointer focused, which `auto_raise` raises at the given tick.
    auto_raise: Option<(WindowId, Tick)>,
    /// The layer records that mirror managed windows, one `surface_toplevel` per
    /// window. `window_opened` adds each record and `window_closed` ends it.
    layers: LayerRegistry,
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

    /// The layer registry: every live layer the policy mirrors, including one
    /// `gnoblin.surface_toplevel:<id>` record per managed window.
    ///
    /// The registry is read-only from outside, so no second writer can change a
    /// record without the window it mirrors.
    #[must_use]
    pub fn layers(&self) -> &LayerRegistry {
        &self.layers
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
        if !self.layouts.is_enabled() {
            let mut effects = self.apply_event(event);
            effects.extend(self.sync_layouts());
            return effects;
        }
        if let Event::Frame { deadline } = event {
            return self.collect_layouts(deadline);
        }
        if self.deferred.is_empty() && !changes_tiling(&event) {
            let mut effects = self.apply_event(event);
            effects.extend(self.sync_layouts());
            return effects;
        }
        self.deferred.push(event);
        Vec::new()
    }

    /// Applies one event, then brings window layers in line with the new state.
    ///
    /// Every path that changes which windows are drawn goes through here, so a
    /// layer cannot drift from its window however the change arrived.
    fn apply_event(&mut self, event: Event) -> Vec<Effect> {
        let effects = self.dispatch(event);
        self.sync_layer_visibility();
        effects
    }

    /// Routes one event to the handler that owns it.
    fn dispatch(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::OutputConnected { output, rect } => self.output_connected(output, rect),
            Event::Frame { deadline } => self.collect_layouts(deadline),
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
            Event::OutputIdentified { output, name, scale } => {
                match self.outputs.iter_mut().find(|entry| entry.id() == output) {
                    Some(entry) => {
                        entry.set_identity(name, scale);
                        self.output_layout_changed(output)
                    }
                    None => vec![Effect::rejected(Rejection::UnknownOutput(output))],
                }
            }
            Event::WindowOpened(spec) => self.window_opened(spec),
            Event::WindowClosed { window } => self.window_closed(window),
            Event::WindowTitleChanged { window, title } => self.window_title_changed(window, title),
            Event::WindowAppIdChanged { window, app_id } => self.window_app_id_changed(window, app_id),
            Event::WindowRoleChanged {
                window,
                kind,
                transient_for,
            } => self.window_role_changed(window, kind, transient_for),
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
            Event::PointerMoved { window, at } => self.pointer_moved(window, at),
            Event::PointerTick { at } => self.pointer_due(at),
            Event::PointerPressed { window } => self.pointer_pressed(window),
            Event::PointerGrab { active } => {
                self.pointer.set_grab(active);
                if active {
                    self.auto_raise = None;
                }
                Vec::new()
            }
            Event::Raise { window } => self.raise(window),
            Event::Lower { window } => self.lower(window),
            Event::Minimise { window } => self.minimise(window),
            Event::Unminimise { window } => self.unminimise(window),
            Event::ToggleMaximise { window } => self.toggle_maximise(window),
            Event::ToggleFullscreen { window } => self.toggle_fullscreen(window),
            Event::SnapRequested { window, zone } => self.snap(window, zone),
            Event::ToggleFloat { window } => self.toggle_float(window),
            Event::Unsnap { window } => self.unsnap(window),
            Event::MoveResize { window, geometry } => self.move_resize(window, geometry),
            Event::FocusDirection { direction } => self.focus_direction(direction),
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

    /// Returns the live window-scope layers, bottom to top, grouped by the window
    /// they follow in stacking order.
    ///
    /// The registry orders layers within one window through `paint_order`, but it
    /// does not know where the window sits in the stack. This walks the stack from
    /// the bottom and emits each window's layers in the registry's order, so a
    /// layer always paints with the window it follows. Layers that follow no
    /// window are left out.
    #[must_use]
    pub fn layer_order(&self) -> Vec<LayerHandle> {
        let painted = gnoblin_layers::paint_order(&self.layers);
        let mut order = Vec::with_capacity(painted.len());
        for &window in self.stack.order().iter().rev() {
            let scope = LayerScope::Window(Some(window));
            order.extend(
                painted
                    .iter()
                    .copied()
                    .filter(|&handle| self.layers.get(handle).is_some_and(|record| record.spec.scope == scope)),
            );
        }
        order
    }

    /// Brings every window's layers in line with whether the window is drawn now.
    ///
    /// A layer follows its window: a minimised window, or one on another
    /// workspace, hides its layers, and restoring it shows them again. Ended
    /// layers are not touched, and layers already in the right state are left
    /// alone, so running this after every event never repeats a change.
    fn sync_layer_visibility(&mut self) {
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in windows {
            let visible = self.is_visible(id);
            self.layers.set_target_visible(id, visible);
        }
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

    /// The topmost visible modal child of `parent`. While it is open the parent cannot
    /// take focus: the modal does.
    fn blocking_modal(&self, parent: WindowId) -> Option<WindowId> {
        self.stack.order().iter().copied().find(|id| {
            self.windows.get(id).is_some_and(|window| {
                window.kind == WindowKind::Modal
                    && window.transient_for == Some(parent)
                    && window.is_visible(self.active_workspace)
            })
        })
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
        let candidate = candidate.map(|id| self.blocking_modal(id).unwrap_or(id));
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
        self.layouts.output_connected(id);
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
        self.layouts
            .output_disconnected(id, survivor.map(|(output, _, _)| output));
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

    /// Registers the `gnoblin.surface_toplevel:<id>` record for a window, so layer
    /// tooling sees every window as a layer in the `windows` band.
    ///
    /// The record is a mirror, not a second source of truth: the stack still
    /// decides where the window paints. The name is always well formed and the id
    /// is unique in `windows`, so the registry refusing it is an invariant break.
    /// That is reported in debug builds; in release the window is still managed
    /// without its record rather than being dropped.
    fn register_surface_record(&mut self, id: WindowId) {
        let Ok(local) = LocalName::keyed("surface_toplevel", &id.get().to_string()) else {
            debug_assert!(false, "a numeric window id always forms a valid local name");
            return;
        };
        let spec = LayerSpec::new(local, LayerScope::Window(Some(id)), Band::Windows);
        let result = self.layers.register(&LayerProvider::Gnoblin("gnoblin-wm"), spec);
        debug_assert!(result.is_ok(), "surface record for {id} refused: {result:?}");
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
        // A transient always floats: it is placed relative to its parent and
        // is never part of a workspace's tiled set. Any other window takes the
        // placement its workspace's layout gives.
        let placement = self.default_placement(workspace, spec.kind.is_transient());
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
            placement,
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
        self.register_surface_record(window_id);
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
        self.pointer.forget(id);
        if self.auto_raise.is_some_and(|(armed, _)| armed == id) {
            self.auto_raise = None;
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
        // Ending every layer that targets the window also ends its surface record, so
        // a reused window id starts with a clean name.
        self.layers.end_target(id);
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

    /// Applies a role change: the kind and parent of a window. A transient follows its
    /// parent on the same workspace and always floats, so a window that becomes one
    /// leaves the tiled set.
    fn window_role_changed(&mut self, id: WindowId, kind: WindowKind, transient_for: Option<WindowId>) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        let parent = transient_for.filter(|parent| *parent != id);
        if let Some(parent) = parent {
            match self.windows.get(&parent) {
                None => return vec![Effect::rejected(Rejection::UnknownWindow(parent))],
                Some(parent_window) if parent_window.workspace != window.workspace => {
                    return vec![Effect::rejected(Rejection::ParentOnOtherWorkspace {
                        window: id,
                        parent,
                    })];
                }
                Some(_) => {}
            }
        }
        let needs_free = kind.is_transient() && window.placement == WindowPlacement::Managed;
        let parent_changed = window.transient_for != parent;
        let Some(window) = self.windows.get_mut(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        window.kind = kind;
        window.transient_for = parent;
        let mut effects = if needs_free {
            self.set_placement(id, WindowPlacement::Free)
        } else {
            Vec::new()
        };
        if parent.is_some() && parent_changed {
            // A transient opens above its parent, so a window that becomes one is stacked
            // above the parent in its layer, as a window opened with a parent is.
            let before = self.stack.order().to_vec();
            self.stack.raise(id);
            effects.extend(self.stack_changed(&before));
        }
        // A modal that appears while its parent holds focus takes focus from it.
        if let Some(parent) = parent
            && kind == WindowKind::Modal
            && self.focus == Some(parent)
            && self.is_focus_candidate(id)
        {
            effects.extend(self.focus_window(id, ActivationDecision::Allow));
        }
        effects
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

    /// The decision for a window that asks to be focused on its own initiative, as
    /// a new window does when it maps. Under `prevent` a new window may take focus
    /// only when nothing is focused yet, or when it is a transient of the focused
    /// window; otherwise it would steal focus from a window that has it. Under
    /// `allow` any window may.
    #[must_use]
    pub fn new_window_decision(&self, id: WindowId) -> ActivationDecision {
        let Some(window) = self.windows.get(&id) else {
            return ActivationDecision::Deny;
        };
        match self.options.new_window_focus {
            NewWindowFocus::Allow => ActivationDecision::Allow,
            NewWindowFocus::Prevent => match window.transient_for {
                _ if self.focus.is_none() => ActivationDecision::Allow,
                Some(parent) if self.focus == Some(parent) => ActivationDecision::Allow,
                _ => ActivationDecision::Deny,
            },
        }
    }

    fn activation(&mut self, id: WindowId, decision: ActivationDecision) -> Vec<Effect> {
        if !self.windows.contains_key(&id) {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        }
        if !decision.permits_focus() {
            return vec![Effect::ActivationDenied { window: id }];
        }
        // A parent with an open modal child cannot take focus: the modal takes it instead.
        let target = self.blocking_modal(id).unwrap_or(id);
        self.focus_window(target, decision)
    }

    /// The pointer moved over `window` (or a gap). Focus follows it as `focus_mode` asks.
    fn pointer_moved(&mut self, window: Option<WindowId>, at: Tick) -> Vec<Effect> {
        // Leaving the window an auto-raise is waiting on cancels the raise.
        if self.auto_raise.is_some_and(|(armed, _)| window != Some(armed)) {
            self.auto_raise = None;
        }
        let action = self.pointer.moved(
            window,
            at,
            self.focus,
            self.options.focus_mode,
            self.options.focus_change_on_pointer_rest,
        );
        self.pointer_effects(action, at)
    }

    /// The clock reached `at`. A rest focus whose time has come takes effect, and an
    /// auto-raise whose delay has run out raises the focused window if the pointer is
    /// still on it.
    fn pointer_due(&mut self, at: Tick) -> Vec<Effect> {
        let action = self.pointer.due(at);
        let mut effects = self.pointer_effects(action, at);
        if let Some((id, _)) = self.auto_raise.filter(|(_, due)| at.reached(*due)) {
            self.auto_raise = None;
            let focused_and_under = self.focus == Some(id) && self.pointer.over() == Some(id);
            let visible = self
                .windows
                .get(&id)
                .is_some_and(|window| window.is_visible(self.active_workspace));
            if focused_and_under && visible {
                effects.extend(self.raise(id));
            }
        }
        effects
    }

    /// A button went down over `window`. It is focused, and raised when `raise_on_click`
    /// is set; otherwise its stacking is left alone.
    fn pointer_pressed(&mut self, window: WindowId) -> Vec<Effect> {
        let decision = if self.options.raise_on_click {
            ActivationDecision::Allow
        } else {
            ActivationDecision::AllowWithoutRaise
        };
        self.activation(window, decision)
    }

    fn pointer_effects(&mut self, action: pointer_focus::PointerAction, at: Tick) -> Vec<Effect> {
        use pointer_focus::PointerAction;
        match action {
            PointerAction::None => Vec::new(),
            PointerAction::Focus(id) => {
                let effects = self.activation(id, ActivationDecision::AllowWithoutRaise);
                if self.options.auto_raise && self.focus == Some(id) {
                    self.auto_raise = Some((id, at.saturating_add(self.options.auto_raise_delay)));
                }
                effects
            }
            PointerAction::Clear => {
                self.auto_raise = None;
                let previous = self.focus.take();
                if previous.is_some() {
                    vec![Effect::focus(previous, None)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Focuses a visible window, raising it when the decision allows. The caller has
    /// already chosen the window, including any modal that stands in for a parent.
    fn focus_window(&mut self, id: WindowId, decision: ActivationDecision) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if !window.is_visible(self.active_workspace) {
            return vec![Effect::rejected(Rejection::NotVisible(id))];
        }
        let previous = self.focus;
        self.focus = Some(id);
        // Focus moving to another window cancels an auto-raise waiting on the last one.
        if self.auto_raise.is_some_and(|(armed, _)| armed != id) {
            self.auto_raise = None;
        }
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

    /// The placement a window takes when it enters `workspace`: the workspace's
    /// layout decides, and a window with no layout keeps the placement it had before
    /// layouts existed (managed, or free for a transient).
    fn default_placement(&self, workspace: WorkspaceId, transient: bool) -> WindowPlacement {
        match self.layouts.kind(workspace) {
            Some(kind) => kind.default_placement(transient),
            None if transient => WindowPlacement::Free,
            None => WindowPlacement::Managed,
        }
    }

    /// Flips a window between managed and free placement, as the user asks. The
    /// layout of its workspace must allow it, and a transient never becomes managed.
    fn toggle_float(&mut self, id: WindowId) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.kind.is_transient() {
            return vec![Effect::rejected(Rejection::TransientAlwaysFloating(id))];
        }
        if self
            .layouts
            .kind(window.workspace)
            .is_some_and(|kind| !kind.allows_toggle_float())
        {
            return vec![Effect::rejected(Rejection::NoFloatToggle(id))];
        }
        let placement = match window.placement {
            WindowPlacement::Free => WindowPlacement::Managed,
            WindowPlacement::Managed => WindowPlacement::Free,
        };
        self.set_placement(id, placement)
    }

    /// Sets a window's placement. Free placement saves the rectangle the window has, so
    /// a later return to managed restores it. Maximised, snapped and fullscreen windows
    /// keep their stronger state; only the placement changes. A transient is always free.
    fn set_placement(&mut self, id: WindowId, placement: WindowPlacement) -> Vec<Effect> {
        let Some(window) = self.windows.get(&id) else {
            return vec![Effect::rejected(Rejection::UnknownWindow(id))];
        };
        if window.kind.is_transient() && placement == WindowPlacement::Managed {
            return vec![Effect::rejected(Rejection::TransientAlwaysFloating(id))];
        }
        if window.placement == placement {
            return Vec::new();
        }
        let (state, geometry, restore) = (window.state, window.geometry, window.restore);
        let work_area = window.output.and_then(|output| self.work_area(output));
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        window.placement = placement;
        if placement == WindowPlacement::Free {
            if !state.maximised && state.snapped.is_none() && !state.fullscreen {
                window.restore = geometry;
            }
        } else if let Some(work_area) =
            work_area.filter(|_| !state.maximised && state.snapped.is_none() && !state.fullscreen)
        {
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

    /// Installs `factory` as the source of one layout engine per workspace, called
    /// with the workspace each engine is for, and submits the arrangement of every
    /// workspace. Placements arrive on the next [`Event::Frame`]; the returned
    /// effects only report input that was refused.
    pub fn set_layout_engines(
        &mut self,
        factory: impl FnMut(WorkspaceId, Option<&str>) -> Box<dyn LayoutEngine> + 'static,
    ) -> Vec<Effect> {
        self.layouts.install(Box::new(factory));
        self.sync_layouts()
    }

    /// Replaces one workspace's engine with a fresh one from the installed factory and
    /// submits its arrangement. The old engine is dropped, and the windows of the
    /// workspace take the placements of the new engine's kind. Does nothing when no
    /// factory is installed or the workspace is not declared.
    pub fn rebuild_layout(&mut self, workspace: WorkspaceId) -> Vec<Effect> {
        self.layouts.forget(workspace);
        self.sync_layouts()
    }

    /// The connector name of the output every workspace is arranged on, when it is known.
    /// A layout's output override is keyed by this name.
    #[must_use]
    pub fn arrange_output_name(&self) -> Option<&str> {
        self.outputs.first().map(Output::name).filter(|name| !name.is_empty())
    }

    /// Replaces every workspace's engine with one from the installed factory and
    /// submits the arrangements. Used when an output's default layout changes.
    pub fn rebuild_all_layouts(&mut self) -> Vec<Effect> {
        self.layouts.forget_all();
        self.sync_layouts()
    }

    /// Sends a named command to `workspace`'s layout and applies the arrangement
    /// it returns, waiting until `deadline`. A failed run is reported as
    /// [`Effect::LayoutFailed`], as a failed arrangement is. Returns
    /// [`CommandError::UnknownLayout`] when the workspace has no layout engine and
    /// [`CommandError::Refused`] when the layout does not know `name`.
    pub fn layout_command(
        &mut self,
        workspace: WorkspaceId,
        command: LayoutCommand,
        deadline: Instant,
    ) -> Result<Vec<Effect>, CommandError> {
        let (windows, outputs) = (&self.windows, &self.outputs);
        let verdict = self.layouts.command(workspace, command, deadline, |output| {
            arrange::tiled_params(workspace, output, windows, outputs)
        });
        match verdict {
            arrange::CommandVerdict::Handled(layout) => Ok(self.apply_layout(workspace, layout)),
            arrange::CommandVerdict::Failed(failure) => Ok(vec![Effect::LayoutFailed {
                workspace,
                reason: failure.reason(),
            }]),
            arrange::CommandVerdict::InvalidParams => Ok(vec![Effect::LayoutFailed {
                workspace,
                reason: "invalid-params",
            }]),
            arrange::CommandVerdict::Idle => Ok(Vec::new()),
            arrange::CommandVerdict::Declined => Err(CommandError::Refused),
            arrange::CommandVerdict::NoLayout => Err(CommandError::UnknownLayout),
        }
    }

    /// Submits a run for every workspace whose tiled input changed since its last
    /// submit. A layout that is created, or whose kind changed, first sets the
    /// placements of its workspace's windows to the kind's default. Does nothing when
    /// no factory is installed.
    fn sync_layouts(&mut self) -> Vec<Effect> {
        if !self.layouts.is_enabled() {
            return Vec::new();
        }
        let first_output = self.outputs.first().map(Output::id);
        let outputs = &self.outputs;
        let output_name = |output: OutputId| {
            outputs
                .iter()
                .find(|entry| entry.id() == output)
                .map(|entry| entry.name())
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
        };
        let changed = self.layouts.rebuild(self.workspaces.ids(), first_output, output_name);
        for (workspace, kind) in changed {
            for window in self.windows.values_mut().filter(|window| window.workspace == workspace) {
                window.placement = kind.default_placement(window.kind.is_transient());
            }
        }
        let (outputs, windows) = (&self.outputs, &self.windows);
        self.layouts
            .sync(|workspace, output| arrange::tiled_params(workspace, output, windows, outputs))
    }

    /// Applies the tiling changes queued since the last frame, syncs the layouts,
    /// and applies the runs that finished before `deadline`. A settled `Float`
    /// takes a window out of tiling only while it is still tiled on that
    /// workspace; placements follow as [`Effect::PlacementTarget`] and failures as
    /// [`Effect::LayoutFailed`].
    fn collect_layouts(&mut self, deadline: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        for event in std::mem::take(&mut self.deferred) {
            effects.extend(self.apply_queued(event, deadline));
        }
        effects.extend(self.sync_layouts());
        for (workspace, outcome) in self.layouts.collect(deadline) {
            match outcome {
                Ok(layout) => effects.extend(self.apply_layout(workspace, layout)),
                Err(failure) => effects.push(Effect::LayoutFailed {
                    workspace,
                    reason: failure.reason(),
                }),
            }
        }
        effects
    }

    /// Applies one queued event. Every workspace whose tiled set it changed is
    /// offered the change as a request; a handled request replaces the window's own
    /// geometry with the layout's placement, and a refused one leaves the window's own
    /// geometry in force.
    fn apply_queued(&mut self, event: Event, deadline: Instant) -> Vec<Effect> {
        let before = self.tiling_state();
        let mut effects = self.apply_event(event);
        let kinds = request_kinds(&before, &self.tiling_state());
        for (workspace, kind) in kinds {
            let (windows, outputs) = (&self.windows, &self.outputs);
            let verdict = self.layouts.offer(workspace, kind, deadline, |output| {
                arrange::tiled_params(workspace, output, windows, outputs)
            });
            match verdict {
                arrange::RequestVerdict::Handled(layout) => {
                    let placed = &self.layouts;
                    effects.retain(|effect| {
                        !matches!(effect, Effect::Configure { window, .. } if placed.placed(workspace, *window).is_some())
                    });
                    effects.extend(self.apply_layout(workspace, layout));
                }
                arrange::RequestVerdict::Failed(failure) => effects.push(Effect::LayoutFailed {
                    workspace,
                    reason: failure.reason(),
                }),
                arrange::RequestVerdict::InvalidParams => effects.push(Effect::LayoutFailed {
                    workspace,
                    reason: "invalid-params",
                }),
                arrange::RequestVerdict::Fallthrough => {}
            }
        }
        effects
    }

    /// Frees the windows a layout took out of tiling, and reports its placements.
    fn apply_layout(&mut self, workspace: WorkspaceId, layout: Layout) -> Vec<Effect> {
        let mut effects = Vec::new();
        for effect in layout.effects {
            let LayoutEffect::Float { window } = effect;
            let tiled_here = self
                .windows
                .get(&window)
                .is_some_and(|entry| entry.workspace == workspace && arrange::is_tiled(&entry.state, entry.placement));
            if tiled_here {
                effects.extend(self.set_placement(window, WindowPlacement::Free));
            }
        }
        // The tiled edges come from the placed rectangles: a placement's neighbours are the
        // other visible placements of the same run. Only a tiling layout tiles its windows.
        let tiling = self.layouts.kind(workspace) == Some(LayoutKind::Tiling);
        let rects: Vec<Option<Rect>> = layout.placements.iter().map(placement_rect).collect();
        for (index, placement) in layout.placements.iter().enumerate() {
            let work_area = self.placement_work_area(placement.window);
            let tiled = match (rects[index], work_area) {
                (Some(rect), Some(area)) => {
                    let neighbours: Vec<Rect> = layout
                        .placements
                        .iter()
                        .enumerate()
                        .filter(|(other, sibling)| *other != index && sibling.visible)
                        .filter_map(|(other, _)| rects[other])
                        .collect();
                    tiled_edges(tiling, rect, &neighbours, area)
                }
                _ => TiledEdges::NONE,
            };
            effects.push(Effect::PlacementTarget {
                workspace,
                placement: placement.clone(),
                tiled,
                work_area,
            });
        }
        effects
    }

    /// The work area of the output a window is on, or `None` when it has no output.
    fn placement_work_area(&self, window: WindowId) -> Option<Rect> {
        let output = self.windows.get(&window)?.output?;
        self.work_area(output)
    }

    /// The tiled windows and the workspace and rectangle each is tiled at.
    fn tiling_state(&self) -> TiledState {
        self.windows
            .iter()
            .filter(|(_, window)| arrange::is_tiled(&window.state, window.placement))
            .map(|(&id, window)| (id, (window.workspace, window.geometry)))
            .collect()
    }

    /// Moves focus to the nearest visible window in `direction` from the focused
    /// one, judged by placement rectangles. Nothing moves when nothing is focused
    /// or no window lies that way.
    fn focus_direction(&mut self, direction: Direction) -> Vec<Effect> {
        let Some(from) = self.focus else {
            return Vec::new();
        };
        let Some(origin) = self.focus_rect(from) else {
            return Vec::new();
        };
        let (ox, oy) = center(origin);
        let active = self.active_workspace;
        let mut best: Option<(f64, WindowId)> = None;
        for (&id, window) in &self.windows {
            if id == from || !window.is_visible(active) {
                continue;
            }
            let Some(rect) = self.focus_rect(id) else {
                continue;
            };
            let (cx, cy) = center(rect);
            let (ahead, aside) = match direction {
                Direction::Left => (ox - cx, (cy - oy).abs()),
                Direction::Right => (cx - ox, (cy - oy).abs()),
                Direction::Up => (oy - cy, (cx - ox).abs()),
                Direction::Down => (cy - oy, (cx - ox).abs()),
            };
            if ahead <= 0.0 {
                continue;
            }
            let score = ahead + 2.0 * aside;
            if best.is_none_or(|(current, _)| score < current) {
                best = Some((score, id));
            }
        }
        match best {
            Some((_, id)) => self.activation(id, ActivationDecision::Allow),
            None => Vec::new(),
        }
    }

    /// The rectangle a tiled window's layout last placed it at, or `None` when no
    /// settled layout places it. A window that left tiling keeps no placement: its
    /// geometry is the one that counts.
    #[must_use]
    pub fn placed_rect(&self, id: WindowId) -> Option<RectF> {
        let window = self.windows.get(&id)?;
        if !arrange::is_tiled(&window.state, window.placement) {
            return None;
        }
        self.layouts.placed(window.workspace, id)
    }

    /// The rectangle focus navigation measures `id` by: its placement when its
    /// layout placed it, otherwise its geometry.
    fn focus_rect(&self, id: WindowId) -> Option<RectF> {
        let window = self.windows.get(&id)?;
        Some(
            self.layouts
                .placed(window.workspace, id)
                .unwrap_or_else(|| window.geometry.to_f64()),
        )
    }
}

/// The rectangle a placement occupies, rounded the way the compositor rounds it when it
/// configures the window. `None` for a raw matrix transform, which has no rectangle.
fn placement_rect(placement: &Placement) -> Option<Rect> {
    let Transform3::Trs { position, .. } = placement.transform else {
        return None;
    };
    Some(Rect::new(
        position.x.round() as i32,
        position.y.round() as i32,
        placement.width.round() as u32,
        placement.height.round() as u32,
    ))
}

/// The tiled windows of a state: each window's workspace and rectangle.
type TiledState = BTreeMap<WindowId, (WorkspaceId, Rect)>;

/// The center of a rectangle.
fn center(rect: RectF) -> (f64, f64) {
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

/// Whether `event` can change which windows are tiled, or where they are.
fn changes_tiling(event: &Event) -> bool {
    matches!(
        event,
        Event::WindowOpened(_)
            | Event::WindowClosed { .. }
            | Event::WindowRoleChanged { .. }
            | Event::Minimise { .. }
            | Event::Unminimise { .. }
            | Event::ToggleFloat { .. }
            | Event::MoveResize { .. }
            | Event::ToggleMaximise { .. }
            | Event::ToggleFullscreen { .. }
            | Event::SnapRequested { .. }
            | Event::Unsnap { .. }
    )
}

/// The requests a change to the tiled set owes each workspace's layout. A window
/// that joined tiling is placed, one that left is left, and one that moved or
/// resized is moved. A window that changed workspace leaves the old one and joins
/// the new one.
fn request_kinds(before: &TiledState, after: &TiledState) -> Vec<(WorkspaceId, RequestKind)> {
    let mut kinds = Vec::new();
    for (&window, &(workspace, rect)) in before {
        match after.get(&window) {
            None => kinds.push((workspace, RequestKind::Leave { window })),
            Some(&(now, _)) if now != workspace => {
                kinds.push((workspace, RequestKind::Leave { window }));
                kinds.push((now, RequestKind::Place { window }));
            }
            Some(&(_, now)) if now != rect => kinds.push((workspace, RequestKind::Move { window, geometry: now })),
            Some(_) => {}
        }
    }
    for (&window, &(workspace, _)) in after {
        if !before.contains_key(&window) {
            kinds.push((workspace, RequestKind::Place { window }));
        }
    }
    kinds
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

    fn wm_with_two_windows() -> Wm {
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        for raw in 1..=2 {
            wm.apply(Event::WindowOpened(WindowSpec::normal(
                WindowId::new(raw),
                "org.example.App",
            )));
        }
        wm
    }

    #[test]
    fn an_open_modal_takes_focus_from_its_parent_until_it_closes() {
        // While a modal dialog is open its parent cannot take focus; the modal does.
        // Once the modal is no longer modal, the parent can be focused again.
        let mut wm = wm_with_two_windows();
        let (parent, modal) = (WindowId::new(1), WindowId::new(2));
        wm.apply(Event::ActivationRequested {
            window: parent,
            decision: ActivationDecision::Allow,
        });
        assert_eq!(wm.focused(), Some(parent));

        wm.apply(Event::WindowRoleChanged {
            window: modal,
            kind: WindowKind::Modal,
            transient_for: Some(parent),
        });
        assert_eq!(
            wm.focused(),
            Some(modal),
            "an opening modal takes focus from its parent"
        );

        wm.apply(Event::ActivationRequested {
            window: parent,
            decision: ActivationDecision::Allow,
        });
        assert_eq!(wm.focused(), Some(modal), "the parent is redirected to its modal child");

        wm.apply(Event::WindowRoleChanged {
            window: modal,
            kind: WindowKind::Normal,
            transient_for: None,
        });
        wm.apply(Event::ActivationRequested {
            window: parent,
            decision: ActivationDecision::Allow,
        });
        assert_eq!(wm.focused(), Some(parent), "the parent focuses again once unblocked");
        assert_eq!(wm.check_invariants().map(|_| ()), Ok(()));
    }

    #[test]
    fn a_transient_floats_and_stacks_above_its_parent() {
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
        let (child, parent) = (WindowId::new(1), WindowId::new(3));
        assert_eq!(wm.stack(), &[WindowId::new(3), WindowId::new(2), child]);

        wm.apply(Event::WindowRoleChanged {
            window: child,
            kind: WindowKind::Dialog,
            transient_for: Some(parent),
        });

        let Some(window) = wm.window(child) else {
            panic!("the transient is managed");
        };
        assert_eq!(
            window.placement,
            WindowPlacement::Free,
            "a transient leaves the tiled set"
        );
        assert_eq!(window.transient_for, Some(parent));
        let child_at = wm.stack().iter().position(|id| *id == child);
        let parent_at = wm.stack().iter().position(|id| *id == parent);
        assert!(child_at < parent_at, "the transient is stacked above its parent");
        assert_eq!(wm.check_invariants().map(|_| ()), Ok(()));
    }

    /// A manager with one output, one workspace and window 1 focused by a user action.
    fn wm_with_focused_window(options: WmOptions) -> Wm {
        let mut wm = Wm::new(options);
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(1),
            "org.example.One",
        )));
        wm.apply(Event::ActivationRequested {
            window: WindowId::new(1),
            decision: ActivationDecision::Allow,
        });
        wm
    }

    #[test]
    fn prevent_stops_background_window_stealing_focus() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(2),
            "org.example.Two",
        )));
        let decision = wm.new_window_decision(WindowId::new(2));
        assert_eq!(decision, ActivationDecision::Deny);
        let effects = wm.apply(Event::ActivationRequested {
            window: WindowId::new(2),
            decision,
        });
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::ActivationDenied { .. }))
        );
        assert_eq!(wm.focused(), Some(WindowId::new(1)));
    }

    #[test]
    fn prevent_lets_a_new_window_take_focus_when_nothing_is_focused() {
        let mut wm = Wm::new(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(1),
        });
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(1),
            "org.example.One",
        )));
        let decision = wm.new_window_decision(WindowId::new(1));
        assert_eq!(decision, ActivationDecision::Allow);
    }

    #[test]
    fn prevent_lets_a_transient_of_the_focused_window_take_focus() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowOpened(WindowSpec::transient(
            WindowId::new(2),
            "org.example.One",
            WindowId::new(1),
        )));
        assert_eq!(wm.new_window_decision(WindowId::new(2)), ActivationDecision::Allow);
    }

    #[test]
    fn prevent_refuses_a_transient_of_some_other_window() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(2),
            "org.example.Two",
        )));
        wm.apply(Event::WindowOpened(WindowSpec::transient(
            WindowId::new(3),
            "org.example.Two",
            WindowId::new(2),
        )));
        assert_eq!(wm.new_window_decision(WindowId::new(3)), ActivationDecision::Deny);
    }

    #[test]
    fn allow_lets_a_background_window_take_focus() {
        let options = WmOptions {
            new_window_focus: NewWindowFocus::Allow,
            ..WmOptions::default()
        };
        let mut wm = wm_with_focused_window(options);
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(2),
            "org.example.Two",
        )));
        let decision = wm.new_window_decision(WindowId::new(2));
        assert_eq!(decision, ActivationDecision::Allow);
        wm.apply(Event::ActivationRequested {
            window: WindowId::new(2),
            decision,
        });
        assert_eq!(wm.focused(), Some(WindowId::new(2)));
    }

    /// The `layer_order` window sequence, bottom to top, one entry per window-scope layer.
    fn layer_windows(wm: &Wm) -> Vec<WindowId> {
        wm.layer_order()
            .into_iter()
            .filter_map(|handle| match wm.layers().get(handle)?.spec.scope {
                LayerScope::Window(Some(id)) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// The state of the `surface_toplevel` record for `id`, or `None` once the
    /// window has closed and its layers have left the registry.
    fn surface_state(wm: &Wm, id: WindowId) -> Option<gnoblin_layers::LayerState> {
        wm.layers()
            .records()
            .find(|record| record.spec.scope == LayerScope::Window(Some(id)))
            .map(|record| record.state)
    }

    #[test]
    fn layer_order_follows_the_window_stack() {
        // Window 2 opens above window 1. Raising window 1 moves its layer last, so
        // the order follows the stack, not the order the windows opened in.
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(2),
            "org.example.Two",
        )));
        assert_eq!(layer_windows(&wm), vec![WindowId::new(1), WindowId::new(2)]);
        wm.apply(Event::Raise {
            window: WindowId::new(1),
        });
        assert_eq!(layer_windows(&wm), vec![WindowId::new(2), WindowId::new(1)]);
    }

    #[test]
    fn extension_layer_paints_below_the_builtin_layer_on_its_window() {
        // An extension cannot bury the compositor's own record on the window it follows.
        let mut wm = wm_with_focused_window(WmOptions::default());
        let spec = LayerSpec::new(
            LocalName::new("panel").expect("a plain local name is valid"),
            LayerScope::Window(Some(WindowId::new(1))),
            Band::Windows,
        );
        wm.layers
            .register(&LayerProvider::Extension("example".to_owned()), spec)
            .expect("the extension layer registers");
        let extension: Vec<bool> = wm
            .layer_order()
            .into_iter()
            .filter_map(|handle| wm.layers().get(handle))
            .map(|record| matches!(record.provider, LayerProvider::Extension(_)))
            .collect();
        assert_eq!(extension, vec![true, false]);
    }

    #[test]
    fn above_target_layer_sits_directly_above_its_window_record() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowOpened(WindowSpec::normal(
            WindowId::new(2),
            "org.example.Two",
        )));
        let mut spec = LayerSpec::new(
            LocalName::new("chrome").expect("a plain local name is valid"),
            LayerScope::Window(Some(WindowId::new(1))),
            Band::Windows,
        );
        spec.order = gnoblin_layers::Order::AboveTarget;
        wm.layers
            .register(&LayerProvider::Gnoblin("gnoblin-wm"), spec)
            .expect("the chrome layer registers");
        let order = wm.layer_order();
        // Window 1's record, then the chrome directly above it, then window 2.
        assert_eq!(
            layer_windows(&wm),
            vec![WindowId::new(1), WindowId::new(1), WindowId::new(2)]
        );
        let middle = wm.layers().get(order[1]).expect("the middle layer is live");
        assert!(matches!(middle.spec.order, gnoblin_layers::Order::AboveTarget));
    }

    #[test]
    fn minimising_hides_a_windows_layers_and_restoring_shows_them() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::Minimise {
            window: WindowId::new(1),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Hidden)
        );
        wm.apply(Event::Unminimise {
            window: WindowId::new(1),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Mapped)
        );
    }

    #[test]
    fn switching_workspace_hides_layers_of_windows_left_behind() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(2),
        });
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(2),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Hidden)
        );
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(1),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Mapped)
        );
    }

    #[test]
    fn closing_a_window_removes_its_layers() {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::WindowClosed {
            window: WindowId::new(1),
        });
        assert!(layer_windows(&wm).is_empty());
        assert_eq!(surface_state(&wm, WindowId::new(1)), None);
    }

    /// A manager with two side-by-side outputs, one workspace, and window 1 on the
    /// first output, focused by a user action. Output 1 is the survivor if output 0 goes.
    fn wm_with_two_outputs() -> Wm {
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::OutputConnected {
            output: OutputId::new(1),
            rect: Rect::new(1920, 0, 1920, 1080),
        });
        wm
    }

    #[test]
    fn unplugging_an_output_keeps_a_window_layer_mapped_on_the_survivor() {
        // Window 1 moves from output 0 to output 1. Its layer stays mapped and keeps
        // its place above the window, because the layer follows the window, not the output.
        let mut wm = wm_with_two_outputs();
        wm.apply(Event::OutputDisconnected {
            output: OutputId::new(0),
        });
        assert_eq!(
            wm.window(WindowId::new(1)).and_then(|window| window.output),
            Some(OutputId::new(1))
        );
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Mapped)
        );
        assert_eq!(layer_windows(&wm), vec![WindowId::new(1)]);
    }

    #[test]
    fn unplugging_the_last_output_hides_a_window_layer_until_an_output_returns() {
        // With no output left the window is unassigned, so its layer is hidden. Replugging
        // adopts the orphan and the layer is mapped again.
        let mut wm = wm_with_focused_window(WmOptions::default());
        wm.apply(Event::OutputDisconnected {
            output: OutputId::new(0),
        });
        assert_eq!(wm.window(WindowId::new(1)).and_then(|window| window.output), None);
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Hidden)
        );
        wm.apply(Event::OutputConnected {
            output: OutputId::new(0),
            rect: Rect::new(0, 0, 1920, 1080),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Mapped)
        );
    }

    #[test]
    fn a_window_moved_to_another_output_keeps_its_layer_hidden_while_its_workspace_is_not_active() {
        // The workspace is global, so the survivor's view is the same as the old output's:
        // a window on an inactive workspace stays hidden after its output changes.
        let mut wm = wm_with_two_outputs();
        wm.apply(Event::WorkspaceAdded {
            workspace: WorkspaceId::new(2),
        });
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(2),
        });
        wm.apply(Event::OutputDisconnected {
            output: OutputId::new(0),
        });
        assert_eq!(
            wm.window(WindowId::new(1)).and_then(|window| window.output),
            Some(OutputId::new(1))
        );
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Hidden)
        );
        wm.apply(Event::WorkspaceActivated {
            workspace: WorkspaceId::new(1),
        });
        assert_eq!(
            surface_state(&wm, WindowId::new(1)),
            Some(gnoblin_layers::LayerState::Mapped)
        );
    }
}
