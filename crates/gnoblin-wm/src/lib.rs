//! Pure window management policy: stacking, focus, workspaces, placement, snapping and rules.
//!
//! This crate is the "Policy core" `gnoblin-wm` of `design/rust-compositor.md`. It
//! owns floating placement, stacking, the focus model and its history, workspaces
//! with stable IDs, maximise, fullscreen, minimise, snapping, interactive move and
//! resize, window rules and work areas from reserved insets.
//!
//! Everything here is a state machine over plain data. There are no Smithay types,
//! no renderer handles, no Wayland objects, no file or socket access and no clock,
//! so the same event sequence always produces the same effects. The composition
//! root translates protocol events into [`Event`]s, feeds them to [`Wm::apply`] and
//! turns the returned [`Effect`]s into client configures, scene-graph stacking and
//! focus changes.
//!
//! Two decisions are deliberately not made here:
//!
//! - **Activation.** Whether an activation request may take focus is decided by
//!   attribution and the `focus_new_windows` policy, which live in
//!   `gnoblin-authority` and the configuration core. The caller passes the answer
//!   as [`ActivationDecision`]; the window manager only refuses to focus a window
//!   that cannot be seen.
//! - **Interactive geometry.** The input layer computes the dragged rectangle from
//!   pointer deltas; [`Event::MoveResize`] carries it and this crate applies the
//!   work-area constraint, so the same constraint code serves drags, rules and
//!   output changes.
//!
//! # Failure modes
//!
//! The list below is the plan's "Failure modes first" for this component plus the
//! ones named for this increment. Each entry names where it is covered.
//!
//! 1. **Focus lands on a closed window.** [`Event::WindowClosed`] removes the
//!    window from the focus set, the history and the stack before anything reads
//!    them, and [`Event::ActivationRequested`] for an unknown window is rejected.
//!    Covered by `tests/failure_modes.rs::focus_never_lands_on_a_closed_window`
//!    and by the history invariant in `tests/model.rs`.
//! 2. **Minimised windows keep focus.** Minimising the focused window refocuses to
//!    the most recently used window that is still visible. Covered by
//!    `failure_modes.rs::minimising_the_focused_window_moves_focus_away`.
//! 3. **A window is placed off every output after hotplug.** Disconnecting an
//!    output retargets its windows to a surviving output and recomputes their
//!    geometry for their state; with no output left they are unassigned and
//!    hidden rather than left pointing at an output that is gone. Covered by
//!    `failure_modes.rs::unplugging_an_output_keeps_windows_on_a_valid_output`,
//!    `unplugging_the_last_output_hides_every_window` and the per-window output
//!    invariant in `tests/model.rs`.
//! 4. **Stacking order inconsistent with focus.** The stack holds every window
//!    exactly once and orders layers bottom-to-top as normal then fullscreen;
//!    raising, layering and closing all go through the stacking type, and focus
//!    changes that raise go through it too. A focused window is always a visible
//!    window, and a stale stack entry cannot exist because removal happens with
//!    the window. Covered by
//!    `failure_modes.rs::raising_keeps_the_fullscreen_layer_on_top`, the stack unit
//!    tests and the stack and layer invariants in `tests/model.rs`.
//! 5. **A rule is applied twice.** Rules run once, when a window is first taken
//!    into management. Title changes, application-ID changes and a rules reload
//!    never re-run them, so placement and maximise cannot be applied twice.
//!    Covered by `failure_modes.rs::rules_apply_once_even_when_titles_and_rules_change`.
//! 6. **A workspace switch loses a fullscreen window.** Fullscreen is a flag over
//!    the other states, not a replacement for them: switching away hides the
//!    window without touching its state, and switching back shows the same
//!    full-screen window on top. Covered by
//!    `failure_modes.rs::switching_workspace_keeps_the_fullscreen_window_intact`.
//! 7. **A rule loops.** A rule cannot loop: each rule is evaluated at most once per
//!    window, matching is a single linear pass over the pattern, and no rule
//!    re-enters evaluation. A rule list that moves a window back and forth
//!    therefore stops at the last matching rule. Covered by
//!    `failure_modes.rs::contradicting_rules_settle_on_the_last_match` and the
//!    pattern tests in `rules.rs`.
//! 8. **Placement never terminates.** Free-space scanning and the cascade fallback
//!    are bounded by `placement::MAX_SCAN_STEPS` and
//!    `placement::MAX_CASCADE_STEPS`, so a work area full of windows still yields
//!    one rectangle. Covered by `placement.rs::a_full_work_area_still_places_the_window`.
//! 9. **A snapped or maximised window drifts from its zone.** Geometry for the
//!    non-floating states is derived from the work area every time the work area
//!    changes, and [`Wm::check_invariants`] requires maximised, fullscreen and
//!    snapped windows to match their computed rectangle exactly. Covered by
//!    `failure_modes.rs::reserved_space_reapplies_to_maximised_windows`.
//! 10. **An effect is emitted for an event that changed nothing.** Effects are
//!     produced from a comparison with the state before the event, so a no-op
//!     event (closing an unknown window, re-activating the current workspace,
//!     re-minimising a minimised window) yields only a [`Effect::Rejected`] or
//!     nothing at all. Covered by the scenario tests in `tests/failure_modes.rs`.
//!
//! # Determinism
//!
//! Windows are keyed by [`gnoblin_types::WindowId`] in an ordered map, outputs keep connection
//! order and workspaces keep declared order, so iteration and effect order are
//! stable. Nothing in this crate reads a clock, a random source or the
//! environment. `tests/model.rs` replays each random sequence twice and requires
//! identical effect streams.

pub mod layout;

mod activation;
mod arrange;
mod drag;
mod effect;
mod event;
mod geometry;
mod output;
mod placement;
mod rules;
mod snapping;
mod stack;
mod window;
mod wm;
mod workspace;

pub use activation::ActivationDecision;
pub use drag::DragConstraint;
pub use effect::{Effect, RuleRejection};
pub use event::{Event, Rejection, WindowSpec};
pub use output::Output;
pub use placement::PlacementMode;
pub use rules::{Match, Pattern, Rule, RuleAction, WorkspaceSelector};
pub use snapping::SnapZone;
pub use window::{Window, WindowKind, WindowState};
pub use wm::{Broken, CommandError, Wm, WmOptions};
