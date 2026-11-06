//! Pointer-driven focus: which window takes keyboard focus as the pointer moves.
//!
//! Failure modes, each covered by a named test below and in `wm.rs`:
//!
//! - Under `hover`, the pointer crossing the gap between two windows keeps the
//!   current focus. Only `hover-strict` clears focus on a gap, so a gap never
//!   drops focus to nothing by accident.
//! - A focus change while a button is held is ignored. A drag must not steal focus.
//! - Motion cancels a pending rest focus and re-arms it for the new position, so a
//!   pointer still moving never focuses a window it is passing over.
//! - A closed window cancels its pending focus.
//!
//! The module holds no clock. Callers pass the instant of each event, so the rest
//! timer is driven by the same injectable time the rest of the manager uses.

use gnoblin_types::{Tick, WindowId};

/// How long the pointer must stay over a window before `focus_change_on_pointer_rest`
/// focuses it, in milliseconds.
pub const POINTER_REST: u64 = 200;

/// `window_management.focus_mode`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FocusMode {
    /// Focus changes only on a button press.
    #[default]
    Click,
    /// Focus follows the pointer when it enters a window and keeps its focus across gaps.
    Hover,
    /// Like `Hover`, but leaving every window for a gap clears focus.
    HoverStrict,
}

/// What the manager should do after the pointer moved or the clock advanced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerAction {
    /// Nothing changes.
    None,
    /// Focus this window now.
    Focus(WindowId),
    /// Clear keyboard focus, leaving the pointer over no window.
    Clear,
}

/// Pointer position and the rest focus waiting on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PointerFocus {
    /// The window the pointer is over, if any.
    over: Option<WindowId>,
    /// A window whose rest focus is due at the given tick.
    pending: Option<(WindowId, Tick)>,
    /// Whether a button is held. Focus does not follow the pointer while one is.
    grabbed: bool,
}

impl PointerFocus {
    /// The window the pointer last reported being over.
    #[must_use]
    pub const fn over(&self) -> Option<WindowId> {
        self.over
    }

    /// The window waiting to take focus on rest, if any.
    #[must_use]
    pub fn pending(&self) -> Option<WindowId> {
        self.pending.map(|(window, _)| window)
    }

    /// Records a button press or release. Focus is never moved while `active`.
    pub fn set_grab(&mut self, active: bool) {
        self.grabbed = active;
        if active {
            self.pending = None;
        }
    }

    /// Records the pointer over `window` at `at` and returns the action to take now.
    ///
    /// `focused` is the window with keyboard focus. Under `rest`, a focus change is
    /// queued in `pending` and reported later by [`PointerFocus::due`].
    pub fn moved(
        &mut self,
        window: Option<WindowId>,
        at: Tick,
        focused: Option<WindowId>,
        mode: FocusMode,
        rest: bool,
    ) -> PointerAction {
        let entered = window != self.over;
        self.over = window;
        // Any motion cancels a pending rest focus; it is re-armed below if the
        // pointer is still over a window.
        self.pending = None;
        if self.grabbed {
            return PointerAction::None;
        }
        match (mode, window) {
            (FocusMode::Click, _) => PointerAction::None,
            (FocusMode::Hover, None) => PointerAction::None,
            (FocusMode::HoverStrict, None) => match focused {
                Some(_) => PointerAction::Clear,
                None => PointerAction::None,
            },
            (_, Some(id)) => {
                if focused == Some(id) {
                    return PointerAction::None;
                }
                if rest {
                    // Re-armed on every motion, so the timer measures time at rest.
                    let due = at.saturating_add(POINTER_REST);
                    self.pending = Some((id, due));
                    PointerAction::None
                } else if entered {
                    PointerAction::Focus(id)
                } else {
                    PointerAction::None
                }
            }
        }
    }

    /// Takes the pending rest focus if its time has come and the pointer is still over it.
    pub fn due(&mut self, at: Tick) -> PointerAction {
        let Some((window, when)) = self.pending else {
            return PointerAction::None;
        };
        if self.grabbed || !at.reached(when) || self.over != Some(window) {
            return PointerAction::None;
        }
        self.pending = None;
        PointerAction::Focus(window)
    }

    /// Forgets `window`, cancelling any pending focus for it.
    pub fn forget(&mut self, window: WindowId) {
        if self.over == Some(window) {
            self.over = None;
        }
        if self.pending() == Some(window) {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(n: u64) -> WindowId {
        WindowId::new(n)
    }

    fn t(ms: u64) -> Tick {
        Tick::new(ms)
    }

    #[test]
    fn click_mode_never_moves_focus_on_motion() {
        let mut p = PointerFocus::default();
        assert_eq!(
            p.moved(Some(w(1)), t(0), None, FocusMode::Click, false),
            PointerAction::None
        );
    }

    #[test]
    fn hover_focuses_on_enter_and_keeps_focus_across_a_gap() {
        let mut p = PointerFocus::default();
        assert_eq!(
            p.moved(Some(w(1)), t(0), None, FocusMode::Hover, false),
            PointerAction::Focus(w(1))
        );
        assert_eq!(
            p.moved(None, t(5), Some(w(1)), FocusMode::Hover, false),
            PointerAction::None
        );
    }

    #[test]
    fn hover_strict_clears_focus_on_a_gap() {
        let mut p = PointerFocus::default();
        p.moved(Some(w(1)), t(0), None, FocusMode::HoverStrict, false);
        assert_eq!(
            p.moved(None, t(5), Some(w(1)), FocusMode::HoverStrict, false),
            PointerAction::Clear
        );
    }

    #[test]
    fn a_grab_suppresses_focus_changes() {
        let mut p = PointerFocus::default();
        p.set_grab(true);
        assert_eq!(
            p.moved(Some(w(1)), t(0), None, FocusMode::Hover, false),
            PointerAction::None
        );
        assert_eq!(
            p.moved(None, t(5), Some(w(1)), FocusMode::HoverStrict, false),
            PointerAction::None
        );
    }

    #[test]
    fn rest_focus_waits_for_the_pointer_to_settle() {
        let mut p = PointerFocus::default();
        assert_eq!(
            p.moved(Some(w(1)), t(0), None, FocusMode::Hover, true),
            PointerAction::None
        );
        assert_eq!(p.due(t(POINTER_REST - 1)), PointerAction::None);
        assert_eq!(p.due(t(POINTER_REST)), PointerAction::Focus(w(1)));
    }

    #[test]
    fn motion_cancels_the_pending_rest_timer() {
        let mut p = PointerFocus::default();
        p.moved(Some(w(1)), t(0), None, FocusMode::Hover, true);
        // Motion within the same window re-arms the timer from the new position.
        assert_eq!(
            p.moved(Some(w(1)), t(100), None, FocusMode::Hover, true),
            PointerAction::None
        );
        assert_eq!(p.pending(), Some(w(1)));
        assert_eq!(p.due(t(POINTER_REST)), PointerAction::None);
        assert_eq!(p.due(t(100 + POINTER_REST)), PointerAction::Focus(w(1)));
    }

    #[test]
    fn leaving_the_window_cancels_the_pending_focus() {
        let mut p = PointerFocus::default();
        p.moved(Some(w(1)), t(0), None, FocusMode::Hover, true);
        p.moved(None, t(50), None, FocusMode::Hover, true);
        assert_eq!(p.pending(), None);
        assert_eq!(p.due(t(POINTER_REST)), PointerAction::None);
    }

    #[test]
    fn a_closed_window_cancels_its_pending_focus() {
        let mut p = PointerFocus::default();
        p.moved(Some(w(1)), t(0), None, FocusMode::Hover, true);
        p.forget(w(1));
        assert_eq!(p.due(t(POINTER_REST)), PointerAction::None);
    }
}
