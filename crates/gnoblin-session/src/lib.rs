//! Guardian: supervision, systemd and logind, environment, autostart, logs and recovery.
//!
//! This crate is the guardian of the process model in `design/rust-compositor.md`
//! and the supervision policy of `design/lean-runtime.md`: one process lives for
//! the login and owns the session, while the compositor, the runtime, the
//! services role, the front and the autostart applications are children it
//! starts, watches and stops.
//!
//! The state machine is pure logic: [`Guardian::apply`] spawns no process, signals no
//! socket, reads no clock and writes no file, and every [`Event`] carries its own time, so
//! one event sequence always produces one effect sequence. [`supervisor`] is the other half
//! — the module that performs the [`Effect`]s, reads the children's reports and turns the
//! session manager's signals into events — and it is the only place in the crate that
//! touches the operating system. `gnoblin` calls it for the guardian role.
//!
//! # What it guarantees
//!
//! - **The compositor is never restarted.** Its [`Restart::EndsSession`] policy
//!   means its exit ends the session, and the guardian then stops every other
//!   child in reverse start order, so nothing is left talking to a dead server.
//! - **At most one instance per child.** A child is spawned only from `Pending`
//!   or `Backoff`, and every spawn increments its generation, so an `Exited` or
//!   `Ready` from an earlier instance is recognised and ignored instead of
//!   restarting a healthy child or counting one crash twice.
//! - **Restarts are bounded and caller-driven.** The runtime and the services
//!   role restart on [`Backoff`] with the C supervisor's ladder, and a child that
//!   exhausts its budget fails once, is reported degraded, and is never spawned
//!   again: no reachable state spins.
//! - **A restartable child never ends the session.** Only the compositor and an
//!   unusable start do. A services or runtime failure degrades the session and
//!   leaves every working child alone, which is the property that lets a user
//!   keep their windows while an agent restarts.
//! - **Nothing starts before it can work.** Children start in plan order, one at
//!   a time, each after the previous one reports ready; the compositor's display
//!   environment is exported before the next child is spawned.
//! - **Nothing runs forever.** A child that misses its readiness deadline, and a
//!   child that ignores the shutdown grace period, are killed, and a kill the
//!   caller never reports does not hold the session open.
//! - **The session ends in order.** Stop goes by rank in reverse start order
//!   (autostart applications, front, runtime, services, compositor), so a
//!   client is always stopped before the server it talks to. Everything in one
//!   rank is signalled together, so a desktop with ten autostart applications
//!   does not pay ten grace periods.
//! - **The environment is data.** [`EnvironmentRules`] describes what the
//!   session sets, clears and inherits, and the compositor's published display
//!   variables are validated against it before any later child sees them.
//!
//! # Failure modes first
//!
//! The plan's card, `design/startup-recovery.md`, `design/lean-runtime.md` and
//! `src/session` give this list. Each entry names the test that covers it.
//!
//! 1. A runtime that crashes immediately is restarted forever and burns a core:
//!    the budget stops it and the session is reported degraded —
//!    `a_crash_looping_runtime_stops_and_degrades_the_session`.
//! 2. A services crash takes the whole session down with it: the session stays up
//!    and the child comes back —
//!    `a_services_crash_does_not_end_the_session`.
//! 3. A compositor crash leaves orphans running: every other child is stopped,
//!    dependants first —
//!    `a_compositor_crash_stops_every_other_child`.
//! 4. Stop order leaves a client talking to a dead server: autostart
//!    applications and the front stop before the runtime, services and
//!    compositor they talk to —
//!    `dependants_stop_before_the_compositor`.
//! 5. A child never becomes ready: it is killed at its deadline and treated as
//!    the failure it announced —
//!    `an_unready_child_is_killed_at_its_deadline`.
//! 6. Backoff overflow: a doubling ladder or a long uptime wraps into an
//!    immediate restart or a negative wait —
//!    `backoff_saturates_instead_of_overflowing`.
//! 7. A late exit from an earlier instance restarts a healthy child —
//!    `a_stale_exit_does_not_restart_a_healthy_child`.
//! 8. A child ignores the graceful signal and the session never ends —
//!    `a_child_that_ignores_the_graceful_signal_is_killed`.
//! 9. A child the caller never reaps holds the session open —
//!    `an_unreaped_kill_does_not_hold_the_session`.
//! 10. A failed autostart application ends the session —
//!     `a_failed_autostart_entry_does_not_end_the_session`.
//! 11. A front that dies before the session is up blocks the start sequence —
//!     `a_front_that_dies_before_the_session_is_up_does_not_block_it`.
//! 12. The compositor never becomes ready, or reports ready without the display
//!     the session needs, and children still launch —
//!     `an_unready_compositor_ends_the_session` and
//!     `a_compositor_without_a_display_environment_ends_the_session`.
//! 13. Every session start fails and the guardian keeps trying the same
//!     configuration: safe mode is requested after the configured number —
//!     `safe_mode_is_requested_after_the_configured_failed_starts`.
//! 14. A rejected configuration loses the last-good revision —
//!     `a_rejected_configuration_keeps_the_last_good_revision`.
//! 15. A later child starts without the display the compositor published —
//!     `the_compositor_display_environment_reaches_later_children`.
//! 16. A child restarts through a shutdown, or an event arrives after the
//!     session ended and starts something —
//!     `events_after_the_session_ended_change_nothing`.
//! 17. A child that keeps crashing never gets its budget back, so one bad minute
//!     disables it for the rest of the login —
//!     `a_stable_child_gets_its_restart_budget_back`.
//!
//! `src/model.rs` adds model-based tests over random event sequences for the
//! invariants no single scenario can establish: the compositor is never
//! restarted, at most one instance of a child is ever live, and every spawn is
//! matched by an exit or a kill by the time a session has fully stopped.

#![forbid(unsafe_code)]

mod effect;
mod environment;
mod event;
mod guardian;
mod ids;
#[cfg(test)]
mod model;
mod plan;
mod recovery;
mod state;
pub mod supervisor;
#[cfg(test)]
mod tests;
mod time;

pub use effect::{Effect, Level};
pub use environment::{EnvTarget, Environment, EnvironmentRules, InheritedEnvironment};
pub use event::{Event, EventKind, ExitStatus, ShutdownReason, Signal};
pub use guardian::{Guardian, Rules};
pub use ids::ChildId;
pub use plan::{Backoff, ChildKind, ChildSpec, DEFAULT_READY_WITHIN, Plan, PlanError, Restart};
pub use recovery::{ConfigRecovery, ConfigStatus, Recovery};
pub use state::{ChildState, ChildStatus, SessionState, StopReason};
pub use supervisor::{SupervisorConfig, SupervisorError, TEST_FAST_VARIABLE, run as run_guardian};
pub use time::Tick;
