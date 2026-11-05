//! The accessibility configuration, with the field names, ranges and defaults
//! of the C tree.
//!
//! Every keyboard field maps one to one onto `docs/config/configure/input/keyboard.md`
//! and the `keyboard_accessibility_fields` table in
//! `src/config/gnoblin-config-input.c`, so a configuration file written for
//! Gnoblin 0.1.x reads the same after the cutover. The magnifier is new state in
//! this crate; its settings are the ones `gnoblin-render` needs and nothing
//! else.

use crate::geometry::Zoom;

/// Everything under `accessibility`, plus the magnifier.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Config {
    /// Sticky keys.
    pub sticky_keys: StickyKeysConfig,
    /// Slow keys.
    pub slow_keys: SlowKeysConfig,
    /// Bounce keys.
    pub bounce_keys: BounceKeysConfig,
    /// Mouse keys.
    pub mouse_keys: MouseKeysConfig,
    /// The magnifier.
    pub magnifier: MagnifierConfig,
}

/// `sticky_keys`: one press latches a modifier, two lock it, three clear it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StickyKeysConfig {
    /// Whether sticky keys is on in the configuration. A two-key-off turn can
    /// disable it at run time until this value changes again.
    pub enabled: bool,
    /// Whether pressing two modifiers together turns sticky keys off.
    pub two_key_off: bool,
    /// Whether latching or locking a modifier beeps.
    pub beep_on_modifier: bool,
}

/// `slow_keys`: a press is accepted only after the key is held for a delay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlowKeysConfig {
    /// Whether slow keys holds presses.
    pub enabled: bool,
    /// How long a key must be held before the press is accepted.
    pub delay_ms: u64,
    /// Whether a press that starts the delay beeps.
    pub beep_on_press: bool,
    /// Whether an accepted press beeps.
    pub beep_on_accept: bool,
    /// Whether a press released too early beeps.
    pub beep_on_reject: bool,
}

impl Default for SlowKeysConfig {
    /// The GNOME schema defaults: off, with a 300 ms delay.
    fn default() -> Self {
        Self {
            enabled: false,
            delay_ms: 300,
            beep_on_press: false,
            beep_on_accept: false,
            beep_on_reject: false,
        }
    }
}

/// `bounce_keys`: a repeat of the same key inside the window is ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BounceKeysConfig {
    /// Whether bounce keys filters repeats.
    pub enabled: bool,
    /// The minimum interval between accepted presses of the same key.
    pub delay_ms: u64,
    /// Whether a rejected repeat beeps.
    pub beep_on_reject: bool,
}

impl Default for BounceKeysConfig {
    /// The GNOME schema defaults: off, with a 300 ms window.
    fn default() -> Self {
        Self {
            enabled: false,
            delay_ms: 300,
            beep_on_reject: false,
        }
    }
}

/// `mouse_keys`: the keypad moves and clicks the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseKeysConfig {
    /// Whether the keypad drives the pointer.
    pub enabled: bool,
    /// The pointer speed once acceleration is complete, in pixels per second.
    pub max_speed: u32,
    /// How long the speed takes to reach [`Self::max_speed`].
    pub acceleration_time_ms: u64,
    /// How long a movement key is held before the pointer starts moving.
    pub initial_delay_ms: u64,
}

impl Default for MouseKeysConfig {
    /// The GNOME schema defaults: off, 10 pixels per second, 300 ms of ramp and
    /// 300 ms before the first move.
    fn default() -> Self {
        Self {
            enabled: false,
            max_speed: 10,
            acceleration_time_ms: 300,
            initial_delay_ms: 300,
        }
    }
}

/// The magnifier's own settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MagnifierConfig {
    /// Whether the magnifier draws at all.
    pub enabled: bool,
    /// Whether the renderer paints the source rectangle over the whole output
    /// or in a lens at its own position.
    pub mode: Mode,
    /// The zoom factor.
    pub zoom: Zoom,
    /// What the view follows. Only this source moves it, so a magnifier set to
    /// follow the caret does not jump when the mouse moves.
    pub follow: Follow,
}

impl Default for MagnifierConfig {
    /// Off, full screen, doubled, following the pointer.
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::FullScreen,
            zoom: Zoom::X2,
            follow: Follow::Pointer,
        }
    }
}

/// How the magnifier paints its source rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// A lens drawn at the source rectangle's own position.
    Lens,
    /// The source rectangle drawn over the whole output.
    FullScreen,
}

/// What moves the magnifier's view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Follow {
    /// The focused window's anchor.
    Focus,
    /// The text caret's anchor.
    Caret,
    /// The pointer.
    Pointer,
}
