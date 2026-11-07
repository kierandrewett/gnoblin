//! Accessibility: the keyboard-monitor grab table, sticky, slow, bounce and
//! mouse keys, the magnifier, and the motion and contrast flags apps read.
//!
//! This crate is policy with no compositor in it. `gnoblin-input` turns seat and
//! D-Bus traffic into [`Event`]s, [`A11y::apply`] answers with [`Effect`]s in
//! the order they must happen, `gnoblin-compositor` performs them, and
//! `gnoblin-render` draws the magnifier's view and honours the published
//! appearance flags. Nothing here depends on Smithay, on a renderer or on a
//! clock: every timed decision is driven by [`Event::Tick`] carrying the
//! compositor's monotonic milliseconds, and the machine asks for the next one
//! with [`Effect::AwaitTick`]. A test can therefore replay an hour of holds and
//! releases in a loop and get the same effects every time.
//!
//! # Order of a key event
//!
//! Mouse keys, bounce keys, slow keys, sticky keys, keyboard monitor, in that
//! order and for the reasons [`A11y`] documents. A key that any of the first
//! three swallows produces no [`Effect::Key`] at all, so "the client must not
//! see this" is one observable fact instead of a flag the wiring can forget.
//!
//! # Failure modes first
//!
//! The plan's `gnoblin-a11y` card, `docs/config/configure/input/keyboard.md`, the
//! `SetKeyGrabs` contract in `org.freedesktop.a11y.xml` and the reference in
//! `src/backends/native/meta-keyboard-a11y.c` give this list. Each entry names
//! the test that covers it.
//!
//! 1. A grab outlives the screen reader: `a_grab_ends_when_its_owner_drops`.
//! 2. A dropped consumer still receives keys: `a_dropped_owner_receives_nothing`.
//! 3. A grabbed key also reaches the focused client:
//!    `a_grabbed_key_never_reaches_the_focused_client`.
//! 4. A watcher without a grab suppresses typing:
//!    `watching_does_not_suppress_the_key`.
//! 5. Ungrabbing leaves the grab in place:
//!    `ungrab_keyboard_restores_normal_handling`.
//! 6. A grabbed keystroke matches with the wrong modifiers:
//!    `a_grabbed_keystroke_needs_its_modifiers`.
//! 7. A chord pressed while a grabbed modifier is held is not grabbed:
//!    `a_held_grabbed_modifier_grabs_every_key`.
//! 8. Sticky keys leave a modifier latched across a session lock:
//!    `a_session_lock_clears_sticky_modifiers`.
//! 9. Sticky keys leave a modifier latched when a consumer drops:
//!    `a_consumer_drop_clears_sticky_modifiers`.
//! 10. A latched modifier applies to more than one key:
//!     `a_latched_modifier_applies_to_one_key_only`.
//! 11. A second press latches instead of locking:
//!     `a_second_press_locks_the_modifier`.
//! 12. A third press does not clear the lock: `a_third_press_clears_the_lock`.
//! 13. A locked modifier is spent by the next key:
//!     `a_locked_modifier_applies_to_every_key`.
//! 14. Two modifiers pressed together latch instead of turning sticky keys off:
//!     `two_modifiers_together_turn_sticky_keys_off`.
//! 15. A withheld modifier press reaches the client:
//!     `a_withheld_modifier_press_never_reaches_the_client`.
//! 16. Slow keys accept a press before the delay:
//!     `slow_keys_hold_the_press_until_the_delay`.
//! 17. Slow keys accept a tap that ended before the delay:
//!     `a_tap_that_ends_before_the_delay_is_rejected`.
//! 18. Disabling slow keys delivers the press it was holding:
//!     `disabling_slow_keys_drops_pending_presses`.
//! 19. Bounce keys swallow a different key: `bounce_keys_admit_a_different_key`.
//! 20. Bounce keys accept a repeat inside the window:
//!     `bounce_keys_reject_a_repeat_inside_the_window`.
//! 21. A bounced press still produces a release:
//!     `a_bounced_press_produces_no_release`.
//! 22. Mouse keys move the pointer before the initial delay:
//!     `mouse_keys_wait_for_the_initial_delay`.
//! 23. Mouse keys jump straight to full speed:
//!     `mouse_keys_accelerate_to_the_maximum_speed`.
//! 24. Mouse keys keep moving after the key is released:
//!     `releasing_the_movement_key_stops_the_pointer`.
//! 25. Mouse keys leave a button pressed when they are disabled:
//!     `disabling_mouse_keys_releases_pressed_buttons`.
//! 26. Mouse keys intercept the keypad while Num Lock is on:
//!     `mouse_keys_step_aside_for_num_lock`.
//! 27. Mouse keys hand their keypad key to the client:
//!     `mouse_keys_swallow_the_keypad`.
//! 28. The magnifier shows lock-screen content on another output:
//!     `a_locked_magnifier_stays_on_its_output`.
//! 29. The magnifier's view escapes its output at an edge:
//!     `the_view_is_clamped_inside_the_output`.
//! 30. The magnifier keeps a view of an output that is gone:
//!     `a_removed_output_moves_the_view`.
//! 31. A follow source that is not configured moves the view:
//!     `only_the_configured_follow_source_moves_the_view`.
//! 32. Reduced motion and high contrast are not published to apps:
//!     `appearance_is_published_as_it_changes`.
//! 33. A native animation ignores reduced motion. That one fails in
//!     `gnoblin-render`, which owns every animation; this crate's part is the
//!     published flag, which entry 32 covers. No test here can establish it.
//!
//! `src/model.rs` adds model-based tests over random event sequences for the
//! invariants no single scenario can establish: no latched modifier survives a
//! lock event, every grab belongs to a live owner, and the magnifier's view
//! always lies inside its output.
//!
//! # Shared primitives
//!
//! [`geometry`] and [`ids`] re-export `gnoblin-types`: a [`geometry::Rect`]
//! here is the same type as a `Rect` in window management or in the renderer,
//! and an [`ids::OutputId`] is the same identity the output layout allocates, so
//! a magnifier view needs no conversion to be drawn. A [`Modifiers`] here is the
//! mask the input router compared a chord against, so sticky keys adds a bit the
//! router already understands. What this crate defines itself is only the key
//! event, the keystroke and the machines' own states.
//!
//! # What this crate does not own
//!
//! The on-screen keyboard, the shell's own accessibility and the apps' own
//! accessibility trees belong to the shell and the apps. Key grabs stay in the
//! compositor because a grab decides before the focused client sees the key, so
//! a process hop would add latency to every press; that is why this crate is
//! linked into `gnoblin-compositor` rather than into a D-Bus service.

#![forbid(unsafe_code)]

mod a11y;
mod config;
mod effect;
mod event;
/// Magnifier view geometry: points, sizes, rectangles and scale, shared with the renderer.
pub mod geometry;
/// Identifiers the accessibility machines key their tables by (outputs and owners).
pub mod ids;
mod keyboard;
mod magnifier;
mod monitor;
mod mouse_keys;

#[cfg(test)]
mod model;
#[cfg(test)]
mod tests;

/// The accessibility state machines: feed them [`Event`]s, apply their [`Effect`]s.
pub use a11y::A11y;
/// Settings under `accessibility`, one struct per feature, read from configuration.
pub use config::{
    BounceKeysConfig, Config, Follow, MagnifierConfig, Mode, MouseKeysConfig, SlowKeysConfig, StickyKeysConfig,
};
/// What the machines emit: the decisions, views and published flags the compositor acts on.
pub use effect::{Appearance, BeepReason, Effect, KeyDecision, KeyRouting, MagnifierArea, PointerButton, Published};
/// Inputs to the machines (key, pointer and output events) and the keystroke type they use.
pub use event::{Event, Key, Keystroke, Output};
/// Shared input enums (button and key state, modifiers), re-exported from `gnoblin-types`.
pub use gnoblin_types::{ButtonState, KeyState, Modifiers};
/// The latched-modifier state that sticky keys reports to the compositor.
pub use keyboard::Sticky;
