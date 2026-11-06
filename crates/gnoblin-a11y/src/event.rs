//! What the compositor tells the accessibility machines.

use crate::geometry::{Point, Rect};
use crate::ids::{KeyCode, Keysym, OutputId, OwnerId};
use crate::{Config, Modifiers};

/// One key press or release, as the compositor saw it.
///
/// The compositor supplies the modifier mask it has already applied, because
/// only the input layer has the XKB state. A modifier press this crate withholds
/// is never applied there, so the key mask never contains a withheld modifier
/// and the mask in [`crate::KeyRouting`] is the one the client must see.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Key {
    /// The hardware keycode.
    pub keycode: KeyCode,
    /// The keysym in the current layout.
    pub keysym: Keysym,
    /// The modifier mask the compositor has applied for this event, as X11
    /// reports it in `state`.
    pub modifiers: Modifiers,
    /// The character this key produces, or zero if it produces none.
    pub unichar: u32,
    /// The modifier bits this key contributes, or `None` for a non-modifier
    /// key. Sticky keys needs this rather than a keysym table of its own, so it
    /// cannot disagree with the layout.
    pub modifier: Option<Modifiers>,
}

/// A keystroke the screen reader asked for: a key and the exact modifiers that
/// must be held with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Keystroke {
    /// The non-modifier key.
    pub keysym: Keysym,
    /// The modifier mask the grab matches exactly.
    pub modifiers: Modifiers,
}

/// One output in the layout.
///
/// The magnifier clamps its view into these bounds, so an output with no bounds
/// is a layout the compositor has not finished configuring and never appears
/// here. `bounds` is the output's projected area in the layout plane: a later
/// three-dimensional scene projects into the same field, and the magnifier needs
/// nothing else from it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Output {
    /// The compositor's name for it.
    pub id: OutputId,
    /// Its logical position and size in the layout.
    pub bounds: Rect,
}

/// Everything that can change the accessibility machines' minds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// A configuration load or reload. Fields this crate does not own are
    /// absent because it never reads them.
    Configure(Config),
    /// The settings bridge (D15) republished what apps read about motion and
    /// contrast.
    AppearanceChanged {
        /// Whether animations must be suppressed.
        reduced_motion: bool,
        /// Whether the shell and apps must use a high-contrast theme.
        high_contrast: bool,
    },
    /// A key went down.
    KeyPressed(Key),
    /// A key came up.
    KeyReleased(Key),
    /// Monotonic time advanced to this instant, in milliseconds.
    ///
    /// This is the only clock the crate has. The compositor arms a timer for
    /// each [`crate::Effect::AwaitTick`] it receives; a tick may arrive later
    /// than asked and the machines never assume it arrived on time.
    Tick {
        /// The compositor's monotonic milliseconds.
        now_ms: u64,
    },
    /// The session is covered: no client may type, and no latched modifier may
    /// survive into the lock screen.
    SessionLocked,
    /// The cover is gone.
    SessionUnlocked,
    /// Num Lock changed. Mouse keys steps aside while it is on, because the
    /// keypad then types digits.
    NumLockChanged {
        /// Whether Num Lock is now active.
        active: bool,
    },
    /// A consumer asked to see every key event.
    WatchKeyboard {
        /// The consumer.
        owner: OwnerId,
    },
    /// A consumer stopped watching keys.
    UnwatchKeyboard {
        /// The consumer.
        owner: OwnerId,
    },
    /// A consumer asked to take every key event away from clients.
    GrabKeyboard {
        /// The consumer.
        owner: OwnerId,
    },
    /// A consumer stopped grabbing every key.
    UngrabKeyboard {
        /// The consumer.
        owner: OwnerId,
    },
    /// A consumer replaced its grab list, as `SetKeyGrabs` describes: modifier
    /// keysyms, and keystrokes with the exact modifiers they need.
    SetKeyGrabs {
        /// The consumer.
        owner: OwnerId,
        /// The keysyms to grab as modifiers.
        modifiers: Vec<Keysym>,
        /// The keystrokes to grab.
        keystrokes: Vec<Keystroke>,
    },
    /// A consumer's connection went away.
    MonitorDropped {
        /// The consumer that disappeared.
        owner: OwnerId,
    },
    /// The output layout is now exactly this list.
    OutputsChanged {
        /// Every output that exists now.
        outputs: Vec<Output>,
    },
    /// The focused window moved to this anchor point.
    FocusMoved {
        /// The anchor in layout coordinates.
        at: Point,
    },
    /// The text caret moved to this point.
    CaretMoved {
        /// The anchor in layout coordinates.
        at: Point,
    },
    /// The pointer moved to this point.
    PointerMoved {
        /// The anchor in layout coordinates.
        at: Point,
    },
}
