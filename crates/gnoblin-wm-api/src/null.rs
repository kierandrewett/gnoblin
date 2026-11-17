//! [`NullWm`]: the window manager of a core that has no provider.
//!
//! It is the least a compositor can do and still run. A new window is placed at the origin of the first output at its
//! natural size and keeps the paint order it arrived in; nothing is cascaded, tiled or kept inside the work area. It has no
//! focus policy: a click focuses nothing, a new window takes no focus, and the compositor never moves keyboard focus on its
//! own. Focus belongs to whoever holds the tree, and a core-only session with no shell has no focused window.
//!
//! It refuses nothing a request can reasonably ask for and drops none: a maximise, fullscreen or minimise is recorded and
//! answered with the window's new rectangle, and an explicit move or resize is applied as asked. Only a window that is
//! already open, or one past [`MAX_WINDOWS`], is refused.
//!
//! The record of windows and outputs is kept in full, because the compositor reads it for every window whichever provider
//! is installed.

use std::collections::BTreeMap;

use gnoblin_types::{Insets, OutputId, Point, Rect, Size, WindowId};

use crate::manager::WindowManager;
use crate::policy::NewWindowFocus;
use crate::window::{
    DEFAULT_SIZE, MAX_WINDOWS, Open, OutputRecord, Standing, Window, WindowKind, WindowState, WmEffect, WmRefusal,
};

/// The provider of a core with none: windows at the origin, no focus policy.
#[derive(Debug, Default)]
pub struct NullWm {
    windows: BTreeMap<WindowId, Window>,
    outputs: Vec<OutputRecord>,
}

impl NullWm {
    /// The configure that tells a window's client where it is and what state it is in.
    fn configured(window: &Window) -> WmEffect {
        WmEffect::Configure {
            window: window.id,
            geometry: window.geometry,
            state: window.state,
        }
    }

    /// Where a window belongs on `output` for its state: the monitor when fullscreen, the work area when maximised and
    /// the rectangle it already has otherwise.
    fn fitted(window: &Window, output: &OutputRecord) -> Rect {
        if window.state.fullscreen {
            output.rect
        } else if window.state.maximised {
            output.work_area()
        } else {
            window.geometry
        }
    }

    /// Re-fits every window on `output`, reporting the ones that moved.
    fn refit(&mut self, output: &OutputRecord) -> Vec<WmEffect> {
        let mut effects = Vec::new();
        for window in self
            .windows
            .values_mut()
            .filter(|window| window.output == Some(output.id))
        {
            let fitted = Self::fitted(window, output);
            if fitted != window.geometry {
                window.geometry = fitted;
                effects.push(Self::configured(window));
            }
        }
        effects
    }

    /// The paint order after `change`, with a window the caller has not listed yet on top, as a [`WmEffect::Stack`] when it
    /// differs from the one the caller holds.
    fn restacked(&self, seat: Standing<'_>, change: impl FnOnce(&mut Vec<WindowId>)) -> Vec<WmEffect> {
        let mut order = seat.stack.to_vec();
        for id in self.windows.keys().rev() {
            if !order.contains(id) {
                order.insert(0, *id);
            }
        }
        change(&mut order);
        if order == seat.stack {
            Vec::new()
        } else {
            vec![WmEffect::Stack(order)]
        }
    }

    /// Sets a floating window's rectangle, with no constraint.
    fn set_floating(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
        let window = self.windows.get_mut(&id).ok_or(WmRefusal::Gone)?;
        if window.state.maximised || window.state.fullscreen {
            return Err(WmRefusal::NotFloating);
        }
        if wanted == window.geometry {
            return Ok(Vec::new());
        }
        window.geometry = wanted;
        window.restore = wanted;
        Ok(vec![Self::configured(window)])
    }
}

impl WindowManager for NullWm {
    fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    fn windows(&self) -> Box<dyn Iterator<Item = &Window> + '_> {
        Box::new(self.windows.values())
    }

    fn output(&self, id: OutputId) -> Option<&OutputRecord> {
        self.outputs.iter().find(|output| output.id == id)
    }

    fn outputs(&self) -> Box<dyn Iterator<Item = &OutputRecord> + '_> {
        Box::new(self.outputs.iter())
    }

    fn set_role(
        &mut self,
        id: WindowId,
        kind: WindowKind,
        transient_for: Option<WindowId>,
        _seat: Standing<'_>,
    ) -> Vec<WmEffect> {
        let parent = transient_for.filter(|parent| *parent != id);
        if parent.is_some_and(|parent| !self.windows.contains_key(&parent)) {
            return Vec::new();
        }
        if let Some(window) = self.windows.get_mut(&id) {
            window.kind = kind;
            window.transient_for = parent;
        }
        Vec::new()
    }

    /// Never: this provider has no focus policy, so a new window takes none.
    fn may_take_focus(&self, _id: WindowId, _policy: NewWindowFocus, _seat: Standing<'_>) -> bool {
        false
    }

    fn open(&mut self, open: Open, _seat: Standing<'_>) -> Result<Vec<WmEffect>, WmRefusal> {
        if self.windows.contains_key(&open.id) {
            return Err(WmRefusal::Duplicate);
        }
        if self.windows.len() >= MAX_WINDOWS {
            return Err(WmRefusal::TooMany);
        }
        let home = self.outputs.first().copied();
        let size = open.requested.map_or(DEFAULT_SIZE, Rect::size).at_least_one();
        let origin = home.map_or(Point::new(0, 0), |home| Point::new(home.rect.x, home.rect.y));
        let geometry = Rect::new(origin.x, origin.y, size.width, size.height);
        let transient_for = open
            .transient_for
            .filter(|parent| *parent != open.id && self.windows.contains_key(parent));
        self.windows.insert(
            open.id,
            Window {
                id: open.id,
                kind: open.kind,
                transient_for,
                output: home.map(|home| home.id),
                geometry,
                restore: geometry,
                state: WindowState::default(),
            },
        );
        Ok(vec![
            WmEffect::Configure {
                window: open.id,
                geometry,
                state: WindowState::default(),
            },
            WmEffect::Visible {
                window: open.id,
                visible: home.is_some(),
            },
        ])
    }

    fn close(&mut self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        if self.windows.remove(&id).is_none() {
            return Vec::new();
        }
        for window in self.windows.values_mut() {
            if window.transient_for == Some(id) {
                window.transient_for = None;
            }
        }
        // No window takes the focus over: the compositor must not keep pointing at one that is gone.
        if seat.focus == Some(id) {
            vec![WmEffect::Focus(None)]
        } else {
            Vec::new()
        }
    }

    /// Changes nothing: a click or an activation request moves no focus and raises nothing under this provider.
    fn activate(&self, _id: WindowId, _raise: bool, _seat: Standing<'_>) -> Vec<WmEffect> {
        Vec::new()
    }

    fn raise(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        if !self.windows.contains_key(&id) {
            return Vec::new();
        }
        self.restacked(seat, |order| {
            order.retain(|window| *window != id);
            order.insert(0, id);
        })
    }

    fn lower(&self, id: WindowId, seat: Standing<'_>) -> Vec<WmEffect> {
        if !self.windows.contains_key(&id) {
            return Vec::new();
        }
        self.restacked(seat, |order| {
            order.retain(|window| *window != id);
            order.push(id);
        })
    }

    fn set_maximised(&mut self, id: WindowId, on: bool) -> Vec<WmEffect> {
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
            window.geometry = window.restore;
        }
        vec![Self::configured(window)]
    }

    fn set_fullscreen(&mut self, id: WindowId, on: bool, _seat: Standing<'_>) -> Vec<WmEffect> {
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
        } else if window.state.maximised {
            window.geometry = output.work_area();
        } else {
            window.geometry = window.restore;
        }
        vec![Self::configured(window)]
    }

    fn set_minimised(&mut self, id: WindowId, on: bool, seat: Standing<'_>) -> Vec<WmEffect> {
        let Some(window) = self.windows.get_mut(&id) else {
            return Vec::new();
        };
        if window.state.minimised == on {
            return Vec::new();
        }
        window.state.minimised = on;
        let mut effects = vec![WmEffect::Visible {
            window: id,
            visible: !on && window.output.is_some(),
        }];
        // A hidden window cannot keep the focus; nothing else is chosen in its place.
        if on && seat.focus == Some(id) {
            effects.push(WmEffect::Focus(None));
        }
        effects
    }

    fn move_resize(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
        self.set_floating(id, wanted.at_least_one())
    }

    fn place(&mut self, id: WindowId, at: Point) -> Result<Vec<WmEffect>, WmRefusal> {
        let size = self.windows.get(&id).ok_or(WmRefusal::Gone)?.geometry;
        self.set_floating(id, Rect::new(at.x, at.y, size.width, size.height))
    }

    fn set_rect(&mut self, id: WindowId, wanted: Rect) -> Result<Vec<WmEffect>, WmRefusal> {
        self.set_floating(id, wanted.at_least_one())
    }

    fn adopt_size(&mut self, id: WindowId, size: Size) -> bool {
        let Some(window) = self.windows.get_mut(&id) else {
            return false;
        };
        let size = size.at_least_one();
        if window.state.maximised || window.state.fullscreen || window.geometry.size() == size {
            return false;
        }
        window.geometry = Rect::new(window.geometry.x, window.geometry.y, size.width, size.height);
        window.restore = window.geometry;
        true
    }

    fn move_to_output(&mut self, id: WindowId, target: OutputId) -> Result<Vec<WmEffect>, WmRefusal> {
        if !self.windows.contains_key(&id) {
            return Err(WmRefusal::Gone);
        }
        let output = *self.output(target).ok_or(WmRefusal::UnknownOutput)?;
        let mut effects = Vec::new();
        for window in self
            .windows
            .values_mut()
            .filter(|window| window.id == id || window.transient_for == Some(id))
        {
            if window.output == Some(target) {
                continue;
            }
            window.output = Some(target);
            window.geometry = Self::fitted(window, &output);
            effects.push(Self::configured(window));
        }
        Ok(effects)
    }

    fn output_added(&mut self, id: OutputId, rect: Rect, _seat: Standing<'_>) -> Vec<WmEffect> {
        if self.output(id).is_some() {
            return Vec::new();
        }
        let output = OutputRecord {
            id,
            rect: rect.at_least_one(),
            reserved: Insets::ZERO,
        };
        self.outputs.push(output);
        // A window left on no output is adopted by the first one that arrives.
        let mut effects = Vec::new();
        for window in self.windows.values_mut().filter(|window| window.output.is_none()) {
            window.output = Some(id);
            window.geometry = Self::fitted(window, &output);
            effects.push(WmEffect::Visible {
                window: window.id,
                visible: !window.state.minimised,
            });
            effects.push(Self::configured(window));
        }
        effects
    }

    fn output_removed(&mut self, id: OutputId, _seat: Standing<'_>) -> Vec<WmEffect> {
        let Some(index) = self.outputs.iter().position(|output| output.id == id) else {
            return Vec::new();
        };
        self.outputs.remove(index);
        let survivor = self.outputs.first().copied();
        let mut effects = Vec::new();
        for window in self.windows.values_mut().filter(|window| window.output == Some(id)) {
            window.output = survivor.map(|survivor| survivor.id);
            match survivor {
                Some(survivor) => {
                    window.geometry = Self::fitted(window, &survivor);
                    effects.push(Self::configured(window));
                }
                None => effects.push(WmEffect::Visible {
                    window: window.id,
                    visible: false,
                }),
            }
        }
        effects
    }

    fn output_moved(&mut self, id: OutputId, rect: Rect) -> Vec<WmEffect> {
        let Some(output) = self.outputs.iter_mut().find(|output| output.id == id) else {
            return Vec::new();
        };
        output.rect = rect.at_least_one();
        let output = *output;
        self.refit(&output)
    }

    fn reserve(&mut self, id: OutputId, reserved: Insets) -> Vec<WmEffect> {
        let Some(output) = self.outputs.iter_mut().find(|output| output.id == id) else {
            return Vec::new();
        };
        output.reserved = reserved;
        let output = *output;
        self.refit(&output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    fn id(raw: u64) -> WindowId {
        WindowId::new(raw)
    }

    fn standing(stack: &[WindowId], focus: Option<WindowId>) -> Standing<'_> {
        Standing { stack, focus }
    }

    fn open(wm: &mut NullWm, window: u64, requested: Option<Rect>) -> Vec<WmEffect> {
        let open = Open {
            id: id(window),
            kind: WindowKind::Normal,
            transient_for: None,
            requested,
        };
        wm.open(open, standing(&[], None)).expect("the window opens")
    }

    fn with_output() -> NullWm {
        let mut wm = NullWm::default();
        wm.output_added(OutputId::new(1), MONITOR, standing(&[], None));
        wm
    }

    #[test]
    fn a_new_window_is_placed_at_the_origin_at_its_natural_size() {
        let mut wm = with_output();
        let effects = open(&mut wm, 1, Some(Rect::new(300, 200, 640, 480)));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 0, 640, 480));
        assert_eq!(
            effects,
            vec![
                WmEffect::Configure {
                    window: id(1),
                    geometry: Rect::new(0, 0, 640, 480),
                    state: WindowState::default()
                },
                WmEffect::Visible {
                    window: id(1),
                    visible: true
                },
            ]
        );
        open(&mut wm, 2, None);
        assert_eq!(
            wm.window(id(2)).expect("open").geometry,
            Rect::new(0, 0, 800, 600),
            "a window that asked for no size gets the default one, at the origin too: nothing cascades"
        );
    }

    #[test]
    fn a_new_window_is_not_restacked_so_it_keeps_the_order_it_arrived_in() {
        let mut wm = with_output();
        let effects = open(&mut wm, 1, None);
        assert!(
            !effects.iter().any(|effect| matches!(effect, WmEffect::Stack(_))),
            "{effects:?}"
        );
    }

    #[test]
    fn it_has_no_focus_policy() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        open(&mut wm, 2, None);
        let stack = [id(2), id(1)];
        assert!(
            !wm.may_take_focus(id(2), NewWindowFocus::Allow, standing(&stack, None)),
            "a new window takes no focus, whatever the option says"
        );
        assert_eq!(
            wm.activate(id(1), true, standing(&stack, Some(id(2)))),
            Vec::<WmEffect>::new()
        );
    }

    #[test]
    fn closing_the_focused_window_clears_the_focus_and_picks_no_other() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        open(&mut wm, 2, None);
        let stack = [id(2), id(1)];
        assert_eq!(
            wm.close(id(2), standing(&stack, Some(id(2)))),
            vec![WmEffect::Focus(None)]
        );
        assert_eq!(wm.close(id(1), standing(&[id(1)], None)), Vec::<WmEffect>::new());
        assert!(wm.windows().next().is_none());
    }

    #[test]
    fn a_window_arriving_before_any_output_waits_for_the_first_one() {
        let mut wm = NullWm::default();
        let effects = open(&mut wm, 1, None);
        assert_eq!(wm.window(id(1)).expect("open").output, None);
        assert!(effects.contains(&WmEffect::Visible {
            window: id(1),
            visible: false
        }));
        let adopted = wm.output_added(
            OutputId::new(7),
            Rect::new(1920, 0, 1280, 720),
            standing(&[id(1)], None),
        );
        assert_eq!(wm.window(id(1)).expect("open").output, Some(OutputId::new(7)));
        assert!(adopted.contains(&WmEffect::Visible {
            window: id(1),
            visible: true
        }));
    }

    #[test]
    fn a_window_opens_at_the_origin_of_the_first_output_not_the_desktop() {
        let mut wm = NullWm::default();
        wm.output_added(OutputId::new(1), Rect::new(1920, 0, 1280, 720), standing(&[], None));
        open(&mut wm, 1, Some(Rect::new(5, 5, 100, 100)));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(1920, 0, 100, 100));
    }

    #[test]
    fn requests_are_applied_as_asked_with_no_constraint() {
        let mut wm = with_output();
        open(&mut wm, 1, Some(Rect::new(0, 0, 400, 300)));
        wm.set_rect(id(1), Rect::new(-50, 4000, 400, 300)).expect("floating");
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(-50, 4000, 400, 300));
        wm.place(id(1), Point::new(10, 20)).expect("floating");
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(10, 20, 400, 300));
        assert_eq!(wm.set_rect(id(9), MONITOR), Err(WmRefusal::Gone));
    }

    #[test]
    fn maximise_and_fullscreen_are_recorded_and_undone() {
        let mut wm = with_output();
        open(&mut wm, 1, Some(Rect::new(0, 0, 400, 300)));
        let maximised = wm.set_maximised(id(1), true);
        assert_eq!(wm.window(id(1)).expect("open").geometry, MONITOR);
        assert!(matches!(maximised.as_slice(), [WmEffect::Configure { state, .. }] if state.maximised));
        wm.set_maximised(id(1), false);
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 0, 400, 300));
        wm.set_fullscreen(id(1), true, standing(&[id(1)], None));
        assert_eq!(wm.window(id(1)).expect("open").geometry, MONITOR);
        assert_eq!(wm.set_rect(id(1), MONITOR), Err(WmRefusal::NotFloating));
        wm.set_fullscreen(id(1), false, standing(&[id(1)], None));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 0, 400, 300));
    }

    #[test]
    fn minimise_hides_the_window_and_drops_its_focus() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        let hidden = wm.set_minimised(id(1), true, standing(&[id(1)], Some(id(1))));
        assert_eq!(
            hidden,
            vec![
                WmEffect::Visible {
                    window: id(1),
                    visible: false
                },
                WmEffect::Focus(None)
            ]
        );
        assert!(!wm.is_visible(id(1)));
    }

    #[test]
    fn raise_and_lower_restack_only_when_asked() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        open(&mut wm, 2, None);
        let stack = [id(2), id(1)];
        assert_eq!(
            wm.raise(id(1), standing(&stack, None)),
            vec![WmEffect::Stack(vec![id(1), id(2)])]
        );
        assert_eq!(wm.raise(id(2), standing(&stack, None)), Vec::<WmEffect>::new());
        assert_eq!(
            wm.lower(id(2), standing(&stack, None)),
            vec![WmEffect::Stack(vec![id(1), id(2)])]
        );
    }

    #[test]
    fn losing_an_output_moves_its_windows_to_the_survivor_or_hides_them() {
        let mut wm = with_output();
        wm.output_added(OutputId::new(2), Rect::new(1920, 0, 1280, 720), standing(&[], None));
        open(&mut wm, 1, None);
        wm.move_to_output(id(1), OutputId::new(2)).expect("a connected output");
        assert_eq!(wm.window(id(1)).expect("open").output, Some(OutputId::new(2)));
        wm.output_removed(OutputId::new(2), standing(&[id(1)], None));
        assert_eq!(wm.window(id(1)).expect("open").output, Some(OutputId::new(1)));
        let hidden = wm.output_removed(OutputId::new(1), standing(&[id(1)], None));
        assert_eq!(wm.window(id(1)).expect("open").output, None);
        assert_eq!(
            hidden,
            vec![WmEffect::Visible {
                window: id(1),
                visible: false
            }]
        );
        assert_eq!(
            wm.move_to_output(id(1), OutputId::new(9)),
            Err(WmRefusal::UnknownOutput)
        );
    }

    #[test]
    fn a_maximised_window_follows_the_output_it_is_on() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        wm.set_maximised(id(1), true);
        wm.reserve(OutputId::new(1), Insets::new(32, 0, 0, 0));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 32, 1920, 1048));
        wm.output_moved(OutputId::new(1), Rect::new(0, 0, 1280, 720));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 32, 1280, 688));
    }

    #[test]
    fn the_record_is_bounded_and_refuses_a_window_twice() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        let again = Open {
            id: id(1),
            kind: WindowKind::Normal,
            transient_for: None,
            requested: None,
        };
        assert_eq!(wm.open(again, standing(&[], None)).err(), Some(WmRefusal::Duplicate));
        for window in 2..=MAX_WINDOWS as u64 {
            open(&mut wm, window, None);
        }
        let past = Open {
            id: id(MAX_WINDOWS as u64 + 1),
            ..again
        };
        assert_eq!(wm.open(past, standing(&[], None)).err(), Some(WmRefusal::TooMany));
    }

    #[test]
    fn adopting_the_clients_size_keeps_the_position() {
        let mut wm = with_output();
        open(&mut wm, 1, None);
        assert!(wm.adopt_size(id(1), Size::new(321, 123)));
        assert_eq!(wm.window(id(1)).expect("open").geometry, Rect::new(0, 0, 321, 123));
        assert!(
            !wm.adopt_size(id(1), Size::new(321, 123)),
            "an unchanged size changes nothing"
        );
    }
}
