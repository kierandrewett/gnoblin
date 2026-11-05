//! What the accessibility machines ask the compositor to do.
//!
//! Effects are in the order they must happen. Within one key event the order is
//! always: a swallow before a delivery, a pointer move before the tick that
//! re-arms the motion loop, and one publish last.

use crate::geometry::{Rect, Scale};
use crate::ids::{OutputId, OwnerId};
use crate::{Key, Mode, Modifiers, Sticky};

/// Who sees a key event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyDecision {
    /// The focused client sees the key and no consumer asked for it.
    PassThrough,
    /// The focused client and at least one consumer see it: a watch without a
    /// grab.
    Both,
    /// A grab took the key, so the focused client must not see it. Every
    /// consumer listed in [`KeyRouting::monitors`] still receives it.
    Consumed,
}

/// One key event that reaches the session, and who gets it.
///
/// A key this crate withholds or rejects produces no `Key` effect at all, so
/// "the client must not see it" and "nothing happened" are the same answer and a
/// wiring mistake cannot invent a delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRouting {
    /// Whether this is the key-up half.
    pub released: bool,
    /// The key itself, for the monitor's `KeyEvent` signal and for the client.
    pub key: Key,
    /// The modifier mask the client and the monitor see, with sticky keys
    /// applied.
    pub modifiers: Modifiers,
    /// Who sees it.
    pub decision: KeyDecision,
    /// The consumers that receive it, in owner order. Empty unless the decision
    /// is [`KeyDecision::Both`] or [`KeyDecision::Consumed`].
    pub monitors: Vec<OwnerId>,
}

/// A pointer button, in the order libinput numbers them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerButton {
    /// Button 1.
    Primary,
    /// Button 2.
    Middle,
    /// Button 3.
    Secondary,
}

/// What a mouse-keys click does to a button.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonAction {
    /// Hold the button down.
    Press,
    /// Let the button up.
    Release,
}

impl PointerButton {
    /// The libinput button number.
    #[must_use]
    pub const fn number(self) -> u32 {
        match self {
            Self::Primary => 1,
            Self::Middle => 2,
            Self::Secondary => 3,
        }
    }

    /// The slot this button occupies in the machine's pressed table, so the
    /// table is a small array instead of a map.
    #[must_use]
    pub(crate) const fn slot(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Middle => 1,
            Self::Secondary => 2,
        }
    }
}

/// Why a beep is due, which is exactly the set of `beep_on_*` settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeepReason {
    /// A modifier was latched, locked or cleared.
    Modifier,
    /// A slow key started its delay.
    Press,
    /// A slow key was accepted.
    Accept,
    /// A key was rejected by slow keys, bounce keys or a two-key turn.
    Reject,
}

/// What apps are told about motion and contrast (D15).
///
/// The renderer honours these; this crate only publishes them, because a native
/// animation that ignores the flag fails in `gnoblin-render`, not here.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Appearance {
    /// Whether animations must be suppressed.
    pub reduced_motion: bool,
    /// Whether the high-contrast theme is on.
    pub high_contrast: bool,
}

/// The area of one output the magnifier shows.
///
/// `source` is always inside `output`'s bounds: that is the property the whole
/// magnifier model exists to guarantee. It is a projected area of the output's
/// surface, so it stays two-dimensional whatever depth the scene behind it has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct View {
    /// The output the view belongs to.
    pub output: OutputId,
    /// The area of that output that is magnified.
    pub source: Rect,
    /// The factor applied to it.
    pub zoom: Scale,
    /// How the renderer paints it.
    pub mode: Mode,
}

/// The state observers see: `gnoblin.accessibility` on the IPC and every
/// settings bridge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Published {
    /// Motion and contrast, from the settings bridge.
    pub appearance: Appearance,
    /// Sticky keys, including any latch that is held right now.
    pub sticky: Sticky,
    /// Whether slow keys is on.
    pub slow_keys: bool,
    /// Whether bounce keys is on.
    pub bounce_keys: bool,
    /// Whether mouse keys is on.
    pub mouse_keys: bool,
    /// The magnifier's view, or `None` when it is off.
    pub magnifier: Option<View>,
    /// Whether the session is covered.
    pub locked: bool,
}

/// One instruction, produced by [`crate::A11y::apply`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    /// A key event reaches the session. The decision says who must see it.
    Key(KeyRouting),
    /// Move the pointer by this many whole pixels, from mouse keys.
    PointerMotion {
        /// Horizontal distance, positive rightwards.
        dx: i64,
        /// Vertical distance, positive downwards.
        dy: i64,
    },
    /// Press or release a pointer button, from mouse keys.
    PointerButton {
        /// Which button.
        button: PointerButton,
        /// What happens to it.
        action: ButtonAction,
    },
    /// Ask the compositor's timer to deliver [`crate::Event::Tick`] no earlier
    /// than this instant. A later tick is allowed.
    AwaitTick {
        /// The earliest instant, in the compositor's milliseconds.
        at_ms: u64,
    },
    /// Beep for auditory feedback.
    Beep {
        /// Why, which selects the setting that allowed it.
        reason: BeepReason,
    },
    /// The magnifier's view when it changed, or `None` when it was turned off.
    Magnifier(Option<View>),
    /// Publish the state to observers.
    Publish(Published),
}
