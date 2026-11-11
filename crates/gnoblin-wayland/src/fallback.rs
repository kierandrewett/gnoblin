//! The compositor's own record of windows and outputs, and the minimal window policy it applies when no shell
//! holds the tree (`design/node-core.md`, F1).
//!
//! The record is plain data: for each window its kind, parent, output, rectangle and the three state flags; for each
//! output its rectangle and the insets layer surfaces reserved. The policy is a handful of pure decisions over it:
//!
//! - a new window cascades from (32, 32) by 32 px per visible window on its output
//!   ([`gnoblin_types::cascade_position`]), or is centred over its parent;
//! - maximise fills the work area, fullscreen covers the output, and leaving either returns the floating rectangle;
//! - minimise hides the window and hands focus on; closing the focused window does the same;
//! - activating a window focuses it and, when asked, raises it. A raised window stays under every fullscreen one;
//! - an output's arrival, loss, move or reservation re-fits the windows on it, and a window never points at an
//!   output that is gone.
//!
//! A decision never touches a Smithay type. It returns [`FallbackEffect`]s the caller applies, and reads the paint order and
//! the focus through [`Standing`], so the one record of both stays the scene's and the keyboard's. While a shell holds
//! the tree the caller keeps this record current and applies none of the effects: the shell decides.

use std::collections::BTreeMap;

use gnoblin_types::{Insets, OutputId, Point, Rect, Size, WindowId, cascade_position};

use crate::policy::NewWindowFocus;

/// The size of a window whose client asked for none.
const DEFAULT_SIZE: Size = Size::new(800, 600);

/// The most windows the record holds. A window past it is left unmanaged, so a client that opens toplevels in a
/// loop cannot grow the record without bound.
pub(crate) const MAX_WINDOWS: usize = 256;

/// What kind of toplevel a window is. A transient is centred over its parent and focus goes to a modal child first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WindowKind {
    /// An application toplevel.
    #[default]
    Normal,
    /// A dialog with a parent.
    Dialog,
    /// A dialog that blocks its parent.
    Modal,
}

/// The three state flags of a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WindowState {
    /// The window fills its output's work area.
    pub maximised: bool,
    /// The window covers its whole output, over panels.
    pub fullscreen: bool,
    /// The window is hidden and out of the focus rotation.
    pub minimised: bool,
}

/// One window the record holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    /// The compositor's identity for the window.
    pub id: WindowId,
    /// What kind of toplevel it is.
    pub kind: WindowKind,
    /// The window it is a transient of, while that window is open.
    pub transient_for: Option<WindowId>,
    /// The output it is on; `None` only while no output is connected.
    pub output: Option<OutputId>,
    /// Its frame rectangle in global logical coordinates.
    pub geometry: Rect,
    /// The floating rectangle it returns to when it leaves maximise or fullscreen. Equal to `geometry` while floating.
    restore: Rect,
    /// The state flags.
    pub state: WindowState,
}

/// One connected output: its rectangle and what layer surfaces reserved along its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputRecord {
    /// The output's identity, as the backend numbers it.
    pub id: OutputId,
    /// The whole monitor rectangle, which a fullscreen window covers.
    pub rect: Rect,
    /// The insets layer surfaces reserved.
    pub reserved: Insets,
}

impl OutputRecord {
    /// The monitor rectangle less the reserved insets, never smaller than a pixel.
    pub(crate) fn work_area(&self) -> Rect {
        self.reserved.applied_to(self.rect).at_least_one()
    }
}

/// What a decision asks the caller to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FallbackEffect {
    /// Place the window at `geometry` and tell its client the state.
    Configure {
        window: WindowId,
        geometry: Rect,
        state: WindowState,
    },
    /// Show or hide the window.
    Visible { window: WindowId, visible: bool },
    /// The paint order, topmost first.
    Stack(Vec<WindowId>),
    /// Move keyboard focus to the window, or to none.
    Focus(Option<WindowId>),
}

/// The paint order and the focus a decision starts from, which the scene and the keyboard own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Standing<'a> {
    /// Every window drawn, topmost first.
    pub stack: &'a [WindowId],
    /// The focused window.
    pub focus: Option<WindowId>,
}

/// A window the compositor opens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Open {
    pub id: WindowId,
    pub kind: WindowKind,
    pub transient_for: Option<WindowId>,
    /// The rectangle the client asked for; only its size is used.
    pub requested: Option<Rect>,
}

/// Why the record refused a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackRefusal {
    /// The window is already open.
    Duplicate,
    /// The record is full.
    TooMany,
    /// No output is connected to place the window on.
    NoOutput,
    /// The window is not open.
    Gone,
    /// The window is maximised or fullscreen, so it has no floating rectangle to change.
    NotFloating,
    /// The output is not connected.
    UnknownOutput,
    /// A transient moves with its parent, not on its own.
    FollowsParent,
}

/// The record of windows and outputs, and the decisions over it.
#[derive(Debug, Default)]
pub(crate) struct Fallback {
    windows: BTreeMap<WindowId, Window>,
    outputs: Vec<OutputRecord>,
}

impl Fallback {
    /// One window.
    pub(crate) fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    /// Every window, by identity.
    pub(crate) fn windows(&self) -> impl Iterator<Item = &Window> {
        self.windows.values()
    }

    /// One connected output.
    pub(crate) fn output(&self, id: OutputId) -> Option<&OutputRecord> {
        self.outputs.iter().find(|output| output.id == id)
    }

    /// The work area of an output.
    pub(crate) fn work_area(&self, id: OutputId) -> Option<Rect> {
        self.output(id).map(OutputRecord::work_area)
    }

    /// Whether the window would be drawn: it is on an output and not minimised.
    pub(crate) fn is_visible(&self, id: WindowId) -> bool {
        self.windows
            .get(&id)
            .is_some_and(|window| window.output.is_some() && !window.state.minimised)
    }

    /// Applies a window's role: its kind and parent, as its protocol says. A window that becomes a transient is
    /// stacked directly above its parent, and a modal that appears while its parent holds focus takes it. A parent
    /// that is not open changes nothing.
    pub(crate) fn set_role(
        &mut self,
        id: WindowId,
        kind: WindowKind,
        transient_for: Option<WindowId>,
        seat: Standing<'_>,
    ) -> Vec<FallbackEffect> {
        let parent = transient_for.filter(|parent| *parent != id);
        if parent.is_some_and(|parent| !self.windows.contains_key(&parent)) {
            return Vec::new();
        }
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        let reparented = window.transient_for != parent;
        window.kind = kind;
        window.transient_for = parent;
        let mut effects = Vec::new();
        if reparented && parent.is_some() {
            effects.extend(self.restacked(seat, |this, order| this.put_on_top(order, id, parent)));
        }
        if kind == WindowKind::Modal && parent.is_some() && seat.focus == parent && self.is_visible(id) {
            effects.push(FallbackEffect::Focus(Some(id)));
        }
        effects
    }

    /// Whether a window that has just mapped may take focus: any window under `Allow`; under `Prevent` only when
    /// nothing is focused or it is a transient of the focused window.
    pub(crate) fn may_take_focus(&self, id: WindowId, policy: NewWindowFocus, seat: Standing<'_>) -> bool {
        let Some(window) = self.windows.get(&id) else {
            return false;
        };
        match policy {
            NewWindowFocus::Allow => true,
            NewWindowFocus::Prevent => seat.focus.is_none() || window.transient_for == seat.focus,
        }
    }

    /// Opens a window: a transient is centred over its parent on the parent's output, any other window cascades
    /// on the output of the focused window, or the first output. The new window is on top of its layer, or directly
    /// above its parent.
    ///
    /// # Errors
    /// [`FallbackRefusal::Duplicate`], [`FallbackRefusal::TooMany`] or [`FallbackRefusal::NoOutput`]; nothing changes.
    pub(crate) fn open(&mut self, open: Open, seat: Standing<'_>) -> Result<Vec<FallbackEffect>, FallbackRefusal> {
        if self.windows.contains_key(&open.id) {
            return Err(FallbackRefusal::Duplicate);
        }
        if self.windows.len() >= MAX_WINDOWS {
            return Err(FallbackRefusal::TooMany);
        }
        let focused_output = seat
            .focus
            .and_then(|focus| self.windows.get(&focus)?.output)
            .filter(|output| self.output(*output).is_some());
        let home = focused_output
            .or_else(|| self.outputs.first().map(|output| output.id))
            .ok_or(FallbackRefusal::NoOutput)?;
        let size = open.requested.map_or(DEFAULT_SIZE, Rect::size).at_least_one();
        let parent = open
            .transient_for
            .filter(|parent| *parent != open.id)
            .and_then(|parent| self.windows.get(&parent));
        let (output, geometry) = match parent {
            Some(parent) => {
                let output = parent
                    .output
                    .filter(|output| self.output(*output).is_some())
                    .unwrap_or(home);
                let area = self.work_area(output).unwrap_or(Rect::EMPTY);
                let offset = |parent: u32, own: u32| i32::try_from(parent.saturating_sub(own) / 2).unwrap_or(0);
                let (x, y) = (
                    parent
                        .geometry
                        .x
                        .saturating_add(offset(parent.geometry.width, size.width)),
                    parent
                        .geometry
                        .y
                        .saturating_add(offset(parent.geometry.height, size.height)),
                );
                (output, Rect::new(x, y, size.width, size.height).clamp_inside(area))
            }
            None => {
                let area = self.work_area(home).unwrap_or(Rect::EMPTY);
                let visible = self
                    .windows
                    .values()
                    .filter(|other| other.output == Some(home) && !other.state.minimised)
                    .count();
                let (x, y) = cascade_position(visible, area, size);
                (home, Rect::new(x, y, size.width, size.height))
            }
        };
        let anchor = parent.map(|parent| parent.id);
        self.windows.insert(
            open.id,
            Window {
                id: open.id,
                kind: open.kind,
                transient_for: anchor,
                output: Some(output),
                geometry,
                restore: geometry,
                state: WindowState::default(),
            },
        );
        let mut effects = vec![
            FallbackEffect::Configure {
                window: open.id,
                geometry,
                state: WindowState::default(),
            },
            FallbackEffect::Visible {
                window: open.id,
                visible: true,
            },
        ];
        effects.extend(self.restacked(seat, |this, order| this.put_on_top(order, open.id, anchor)));
        Ok(effects)
    }

    /// Forgets a closed window. A parent loses its link to it, and focus moves on when it held it.
    pub(crate) fn close(&mut self, id: WindowId, seat: Standing<'_>) -> Vec<FallbackEffect> {
        if self.windows.remove(&id).is_none() {
            return Vec::new();
        }
        for window in self.windows.values_mut() {
            if window.transient_for == Some(id) {
                window.transient_for = None;
            }
        }
        if seat.focus == Some(id) {
            return self.refocused(seat);
        }
        Vec::new()
    }

    /// Focuses a window, and raises it when `raise` is set. A window that is not drawn cannot take focus, and a
    /// parent with a modal child hands the focus to the child.
    pub(crate) fn activate(&self, id: WindowId, raise: bool, seat: Standing<'_>) -> Vec<FallbackEffect> {
        if !self.is_visible(id) {
            return Vec::new();
        }
        let target = self.blocking_modal(id, seat.stack).unwrap_or(id);
        let mut effects = Vec::new();
        if raise {
            effects.extend(self.restacked(seat, |this, order| this.raise_in(order, target)));
        }
        if seat.focus != Some(target) {
            effects.push(FallbackEffect::Focus(Some(target)));
        }
        effects
    }

    /// Puts a window on top of its layer.
    pub(crate) fn raise(&self, id: WindowId, seat: Standing<'_>) -> Vec<FallbackEffect> {
        if !self.windows.contains_key(&id) {
            return Vec::new();
        }
        self.restacked(seat, |this, order| this.raise_in(order, id))
            .into_iter()
            .collect()
    }

    /// Puts a window under every other window of its layer.
    pub(crate) fn lower(&self, id: WindowId, seat: Standing<'_>) -> Vec<FallbackEffect> {
        if !self.windows.contains_key(&id) {
            return Vec::new();
        }
        let restack = self.restacked(seat, |this, order| {
            if !this.is_fullscreen(id) {
                order.retain(|window| *window != id);
                order.push(id);
            }
        });
        restack.into_iter().collect()
    }

    /// Maximises a window to the work area of its output, or returns it to its floating rectangle. Refused, with no
    /// effect, for a fullscreen window and for a window on no output; unchanged when it already is as asked.
    pub(crate) fn set_maximised(&mut self, id: WindowId, on: bool) -> Vec<FallbackEffect> {
        let Some(window) = self.windows.get(&id) else {
            return Vec::new();
        };
        let Some(area) = window.output.and_then(|output| self.work_area(output)) else {
            return Vec::new();
        };
        if window.state.fullscreen || window.state.maximised == on {
            return Vec::new();
        }
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        window.state.maximised = on;
        if on {
            window.restore = window.geometry;
            window.geometry = area;
        } else {
            window.geometry = window.restore.clamp_inside(area);
        }
        vec![Self::configured(window)]
    }

    /// Makes a window cover its output, or returns it to the maximised or floating rectangle it came from. Fullscreen
    /// windows are the top layer of the paint order.
    pub(crate) fn set_fullscreen(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<FallbackEffect> {
        let Some(window) = self.windows.get(&id) else {
            return Vec::new();
        };
        let Some(output) = window.output.and_then(|output| self.output(output)).copied() else {
            return Vec::new();
        };
        if window.state.fullscreen == on {
            return Vec::new();
        }
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        window.state.fullscreen = on;
        if on {
            if !window.state.maximised {
                window.restore = window.geometry;
            }
            window.geometry = output.rect;
        } else {
            window.geometry = Self::resting_place(window, &output);
        }
        let mut effects = vec![Self::configured(window)];
        // Entering the layer puts the window on top of it; leaving needs no move, because the sort by layer drops the
        // window under the fullscreen ones that remain.
        effects.extend(self.restacked(seat, |_, order| {
            if on {
                order.retain(|window| *window != id);
                order.insert(0, id);
            }
        }));
        effects
    }

    /// Hides a window and hands focus on, or brings it back. Bringing a window back focuses something when nothing
    /// holds focus, because a desktop with no focused window cannot receive a key.
    pub(crate) fn set_minimised(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<FallbackEffect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        if window.state.minimised == on {
            return Vec::new();
        }
        window.state.minimised = on;
        let visible = !on && window.output.is_some();
        let mut effects = vec![FallbackEffect::Visible { window: id, visible }];
        if (on && seat.focus == Some(id)) || (!on && seat.focus.is_none()) {
            effects.extend(self.refocused(seat));
        }
        effects
    }

    /// Moves and resizes a floating window, as a control request asks. The rectangle is kept inside the work area.
    ///
    /// # Errors
    /// [`FallbackRefusal::Gone`] for a window that is not open, [`FallbackRefusal::NotFloating`] for a maximised or fullscreen one.
    pub(crate) fn move_resize(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<FallbackEffect>, FallbackRefusal> {
        let window = self.windows.get(&id).ok_or(FallbackRefusal::Gone)?;
        if window.state.maximised || window.state.fullscreen {
            return Err(FallbackRefusal::NotFloating);
        }
        let Some(area) = window.output.and_then(|output| self.work_area(output)) else {
            return Err(FallbackRefusal::Gone);
        };
        let window = self.windows.get_mut(&id).ok_or(FallbackRefusal::Gone)?;
        let fitted = wanted.at_least_one().clamp_inside(area);
        if fitted == window.geometry {
            return Ok(Vec::new());
        }
        window.geometry = fitted;
        window.restore = fitted;
        Ok(vec![Self::configured(window)])
    }

    /// Puts a floating window's top left at an exact position and keeps its size, with no work-area constraint.
    ///
    /// [`Fallback::move_resize`] is the policy for a request that comes from a user or a script: it keeps the window
    /// where a person can reach it. This is for the embedder that positions windows deliberately, such as the
    /// conformance suite's `position_window_absolute`, whose tests place a window wherever its geometry needs.
    ///
    /// # Errors
    /// [`FallbackRefusal::Gone`] for a window that is not open, [`FallbackRefusal::NotFloating`] for a maximised or fullscreen one.
    pub(crate) fn place(&mut self, id: WindowId, at: Point) -> Result<Vec<FallbackEffect>, FallbackRefusal> {
        let window = self.windows.get_mut(&id).ok_or(FallbackRefusal::Gone)?;
        if window.state.maximised || window.state.fullscreen {
            return Err(FallbackRefusal::NotFloating);
        }
        let placed = Rect::new(at.x, at.y, window.geometry.width, window.geometry.height);
        if placed == window.geometry {
            return Ok(Vec::new());
        }
        window.geometry = placed;
        window.restore = placed;
        Ok(vec![Self::configured(window)])
    }

    /// Moves a window, and its transients, to another output. A transient follows its parent and does not move on
    /// its own; a fullscreen window stays where it is.
    ///
    /// # Errors
    /// [`FallbackRefusal::Gone`], [`FallbackRefusal::UnknownOutput`], [`FallbackRefusal::FollowsParent`] or [`FallbackRefusal::NotFloating`].
    pub(crate) fn move_to_output(
        &mut self,
        id: WindowId,
        target: OutputId,
    ) -> Result<Vec<FallbackEffect>, FallbackRefusal> {
        let window = self.windows.get(&id).ok_or(FallbackRefusal::Gone)?;
        let output = *self.output(target).ok_or(FallbackRefusal::UnknownOutput)?;
        if window.kind != WindowKind::Normal && window.transient_for.is_some() {
            return Err(FallbackRefusal::FollowsParent);
        }
        if window.state.fullscreen {
            return Err(FallbackRefusal::NotFloating);
        }
        if window.output == Some(target) {
            return Ok(Vec::new());
        }
        let family: Vec<WindowId> = self
            .windows
            .values()
            .filter(|other| other.id == id || other.transient_for == Some(id))
            .map(|other| other.id)
            .collect();
        let mut effects = Vec::new();
        for member in family {
            if let Some(window) = self.windows.get_mut(&member) {
                window.output = Some(target);
                window.restore = window.restore.clamp_inside(output.work_area());
                window.geometry = Self::fitted(window, &output);
                effects.push(Self::configured(window));
            }
        }
        Ok(effects)
    }

    /// An output arrived. A window left on no output is adopted by it, and focus is chosen when none was held.
    pub(crate) fn output_added(&mut self, id: OutputId, rect: Rect, seat: Standing<'_>) -> Vec<FallbackEffect> {
        if self.output(id).is_some() {
            return Vec::new();
        }
        let output = OutputRecord {
            id,
            rect: rect.at_least_one(),
            reserved: Insets::ZERO,
        };
        self.outputs.push(output);
        let mut effects = self.rehome(|window| window.output.is_none(), &output);
        if seat.focus.is_none() {
            effects.extend(self.refocused(seat));
        }
        effects
    }

    /// An output went away. Its windows move to the first output left and are re-fitted there; with none left they
    /// are hidden until one returns. Focus moves on when its window is no longer drawn.
    pub(crate) fn output_removed(&mut self, id: OutputId, seat: Standing<'_>) -> Vec<FallbackEffect> {
        let Some(index) = self.outputs.iter().position(|output| output.id == id) else {
            return Vec::new();
        };
        self.outputs.remove(index);
        let mut effects = match self.outputs.first().copied() {
            Some(survivor) => self.rehome(|window| window.output == Some(id), &survivor),
            None => {
                let orphans: Vec<WindowId> = self
                    .windows
                    .values()
                    .filter(|window| window.output == Some(id))
                    .map(|window| window.id)
                    .collect();
                for window in &orphans {
                    if let Some(window) = self.windows.get_mut(window) {
                        window.output = None;
                    }
                }
                orphans
                    .into_iter()
                    .map(|window| FallbackEffect::Visible { window, visible: false })
                    .collect()
            }
        };
        if seat.focus.is_some_and(|focus| !self.is_visible(focus)) {
            effects.extend(self.refocused(seat));
        }
        effects
    }

    /// An output's rectangle changed: the windows on it follow.
    pub(crate) fn output_moved(&mut self, id: OutputId, rect: Rect) -> Vec<FallbackEffect> {
        let Some(output) = self.outputs.iter_mut().find(|output| output.id == id) else {
            return Vec::new();
        };
        output.rect = rect.at_least_one();
        let output = *output;
        self.refit(&output)
    }

    /// The insets layer surfaces reserve on an output changed: maximised windows follow the new work area.
    pub(crate) fn reserve(&mut self, id: OutputId, reserved: Insets) -> Vec<FallbackEffect> {
        let Some(output) = self.outputs.iter_mut().find(|output| output.id == id) else {
            return Vec::new();
        };
        output.reserved = reserved;
        let output = *output;
        self.refit(&output)
    }

    /// Re-fits every window on `output` to its rectangles, reporting the ones that moved.
    fn refit(&mut self, output: &OutputRecord) -> Vec<FallbackEffect> {
        let mut effects = Vec::new();
        for window in self
            .windows
            .values_mut()
            .filter(|window| window.output == Some(output.id))
        {
            window.restore = window.restore.clamp_inside(output.work_area());
            let fitted = Self::fitted(window, output);
            if fitted != window.geometry {
                window.geometry = fitted;
                effects.push(Self::configured(window));
            }
        }
        effects
    }

    /// Moves the windows `which` selects onto `output` and re-fits them there.
    fn rehome(&mut self, which: impl Fn(&Window) -> bool, output: &OutputRecord) -> Vec<FallbackEffect> {
        let mut effects = Vec::new();
        for window in self.windows.values_mut().filter(|window| which(window)) {
            window.output = Some(output.id);
            window.restore = window.restore.clamp_inside(output.work_area());
            window.geometry = Self::fitted(window, output);
            effects.push(FallbackEffect::Visible {
                window: window.id,
                visible: !window.state.minimised,
            });
            effects.push(Self::configured(window));
        }
        effects
    }

    /// Where a window belongs on `output` for its state.
    fn fitted(window: &Window, output: &OutputRecord) -> Rect {
        if window.state.fullscreen {
            output.rect
        } else if window.state.maximised {
            output.work_area()
        } else {
            window.geometry.clamp_inside(output.work_area())
        }
    }

    /// Where a window that just left fullscreen belongs: the work area when it is maximised, else its floating
    /// rectangle.
    fn resting_place(window: &Window, output: &OutputRecord) -> Rect {
        if window.state.maximised {
            output.work_area()
        } else {
            window.restore.clamp_inside(output.work_area())
        }
    }

    fn configured(window: &Window) -> FallbackEffect {
        FallbackEffect::Configure {
            window: window.id,
            geometry: window.geometry,
            state: window.state,
        }
    }

    fn is_fullscreen(&self, id: WindowId) -> bool {
        self.windows.get(&id).is_some_and(|window| window.state.fullscreen)
    }

    /// The topmost drawn modal child of `parent`, which takes the focus the parent was offered.
    fn blocking_modal(&self, parent: WindowId, order: &[WindowId]) -> Option<WindowId> {
        order.iter().copied().find(|id| {
            self.windows.get(id).is_some_and(|window| {
                window.kind == WindowKind::Modal && window.transient_for == Some(parent) && self.is_visible(*id)
            })
        })
    }

    /// The focus after the focused window left: the topmost drawn window, or none.
    fn refocused(&self, seat: Standing<'_>) -> Vec<FallbackEffect> {
        let next = seat
            .stack
            .iter()
            .copied()
            .find(|id| self.is_visible(*id))
            .map(|id| self.blocking_modal(id, seat.stack).unwrap_or(id));
        if next == seat.focus {
            Vec::new()
        } else {
            vec![FallbackEffect::Focus(next)]
        }
    }

    /// The paint order after `change`, with every fullscreen window above the others, as a [`FallbackEffect::Stack`] when
    /// it differs from the one the caller holds.
    fn restacked(&self, seat: Standing<'_>, change: impl FnOnce(&Self, &mut Vec<WindowId>)) -> Option<FallbackEffect> {
        let mut order = seat.stack.to_vec();
        for id in self.windows.keys().rev() {
            if !order.contains(id) {
                order.insert(0, *id);
            }
        }
        change(self, &mut order);
        // A stable sort: only the layer decides, so windows keep their order within it.
        order.sort_by_key(|id| !self.is_fullscreen(*id));
        (order != seat.stack).then_some(FallbackEffect::Stack(order))
    }

    /// Puts `id` directly above `anchor`, or on top of the paint order.
    fn put_on_top(&self, order: &mut Vec<WindowId>, id: WindowId, anchor: Option<WindowId>) {
        order.retain(|window| *window != id);
        let above = anchor.and_then(|anchor| order.iter().position(|window| *window == anchor));
        order.insert(above.unwrap_or(0), id);
    }

    /// Puts `id` on top of its layer: a fullscreen window is already there.
    fn raise_in(&self, order: &mut Vec<WindowId>, id: WindowId) {
        if !self.is_fullscreen(id) {
            order.retain(|window| *window != id);
            order.insert(0, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    fn id(raw: u64) -> WindowId {
        WindowId::new(raw)
    }

    /// The seat of a desk, built from its fields so the fallback beside them stays free to borrow mutably.
    macro_rules! seat {
        ($desk:expr) => {
            Standing {
                stack: &$desk.stack,
                focus: $desk.focus,
            }
        };
    }

    /// A fallback with one output, and the paint order and focus the compositor would hold, which `apply` keeps.
    struct Desk {
        fallback: Fallback,
        stack: Vec<WindowId>,
        focus: Option<WindowId>,
    }

    impl Desk {
        fn new() -> Self {
            let mut desk = Self {
                fallback: Fallback::default(),
                stack: Vec::new(),
                focus: None,
            };
            desk.output(0, MONITOR);
            desk
        }

        fn apply(&mut self, effects: &[FallbackEffect]) {
            for effect in effects {
                match effect {
                    FallbackEffect::Stack(order) => self.stack.clone_from(order),
                    FallbackEffect::Focus(focus) => self.focus = *focus,
                    FallbackEffect::Configure { .. } | FallbackEffect::Visible { .. } => {}
                }
            }
        }

        fn output(&mut self, raw: u64, rect: Rect) -> Vec<FallbackEffect> {
            let effects = self.fallback.output_added(OutputId::new(raw), rect, seat!(self));
            self.apply(&effects);
            effects
        }

        fn unplug(&mut self, raw: u64) -> Vec<FallbackEffect> {
            let effects = self.fallback.output_removed(OutputId::new(raw), seat!(self));
            self.apply(&effects);
            effects
        }

        fn try_open(&mut self, open: Open) -> Result<Vec<FallbackEffect>, FallbackRefusal> {
            let effects = self.fallback.open(open, seat!(self))?;
            self.apply(&effects);
            Ok(effects)
        }

        fn open_with(&mut self, raw: u64, kind: WindowKind, parent: Option<u64>) -> Vec<FallbackEffect> {
            let open = Open {
                id: id(raw),
                kind,
                transient_for: parent.map(id),
                requested: Some(Rect::new(0, 0, 600, 400)),
            };
            self.try_open(open).expect("the window opens")
        }

        fn open(&mut self, raw: u64) -> Vec<FallbackEffect> {
            self.open_with(raw, WindowKind::Normal, None)
        }

        fn focus(&mut self, raw: u64) -> Vec<FallbackEffect> {
            let effects = self.fallback.activate(id(raw), true, seat!(self));
            self.apply(&effects);
            effects
        }

        fn close(&mut self, raw: u64) -> Vec<FallbackEffect> {
            let effects = self.fallback.close(id(raw), seat!(self));
            self.apply(&effects);
            effects
        }

        fn minimise(&mut self, raw: u64, on: bool) -> Vec<FallbackEffect> {
            let effects = self.fallback.set_minimised(id(raw), on, seat!(self));
            self.apply(&effects);
            effects
        }

        fn fullscreen(&mut self, raw: u64, on: bool) -> Vec<FallbackEffect> {
            let effects = self.fallback.set_fullscreen(id(raw), on, seat!(self));
            self.apply(&effects);
            effects
        }

        fn lower(&mut self, raw: u64) {
            let effects = self.fallback.lower(id(raw), seat!(self));
            self.apply(&effects);
        }

        fn geometry(&self, raw: u64) -> Option<Rect> {
            self.fallback.window(id(raw)).map(|window| window.geometry)
        }

        fn output_of(&self, raw: u64) -> Option<Option<OutputId>> {
            self.fallback.window(id(raw)).map(|window| window.output)
        }
    }

    fn visibility(effects: &[FallbackEffect], window: WindowId) -> Option<bool> {
        effects.iter().find_map(|effect| match effect {
            FallbackEffect::Visible { window: id, visible } if *id == window => Some(*visible),
            _ => None,
        })
    }

    fn configured(effects: &[FallbackEffect], window: WindowId) -> Option<Rect> {
        effects.iter().find_map(|effect| match effect {
            FallbackEffect::Configure {
                window: id, geometry, ..
            } if *id == window => Some(*geometry),
            _ => None,
        })
    }

    // Failure mode F1: with no shell a new window cascades from (32, 32) so it never opens hidden at the origin.
    #[test]
    fn a_new_window_without_a_shell_cascades_from_32_32() {
        let mut desk = Desk::new();
        let effects = desk.open(1);
        assert_eq!(desk.geometry(1), Some(Rect::new(32, 32, 600, 400)));
        assert_eq!(configured(&effects, id(1)), Some(Rect::new(32, 32, 600, 400)));
        assert_eq!(visibility(&effects, id(1)), Some(true));
    }

    #[test]
    fn three_windows_without_a_shell_get_distinct_cascading_positions() {
        let mut desk = Desk::new();
        for raw in 1..=3 {
            desk.open(raw);
        }
        let rects: Vec<Rect> = (1..=3).filter_map(|raw| desk.geometry(raw)).collect();
        assert_eq!(
            rects,
            [
                Rect::new(32, 32, 600, 400),
                Rect::new(64, 64, 600, 400),
                Rect::new(96, 96, 600, 400)
            ]
        );
        assert!(
            rects.iter().all(|rect| rect.is_inside(MONITOR)),
            "every window is on screen"
        );
    }

    #[test]
    fn a_minimised_window_does_not_push_the_cascade() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.minimise(1, true);
        desk.open(2);
        assert_eq!(desk.geometry(2), Some(Rect::new(32, 32, 600, 400)));
    }

    #[test]
    fn a_dialog_opens_centred_above_its_parent_on_the_parents_output() {
        let mut desk = Desk::new();
        desk.output(1, Rect::new(1920, 0, 1920, 1080));
        desk.open(1);
        desk.fallback
            .move_to_output(id(1), OutputId::new(1))
            .expect("the parent moves");
        let parent = desk.geometry(1).expect("the parent is open");
        let effects = desk.open_with(2, WindowKind::Dialog, Some(1));
        let dialog = desk.fallback.window(id(2)).expect("the dialog is open");
        assert_eq!(dialog.output, Some(OutputId::new(1)));
        assert_eq!(dialog.transient_for, Some(id(1)));
        assert_eq!(
            dialog.geometry,
            Rect::new(parent.x, parent.y, 600, 400),
            "a dialog of the parent's size centres exactly over it"
        );
        assert!(
            matches!(effects.last(), Some(FallbackEffect::Stack(order)) if order[..2] == [id(2), id(1)]),
            "the dialog sits directly above its parent: {effects:?}"
        );
    }

    // A window needs an output and a free identity, and a flood is refused without a change.
    #[test]
    fn a_window_needs_an_output_and_a_free_identity() {
        let mut desk = Desk::new();
        desk.open(1);
        let duplicate = Open {
            id: id(1),
            kind: WindowKind::Normal,
            transient_for: None,
            requested: None,
        };
        assert_eq!(desk.try_open(duplicate), Err(FallbackRefusal::Duplicate));
        let mut empty = Fallback::default();
        let lost = Open { id: id(2), ..duplicate };
        let seat = Standing {
            stack: &[],
            focus: None,
        };
        assert_eq!(empty.open(lost, seat), Err(FallbackRefusal::NoOutput));
        assert_eq!(empty.windows().count(), 0);
    }

    #[test]
    fn the_record_stops_at_its_cap() {
        let mut desk = Desk::new();
        let cap = u64::try_from(MAX_WINDOWS).expect("the cap fits");
        for raw in 1..=cap {
            desk.open(raw);
        }
        let past = Open {
            id: id(cap + 1),
            kind: WindowKind::Normal,
            transient_for: None,
            requested: None,
        };
        assert_eq!(desk.try_open(past), Err(FallbackRefusal::TooMany));
        assert_eq!(desk.fallback.windows().count(), MAX_WINDOWS);
    }

    #[test]
    fn new_windows_open_on_the_focused_windows_output() {
        let mut desk = Desk::new();
        desk.output(1, Rect::new(1920, 0, 1920, 1080));
        desk.open(1);
        // Unplugging the first output moves the window to the second; focusing it there is what makes the next
        // window open on the same monitor.
        desk.unplug(0);
        desk.focus(1);
        desk.open(2);
        assert_eq!(desk.output_of(2), Some(Some(OutputId::new(1))));
    }

    // Failure mode 1: focus lands on a closed window.
    #[test]
    fn focus_never_lands_on_a_closed_window() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        desk.focus(1);
        desk.focus(2);
        assert_eq!(desk.focus, Some(id(2)));

        let effects = desk.close(2);
        assert_eq!(
            effects,
            [FallbackEffect::Focus(Some(id(1)))],
            "closing the focused window reports where focus went"
        );
        assert!(
            desk.fallback.activate(id(2), true, seat!(desk)).is_empty(),
            "a closed window cannot be focused"
        );

        assert_eq!(desk.close(1), [FallbackEffect::Focus(None)], "no window can hold focus");
        assert!(desk.close(1).is_empty(), "closing twice changes nothing");
    }

    #[test]
    fn closing_an_unfocused_window_leaves_focus_alone() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        desk.focus(2);
        assert!(desk.close(1).is_empty());
        assert_eq!(desk.focus, Some(id(2)));
    }

    // Failure mode 2: minimised windows keep focus.
    #[test]
    fn minimising_the_focused_window_moves_focus_away() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        desk.focus(1);
        desk.focus(2);

        let effects = desk.minimise(2, true);
        assert_eq!(visibility(&effects, id(2)), Some(false));
        assert_eq!(desk.focus, Some(id(1)));
        assert!(!desk.fallback.is_visible(id(2)));
        assert!(desk.minimise(2, true).is_empty(), "re-minimising changes nothing");

        // Minimising the last drawn window leaves nothing to focus.
        desk.minimise(1, true);
        assert_eq!(desk.focus, None);

        // Bringing one back with nothing focused takes focus, because a desktop with no focus cannot take a key.
        let effects = desk.minimise(2, false);
        assert_eq!(visibility(&effects, id(2)), Some(true));
        assert_eq!(desk.focus, Some(id(2)));
    }

    #[test]
    fn a_hidden_window_cannot_be_activated() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.minimise(1, true);
        assert!(desk.fallback.activate(id(1), true, seat!(desk)).is_empty());
        assert_eq!(desk.focus, None);
    }

    // Failure mode 4: stacking order inconsistent with focus.
    #[test]
    fn raising_keeps_the_fullscreen_layer_on_top() {
        let mut desk = Desk::new();
        for raw in 1..=3 {
            desk.open(raw);
        }
        assert_eq!(desk.stack, [id(3), id(2), id(1)]);

        desk.fullscreen(2, true);
        assert_eq!(desk.stack, [id(2), id(3), id(1)]);

        desk.focus(1);
        assert_eq!(
            desk.stack,
            [id(2), id(1), id(3)],
            "a fullscreen window stays above the window that raised"
        );

        // Lowering the fullscreen window cannot put it under normal windows.
        desk.lower(2);
        assert_eq!(desk.stack, [id(2), id(1), id(3)]);

        // Leaving fullscreen drops it into the normal layer, where raise and lower apply.
        desk.fullscreen(2, false);
        assert_eq!(desk.stack, [id(2), id(1), id(3)]);
        desk.lower(2);
        assert_eq!(desk.stack, [id(1), id(3), id(2)]);
    }

    #[test]
    fn activating_without_raise_leaves_the_stack_alone() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        let effects = desk.fallback.activate(id(1), false, seat!(desk));
        assert_eq!(effects, [FallbackEffect::Focus(Some(id(1)))]);
        desk.apply(&effects);
        assert_eq!(desk.stack, [id(2), id(1)]);
        assert!(
            desk.fallback.activate(id(1), false, seat!(desk)).is_empty(),
            "activating the focused window is a no-op"
        );
    }

    #[test]
    fn a_parent_with_an_open_modal_hands_focus_to_it() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open_with(2, WindowKind::Modal, Some(1));
        let effects = desk.fallback.activate(id(1), true, seat!(desk));
        assert!(effects.contains(&FallbackEffect::Focus(Some(id(2)))), "{effects:?}");
    }

    // Failure mode 3: a window is placed off every output after hotplug.
    #[test]
    fn unplugging_an_output_keeps_windows_on_a_valid_output() {
        let mut desk = Desk::new();
        let second = Rect::new(1920, 0, 1920, 1080);
        desk.output(1, second);
        desk.open(1);
        desk.open(2);
        desk.fallback.set_maximised(id(1), true);
        desk.fullscreen(2, true);

        let effects = desk.unplug(0);
        for raw in [1, 2] {
            assert_eq!(
                desk.output_of(raw),
                Some(Some(OutputId::new(1))),
                "window {raw} moved to the surviving output"
            );
            assert!(
                configured(&effects, id(raw)).is_some(),
                "window {raw} is configured for its new output"
            );
        }
        assert_eq!(
            desk.geometry(1),
            Some(second),
            "a maximised window takes the new work area"
        );
        assert_eq!(
            desk.geometry(2),
            Some(second),
            "a fullscreen window takes the new monitor rectangle"
        );
    }

    #[test]
    fn unplugging_the_last_output_hides_every_window_and_reconnecting_adopts_them() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.focus(1);

        let effects = desk.unplug(0);
        assert_eq!(desk.output_of(1), Some(None));
        assert_eq!(visibility(&effects, id(1)), Some(false));
        assert!(!desk.fallback.is_visible(id(1)));
        assert_eq!(desk.focus, None, "a window on no output cannot hold focus");

        let effects = desk.output(0, MONITOR);
        assert_eq!(desk.output_of(1), Some(Some(OutputId::new(0))));
        assert_eq!(visibility(&effects, id(1)), Some(true));
        assert_eq!(
            desk.focus,
            Some(id(1)),
            "the adopted window takes the focus nothing held"
        );
    }

    #[test]
    fn an_output_move_reclamps_floating_windows() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.fallback
            .move_resize(id(1), Rect::new(1600, 900, 600, 400))
            .expect("a floating window moves");
        let smaller = Rect::new(0, 0, 1280, 720);
        let effects = desk.fallback.output_moved(OutputId::new(0), smaller);
        assert_eq!(
            configured(&effects, id(1)),
            Some(Rect::new(680, 320, 600, 400)),
            "moved as little as possible to stay on the smaller output"
        );
        assert!(desk.fallback.output_moved(OutputId::new(0), smaller).is_empty());
    }

    // State requests are applied directly.
    #[test]
    fn maximise_fills_the_work_area_and_restores_the_floating_rectangle() {
        let mut desk = Desk::new();
        desk.open(1);
        let floating = desk.geometry(1).expect("open");
        let effects = desk.fallback.set_maximised(id(1), true);
        assert_eq!(configured(&effects, id(1)), Some(MONITOR));
        assert!(
            desk.fallback.set_maximised(id(1), true).is_empty(),
            "maximising twice changes nothing"
        );
        let effects = desk.fallback.set_maximised(id(1), false);
        assert_eq!(configured(&effects, id(1)), Some(floating));
    }

    #[test]
    fn fullscreen_returns_to_the_maximised_or_floating_rectangle() {
        let mut desk = Desk::new();
        desk.open(1);
        let floating = desk.geometry(1).expect("open");
        desk.fallback.reserve(OutputId::new(0), Insets::new(32, 0, 0, 0));
        let area = Rect::new(0, 32, 1920, 1048);

        let effects = desk.fullscreen(1, true);
        assert_eq!(
            configured(&effects, id(1)),
            Some(MONITOR),
            "fullscreen covers the panels"
        );
        let effects = desk.fullscreen(1, false);
        assert_eq!(configured(&effects, id(1)), Some(floating));

        desk.fallback.set_maximised(id(1), true);
        desk.fullscreen(1, true);
        let effects = desk.fullscreen(1, false);
        assert_eq!(
            configured(&effects, id(1)),
            Some(area),
            "a maximised window comes back maximised"
        );
    }

    #[test]
    fn maximising_a_fullscreen_window_is_refused() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.fullscreen(1, true);
        assert!(desk.fallback.set_maximised(id(1), true).is_empty());
        assert_eq!(
            desk.fallback.window(id(1)).map(|window| window.state.maximised),
            Some(false)
        );
    }

    // Failure mode 9: a maximised window drifts from its zone when a panel appears.
    #[test]
    fn reserved_space_reapplies_to_maximised_windows() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.fallback.set_maximised(id(1), true);
        let top = Insets::new(32, 0, 0, 0);
        let effects = desk.fallback.reserve(OutputId::new(0), top);
        assert_eq!(configured(&effects, id(1)), Some(Rect::new(0, 32, 1920, 1048)));
        assert!(desk.fallback.reserve(OutputId::new(0), top).is_empty());
        let effects = desk.fallback.reserve(OutputId::new(0), Insets::ZERO);
        assert_eq!(
            configured(&effects, id(1)),
            Some(MONITOR),
            "a panel that goes gives the space back"
        );
    }

    #[test]
    fn an_oversized_reservation_still_leaves_a_one_pixel_work_area() {
        let mut desk = Desk::new();
        desk.fallback.reserve(OutputId::new(0), Insets::uniform(5000));
        assert_eq!(
            desk.fallback.work_area(OutputId::new(0)).map(Rect::size),
            Some(Size::new(1, 1))
        );
    }

    #[test]
    fn a_control_move_is_kept_inside_the_work_area_and_refused_for_a_maximised_window() {
        let mut desk = Desk::new();
        desk.open(1);
        let effects = desk
            .fallback
            .move_resize(id(1), Rect::new(1800, 1000, 600, 400))
            .expect("a floating window moves");
        assert_eq!(configured(&effects, id(1)), Some(Rect::new(1320, 680, 600, 400)));
        assert_eq!(
            desk.fallback.move_resize(id(9), Rect::new(0, 0, 1, 1)),
            Err(FallbackRefusal::Gone)
        );
        desk.fallback.set_maximised(id(1), true);
        assert_eq!(
            desk.fallback.move_resize(id(1), Rect::new(0, 0, 10, 10)),
            Err(FallbackRefusal::NotFloating)
        );
    }

    #[test]
    fn a_placement_is_exact_where_a_control_move_is_clamped_and_refused_for_a_maximised_window() {
        let mut desk = Desk::new();
        desk.open(1);
        let effects = desk
            .fallback
            .place(id(1), Point::new(1800, 1000))
            .expect("a floating window is placed");
        assert_eq!(
            configured(&effects, id(1)),
            Some(Rect::new(1800, 1000, 600, 400)),
            "the position is kept as asked and the size does not change"
        );
        assert_eq!(desk.fallback.place(id(1), Point::new(1800, 1000)), Ok(Vec::new()));
        assert_eq!(desk.fallback.place(id(9), Point::new(0, 0)), Err(FallbackRefusal::Gone));
        desk.fallback.set_maximised(id(1), true);
        assert_eq!(
            desk.fallback.place(id(1), Point::new(0, 0)),
            Err(FallbackRefusal::NotFloating)
        );
    }

    #[test]
    fn a_transient_follows_its_parent_to_another_output_and_does_not_move_alone() {
        let mut desk = Desk::new();
        desk.output(1, Rect::new(1920, 0, 1920, 1080));
        desk.open(1);
        desk.open_with(2, WindowKind::Dialog, Some(1));
        assert_eq!(
            desk.fallback.move_to_output(id(2), OutputId::new(1)),
            Err(FallbackRefusal::FollowsParent)
        );
        assert_eq!(
            desk.fallback.move_to_output(id(1), OutputId::new(7)),
            Err(FallbackRefusal::UnknownOutput)
        );
        desk.fallback
            .move_to_output(id(1), OutputId::new(1))
            .expect("the parent moves");
        for raw in [1, 2] {
            assert_eq!(desk.output_of(raw), Some(Some(OutputId::new(1))));
        }
    }

    #[test]
    fn closing_a_parent_unlinks_its_dialog() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open_with(2, WindowKind::Dialog, Some(1));
        desk.close(1);
        assert_eq!(
            desk.fallback.window(id(2)).map(|window| window.transient_for),
            Some(None)
        );
    }

    // `window_management.focus_new_windows`.
    #[test]
    fn prevent_stops_a_background_window_stealing_focus_and_allow_does_not() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        assert!(
            desk.fallback
                .may_take_focus(id(1), NewWindowFocus::Prevent, seat!(desk)),
            "nothing is focused yet"
        );
        desk.focus(1);
        assert!(
            !desk
                .fallback
                .may_take_focus(id(2), NewWindowFocus::Prevent, seat!(desk))
        );
        assert!(desk.fallback.may_take_focus(id(2), NewWindowFocus::Allow, seat!(desk)));
        desk.open_with(3, WindowKind::Dialog, Some(1));
        assert!(
            desk.fallback
                .may_take_focus(id(3), NewWindowFocus::Prevent, seat!(desk)),
            "a transient of the focused window may take focus"
        );
        assert!(
            !desk.fallback.may_take_focus(id(9), NewWindowFocus::Allow, seat!(desk)),
            "an unknown window cannot"
        );
    }

    // A client names its parent and its dialog hint after the window opened.
    #[test]
    fn a_window_that_becomes_a_modal_child_stacks_above_its_parent_and_takes_the_parents_focus() {
        let mut desk = Desk::new();
        desk.open(1);
        desk.open(2);
        desk.open(3);
        desk.focus(1);
        assert_eq!(desk.stack, [id(1), id(3), id(2)]);

        let effects = desk
            .fallback
            .set_role(id(3), WindowKind::Modal, Some(id(1)), seat!(desk));
        desk.apply(&effects);
        assert_eq!(desk.stack, [id(3), id(1), id(2)], "directly above its parent");
        assert_eq!(desk.focus, Some(id(3)), "the modal takes the focus its parent held");
        let modal = desk.fallback.window(id(3)).expect("open");
        assert_eq!((modal.kind, modal.transient_for), (WindowKind::Modal, Some(id(1))));

        // An unknown parent changes nothing.
        assert!(
            desk.fallback
                .set_role(id(2), WindowKind::Dialog, Some(id(9)), seat!(desk))
                .is_empty()
        );
        assert_eq!(
            desk.fallback.window(id(2)).map(|window| window.kind),
            Some(WindowKind::Normal)
        );
    }
}
