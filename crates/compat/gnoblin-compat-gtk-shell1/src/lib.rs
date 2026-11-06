//! `gtk_shell1` and `gtk_surface1`: the redirect GTK applications use every day.
//!
//! This is `gnoblin-compat-gtk-shell1` of `design/rust-compositor.md` (the redirect
//! map: "activation, titlebar actions, modal, launch, bell", with the note that this
//! "redirect matters most for daily use"). GTK draws its own titlebars, so without it
//! a titlebar double-click never reaches the compositor, a modal dialog is not known
//! to be modal, and an application's bell rings nowhere.
//!
//! # What an adapter owns here, and what it does not
//!
//! The crate owns the protocol — the vendored XML, the generated bindings, the
//! global, the dispatch and the mapping from a request to what the compositor should
//! do. It does not own the compositor: activation, focus, the window manager and the
//! bell are `gnoblin-wayland`'s through [`gnoblin_wayland::Shell1Host`], which is
//! this crate's only dependency on the engine and the reason the transport rule
//! ("the `gnoblin-wayland` extension trait for the Wayland redirects") can hold.
//!
//! # The three rules the mapping keeps
//!
//! - **A gesture becomes the configured action, not a built-in one.** `titlebar_gesture`
//!   names `double_click`, `right_click` or `middle_click`, and the action is the one
//!   `window_management.action_*_click_titlebar` declares, which is the same setting a
//!   Gnoblin-drawn titlebar reads. An unknown gesture is the protocol's own error
//!   (`invalid_gesture`), posted to that client.
//! - **Presentation is activation, and an app id is never activation.**
//!   `present` and `request_focus` go through the session's activation authority, and
//!   the only trusted context `gtk_shell1` can offer is "the user is already working in
//!   this window". A window that names an application id and asks to be raised is
//!   refused with `activation-denied: no-authority`.
//! - **The bell is a signal.** `system_bell` raises the session's `bell` count and
//!   publishes it; whether a bell is visual, audible or both is the configuration's
//!   (`compositor.visual_bell`, `compositor.audible_bell`) and `gnoblin-a11y`'s
//!   decision, not this adapter's.
//!
//! # Failure modes first
//!
//! The plan's list for a redirect, then this adapter's own edges. Each names its test
//! in `tests/`.
//!
//! 1. **`present` grants focus because the window named an app id.** The window is
//!    focused only when it already holds the keyboard focus, and an unfocused one's
//!    request is published as a refusal: `present_does_not_grant_focus_by_app_id`.
//! 2. **A request that arrives after its surface died acts on nothing.**
//!    [`gnoblin_wayland::Shell1Host::set_app_id`] and the rest answer
//!    [`gnoblin_wayland::Surface::Gone`], so a client that races its own window's
//!    destruction changes nothing: `a_dead_surfaces_request_changes_nothing`.
//! 3. **A bell loops.** The adapter publishes one count per request and never calls
//!    itself; a client that rings twice raises the count twice and nothing else
//!    happens: `a_bell_is_counted_once_per_request`.
//! 4. **An unknown gesture is applied as if it were a click.** The protocol's own
//!    `invalid_gesture` error is posted and no action runs:
//!    `an_unknown_gesture_posts_the_protocols_error`.
//! 5. **A configured action the engine cannot perform is pretended.**
//!    [`gnoblin_wayland::Titlebar::Unavailable`] names why nothing happened, so
//!    horizontal and vertical maximise and the window menu are visibly absent rather
//!    than silently a no-op: `an_action_the_engine_lacks_is_reported`.
//! 6. **A member the support matrix does not declare is served.** The frame refuses it
//!    before the mapping runs: `an_undeclared_member_is_refused`.
//! 7. **The redirect is off but still bound.** `Mode::Off` refuses every call, so a
//!    session that turned the adapter off answers nothing:
//!    `the_mode_gate_refuses_a_disabled_adapter`.

// `deny`, not `forbid`: `wayland-scanner` emits the protocol's C interface tables and
// there is no generation mode without them, so `src/protocol.rs` allows unsafe code in
// its generated module and nowhere else. `Cargo.toml` says the same thing.
#![deny(unsafe_code)]

/// Admission: the support matrix, the mode gate and the protocol's operation classes.
pub mod compat;
/// The global, its objects and the dispatch that turns a request into a host call.
pub mod global;
/// The protocol's values, mapped onto the compositor's.
pub mod mapping;
/// The generated protocol bindings, private to this crate.
mod protocol;

/// The admission check, the call it gates, the support matrix and the adapter's name.
pub use compat::{Admission, Call, MATRIX, NAME};
/// The redirect a session is given, and the `gtk_shell1` version this adapter serves.
pub use global::{Redirect, SHELL_VERSION};
/// The titlebar gestures, the requests they arrive as, and the resolution to a configured action.
pub use mapping::{Gesture, Requested, action_for, resolve};

/// The vendored protocol's own names, for a client that has to speak it.
pub mod names {
    /// The `gtk_shell1` and `gtk_surface1` protocol modules, re-exported for clients.
    pub use crate::protocol::{gtk_shell1, gtk_surface1};
}
