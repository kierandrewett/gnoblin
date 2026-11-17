//! [`hand_over`]: how a session changes its window-manager provider while windows are open.
//!
//! A provider is registered after the session started (std loads when a shell asks for it), and it ends when the generation
//! that registered it retires. Either way the compositor holds windows and outputs the new provider has never heard of, whose
//! clients already hold a rectangle and a state. The hand-over replays the old record into the new provider so it holds the
//! same windows, and returns only the effects that would otherwise leave a client out of step: a window the new provider
//! placed or showed differently from where it was.

use gnoblin_types::WindowId;

use crate::manager::WindowManager;
use crate::window::{Open, OutputRecord, Standing, Window, WmEffect};

/// What a provider holds, copied out of it: the windows and the outputs, with nothing that calls back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldRecord {
    /// Every window, by identity.
    pub windows: Vec<Window>,
    /// Every connected output, in the order they arrived.
    pub outputs: Vec<OutputRecord>,
}

impl HeldRecord {
    /// What `wm` holds now.
    #[must_use]
    pub fn of(wm: &dyn WindowManager) -> Self {
        Self {
            windows: wm.windows().cloned().collect(),
            outputs: wm.outputs().copied().collect(),
        }
    }

    /// Whether a window of this record would be drawn: it is on an output and not minimised.
    fn visible(window: &Window) -> bool {
        window.output.is_some() && !window.state.minimised
    }
}

/// Teaches `into`, a provider with an empty record, what `from` held, and returns the effects that bring the clients to
/// what `into` decided where it differs from `from`.
///
/// Outputs go first, with the insets reserved on them, then windows with their parents before their transients, each placed
/// at the rectangle it floats at and then put into the state it is in. A window `into` refuses is left out of its record.
#[must_use]
pub fn hand_over(from: &HeldRecord, into: &mut dyn WindowManager, seat: Standing<'_>) -> Vec<WmEffect> {
    for output in &from.outputs {
        let _ = into.output_added(output.id, output.rect, seat);
        if output.reserved != gnoblin_types::Insets::ZERO {
            let _ = into.reserve(output.id, output.reserved);
        }
    }
    let mut windows = from.windows.clone();
    // A parent opens before its transients, so a dialog is never opened as an orphan.
    windows.sort_by_key(|window| window.transient_for.is_some());
    for window in &windows {
        let open = Open {
            id: window.id,
            kind: window.kind,
            transient_for: window.transient_for,
            requested: Some(window.restore),
        };
        if into.open(open, seat).is_err() {
            continue;
        }
        let _ = into.set_rect(window.id, window.restore);
        if window.state.maximised {
            let _ = into.set_maximised(window.id, true);
        }
        if window.state.fullscreen {
            let _ = into.set_fullscreen(window.id, true, seat);
        }
        if window.state.minimised {
            let _ = into.set_minimised(window.id, true, seat);
        }
    }
    let mut effects = Vec::new();
    for window in &windows {
        let Some(placed) = into.window(window.id) else {
            continue;
        };
        if placed.geometry != window.geometry || placed.state != window.state {
            effects.push(WmEffect::Configure {
                window: window.id,
                geometry: placed.geometry,
                state: placed.state,
            });
        }
        let visible = into.is_visible(window.id);
        if visible != HeldRecord::visible(window) {
            effects.push(WmEffect::Visible {
                window: window.id,
                visible,
            });
        }
    }
    effects
}

/// The windows `from` held that `into` does not, for a caller that wants to know what a hand-over dropped.
#[must_use]
pub fn dropped(from: &HeldRecord, into: &dyn WindowManager) -> Vec<WindowId> {
    from.windows
        .iter()
        .map(|window| window.id)
        .filter(|id| into.window(*id).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use gnoblin_types::{Insets, OutputId, Rect};

    use super::*;
    use crate::{NullWm, WindowKind};

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    fn standing() -> Standing<'static> {
        Standing {
            stack: &[],
            focus: None,
        }
    }

    fn id(raw: u64) -> WindowId {
        WindowId::new(raw)
    }

    fn open(wm: &mut NullWm, window: u64, parent: Option<u64>) {
        wm.open(
            Open {
                id: id(window),
                kind: if parent.is_some() {
                    WindowKind::Dialog
                } else {
                    WindowKind::Normal
                },
                transient_for: parent.map(id),
                requested: Some(Rect::new(0, 0, 400, 300)),
            },
            standing(),
        )
        .expect("opens");
    }

    #[test]
    fn the_new_provider_holds_every_output_and_window_the_old_one_did() {
        let mut old = NullWm::default();
        old.output_added(OutputId::new(1), MONITOR, standing());
        old.reserve(
            OutputId::new(1),
            Insets {
                top: 30,
                ..Insets::ZERO
            },
        );
        // The transient is listed before its parent by identity.
        open(&mut old, 2, None);
        open(&mut old, 1, Some(2));
        old.move_resize(id(2), Rect::new(50, 60, 500, 400)).expect("floats");
        old.set_maximised(id(1), true);

        let mut new = NullWm::default();
        let record = HeldRecord::of(&old);
        let effects = hand_over(&record, &mut new, standing());

        assert_eq!(new.outputs().count(), 1);
        assert_eq!(new.output(OutputId::new(1)).expect("held").reserved.top, 30);
        assert_eq!(new.window(id(2)).expect("held").geometry, Rect::new(50, 60, 500, 400));
        assert!(new.window(id(1)).expect("held").state.maximised);
        assert_eq!(new.window(id(1)).expect("held").transient_for, Some(id(2)));
        assert!(
            effects.is_empty(),
            "a provider that agrees tells no client anything: {effects:?}"
        );
        assert!(dropped(&record, &new).is_empty());
    }
}
