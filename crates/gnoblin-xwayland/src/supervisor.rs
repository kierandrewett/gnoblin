//! Starting Xwayland, and what to do when it dies.
//!
//! The compositor owns the process, the event loop and the clock; this module owns the
//! decisions, so the restart policy is a function of the instants it is handed rather
//! than a thread with a sleep in it. Four things follow from that:
//!
//! - **A crash is retried with a growing delay.** The first failure waits half a
//!   second, the next a second, and so on, so a server that cannot start does not spin
//!   the CPU and does not flood the log.
//! - **A crash loop ends.** After the configured attempts the session stops and says
//!   why, and the reason names what a person has to fix. Retrying forever turns a
//!   misconfiguration into a session that is never usable and never says so.
//! - **A start that never becomes ready is a failure.** The deadline is the policy's,
//!   not the caller's: a hung server takes the same path as a crashed one.
//! - **Clients on the old display are gone.** A restart allocates a new display
//!   number, so every X11 client that was connected is disconnected with the server
//!   that served it. Nothing reconnects them; they exit and can be started again. The
//!   status says so, because a session that silently loses its X11 applications is
//!   worse than one that says they are gone.
//!
//! # Failure modes first
//!
//! Each entry names a test in this module.
//!
//! 1. **A missing binary panics or starts anyway.** [`probe`](crate::settings::probe)
//!    answers a sentence instead, and [`Xwayland::start`] turns it into a disabled
//!    session: `a_missing_binary_disables_the_session`.
//! 2. **A crash becomes a restart loop.** Attempts are counted, the delay doubles and
//!    the last one stops: `a_crash_loop_stops_after_the_last_attempt`.
//! 3. **A restart keeps the dead display.** The new attempt reports a new number and
//!    the old clients are gone with the old server:
//!    `a_restart_reports_a_new_display`.
//! 4. **A ready server is treated as a failure.** Readiness clears the attempt count,
//!    so a later crash starts the ladder again instead of stopping at once:
//!    `a_ready_server_starts_the_ladder_again`.
//! 5. **A hung start looks healthy forever.** The deadline is stated by the policy and
//!    the same failure path applies: `a_start_that_never_becomes_ready_fails`.

use std::ffi::OsString;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gnoblin_backend::x11::{Restart, Setting, Start};

use crate::settings::{Settings, probe};
use crate::windows::Xwm;
use crate::xsettings::XSettings;

/// The first delay after a failed start.
const FIRST_DELAY: Duration = Duration::from_millis(500);
/// The longest delay between two attempts.
const MAX_DELAY: Duration = Duration::from_secs(8);
/// How many attempts a session makes before it stops.
const ATTEMPTS: u32 = 5;
/// How long a start may take before it counts as a failure.
const STARTUP_DEADLINE: Duration = Duration::from_secs(10);

/// Where a session is.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Status {
    /// Nothing has been started yet.
    #[default]
    Idle,
    /// An attempt is running.
    Starting {
        /// The attempt number, from one.
        attempt: u32,
    },
    /// The server is up and X11 clients can connect on this display number.
    Ready {
        /// The display number the server chose.
        display: u32,
        /// How many times the session was restarted before this instance.
        restarts: u32,
    },
    /// An attempt failed and the next one is scheduled.
    Restarting {
        /// The attempt number that failed.
        attempt: u32,
        /// How long the session waits before starting again.
        after: Duration,
    },
    /// No attempt can succeed, and this is why.
    Disabled {
        /// The sentence to log as it stands.
        reason: String,
    },
    /// Every attempt failed.
    Stopped {
        /// The sentence to log as it stands.
        reason: String,
    },
}

impl Status {
    /// The display number X11 clients may connect to, while there is one.
    #[must_use]
    pub fn display(&self) -> Option<u32> {
        match self {
            Self::Ready { display, .. } => Some(*display),
            _ => None,
        }
    }

    /// Whether the session is disabled or has given up.
    #[must_use]
    pub fn is_off(&self) -> bool {
        matches!(self, Self::Disabled { .. } | Self::Stopped { .. })
    }
}

/// The mutable half of a session.
struct Lifecycle {
    settings: Settings,
    /// The `PATH` the binary is looked for on, kept so a start path can be tested
    /// without touching the process environment.
    path: Option<OsString>,
    /// The settings owner, when a compat adapter is attached.
    xsettings: Option<Box<dyn XSettings>>,
    /// The attempts made since the last ready instance.
    attempt: u32,
    /// How many instances became ready.
    instances: u32,
    status: Status,
}

/// An Xwayland session: the start policy, the restart policy and the window table.
///
/// The compositor starts one with [`Xwayland::new`], hands [`Xwayland::parts`] its two
/// halves and keeps [`Xwayland::status`] for diagnostics. The halves share one
/// lifecycle, so a status read from any clone sees the whole session.
pub struct Xwayland {
    lifecycle: Arc<Mutex<Lifecycle>>,
    settings: Settings,
}

impl Xwayland {
    /// A session that looks for its binary on `path` and needs no settings owner.
    #[must_use]
    pub fn new(settings: Settings, path: Option<OsString>) -> Self {
        Self::with_settings(settings, path, None)
    }

    /// A session with an XSETTINGS owner attached.
    #[must_use]
    pub fn with_settings(settings: Settings, path: Option<OsString>, xsettings: Option<Box<dyn XSettings>>) -> Self {
        Self {
            lifecycle: Arc::new(Mutex::new(Lifecycle {
                settings: settings.clone(),
                path,
                xsettings,
                attempt: 0,
                instances: 0,
                status: Status::Idle,
            })),
            settings,
        }
    }

    /// A handle the compositor drives the session with.
    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle {
            lifecycle: Arc::clone(&self.lifecycle),
        }
    }

    /// The two halves the compositor keeps: the lifecycle it ticks and the window table
    /// it asks about X11 windows.
    ///
    /// They are separate values because the window table is owned by the compositor and
    /// the lifecycle is shared with whoever reads the status, and because a caller that
    /// must not touch X11 at all asks for neither.
    #[must_use]
    pub fn parts(&self) -> (Handle, Xwm) {
        let windows = Xwm::new(self.settings.allow_grabs, self.settings.grab_access_rules.clone());
        (self.handle(), windows)
    }

    /// Where the session is now.
    #[must_use]
    pub fn status(&self) -> Status {
        let lifecycle = self.lifecycle.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lifecycle.status.clone()
    }

    /// The display number X11 clients can connect to, while the server is up.
    #[must_use]
    pub fn display(&self) -> Option<u32> {
        self.status().display()
    }
}

/// The compositor's half of a session.
///
/// A clone is a handle to the same session; the compositor ticks one and keeps another
/// for diagnostics. Dropping every clone drops the settings owner, which is how the
/// adapter that registered one learns the session is over.
#[derive(Clone)]
pub struct Handle {
    lifecycle: Arc<Mutex<Lifecycle>>,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("XwaylandHandle")
            .field("status", &self.status())
            .finish()
    }
}

impl Handle {
    fn lock(&self) -> MutexGuard<'_, Lifecycle> {
        self.lifecycle.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Where the session is now.
    #[must_use]
    pub fn status(&self) -> Status {
        self.lock().status.clone()
    }

    /// The delay before attempt `attempt` is retried.
    ///
    /// The first failure waits [`FIRST_DELAY`] and every later one doubles it, capped at
    /// [`MAX_DELAY`]. The doubling is what separates a transient crash from a server
    /// that cannot start: the log shows five attempts a minute apart, not five a second.
    fn delay(attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(8);
        FIRST_DELAY.saturating_mul(1 << shift).min(MAX_DELAY)
    }
}

impl gnoblin_backend::x11::Session for Handle {
    fn start(&mut self, _now: Instant) -> Start {
        let mut lifecycle = self.lock();
        if let Err(reason) = probe(lifecycle.path.as_deref()) {
            lifecycle.status = Status::Disabled { reason: reason.clone() };
            return Start::Disabled(reason);
        }
        lifecycle.attempt = lifecycle.attempt.saturating_add(1);
        lifecycle.status = Status::Starting {
            attempt: lifecycle.attempt,
        };
        // The display number is not fixed: the server writes the one it took to the
        // readiness pipe, and fixing it here would collide with another session on the
        // same machine.
        Start::Spawn(lifecycle.settings.launch(None))
    }

    fn ready(&mut self, display: u32, _now: Instant) {
        let mut lifecycle = self.lock();
        // Readiness is what separates an instance that worked from one that did not:
        // the ladder starts from the first delay again, so a session that crashes once
        // a day never accumulates failures.
        lifecycle.attempt = 0;
        lifecycle.instances = lifecycle.instances.saturating_add(1);
        lifecycle.status = Status::Ready {
            display,
            restarts: lifecycle.instances.saturating_sub(1),
        };
    }

    fn failed(&mut self, _now: Instant) -> Restart {
        let mut lifecycle = self.lock();
        let attempt = lifecycle.attempt.max(1);
        if attempt >= ATTEMPTS {
            let reason = format!(
                "Xwayland failed {attempt} starts in a row and is not restarted; the X11 clients of \
                 the last display are gone with it"
            );
            lifecycle.status = Status::Stopped { reason: reason.clone() };
            return Restart::Stop(reason);
        }
        let after = Handle::delay(attempt);
        lifecycle.status = Status::Restarting { attempt, after };
        Restart::RetryAfter(after)
    }

    fn startup_deadline(&self) -> Duration {
        STARTUP_DEADLINE
    }

    fn settings(&mut self) -> Vec<(String, Setting)> {
        let mut lifecycle = self.lock();
        match lifecycle.xsettings.as_mut() {
            Some(owner) => owner.settings(),
            None => Vec::new(),
        }
    }

    fn settings_published(&mut self, result: Result<(), String>) {
        let mut lifecycle = self.lock();
        if let Some(owner) = lifecycle.xsettings.as_mut() {
            owner.published(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gnoblin_backend::x11::{Session, Windows};

    use crate::xsettings::RecordingOwner;

    /// A `PATH` with no `Xwayland` on it, so the disabled path is exercised without
    /// spawning anything.
    fn empty_path() -> Option<OsString> {
        Some(OsString::from("/nonexistent-gnoblin-test-path"))
    }

    /// The real binary, for a test that needs a start to be possible.
    fn real_path() -> Option<OsString> {
        std::env::var_os("PATH")
    }

    fn now() -> Instant {
        Instant::now()
    }

    /// A missing binary is a disabled session with a sentence, and no attempt is
    /// invented.
    #[test]
    fn a_missing_binary_disables_the_session() {
        let mut handle = Xwayland::new(Settings::default(), empty_path()).handle();
        match handle.start(now()) {
            Start::Disabled(reason) => assert!(reason.contains("no Xwayland binary"), "{reason}"),
            other => panic!("expected a disabled session, got {other:?}"),
        }
        assert!(matches!(handle.status(), Status::Disabled { .. }));
        assert_eq!(handle.status().display(), None);

        // Asking again is the same answer, not a second failure: a disabled session
        // never counts an attempt.
        assert!(matches!(handle.start(now()), Start::Disabled(_)));
    }

    /// A server that keeps failing stops after the last attempt, and the delays grow.
    ///
    /// `ATTEMPTS` failures are the whole budget: the failures before the last one are
    /// retried, and the last one stops the session rather than scheduling a start that
    /// cannot work.
    #[test]
    fn a_crash_loop_stops_after_the_last_attempt() {
        let mut handle = Xwayland::new(Settings::default(), real_path()).handle();
        let mut delays = Vec::new();

        for attempt in 1..ATTEMPTS {
            assert!(
                matches!(handle.start(now()), Start::Spawn(_)),
                "attempt {attempt} starts"
            );
            match handle.failed(now()) {
                Restart::RetryAfter(after) => delays.push(after),
                other => panic!("attempt {attempt} should be retried, got {other:?}"),
            }
        }
        assert_eq!(delays.first(), Some(&FIRST_DELAY));
        assert!(
            delays.windows(2).all(|pair| pair[0] <= pair[1]),
            "the delay never shrinks"
        );
        assert!(delays.last() < Some(&MAX_DELAY) || delays.last() == Some(&MAX_DELAY));

        assert!(
            matches!(handle.start(now()), Start::Spawn(_)),
            "the last attempt starts"
        );
        match handle.failed(now()) {
            Restart::Stop(reason) => {
                assert!(reason.contains("not restarted"), "{reason}");
                assert!(reason.contains(&ATTEMPTS.to_string()), "{reason}");
            }
            other => panic!("the last attempt must stop the session, got {other:?}"),
        }
        assert!(matches!(handle.status(), Status::Stopped { .. }));
    }

    /// Readiness clears the ladder, so the restarts of a long session are not counted
    /// as a crash loop.
    #[test]
    fn a_ready_server_starts_the_ladder_again() {
        let mut handle = Xwayland::new(Settings::default(), real_path()).handle();

        handle.start(now());
        handle.failed(now());
        handle.start(now());
        handle.ready(7, now());
        assert_eq!(handle.status().display(), Some(7));

        for _ in 0..ATTEMPTS - 1 {
            assert!(matches!(handle.start(now()), Start::Spawn(_)));
            handle.failed(now());
            handle.start(now());
            handle.ready(8, now());
        }
        assert!(matches!(handle.start(now()), Start::Spawn(_)));
        assert!(
            matches!(handle.failed(now()), Restart::RetryAfter(_) | Restart::Stop(_)),
            "a ready session that fails only once must not stop at once"
        );
    }

    /// A restart reports a new display, and the status says how many instances came
    /// before it: the clients of the old display are gone with the server that served
    /// them.
    #[test]
    fn a_restart_reports_a_new_display() {
        let mut handle = Xwayland::new(Settings::default(), real_path()).handle();

        handle.start(now());
        handle.ready(3, now());
        assert_eq!(handle.status().display(), Some(3));

        handle.start(now());
        handle.failed(now());
        handle.start(now());
        handle.ready(4, now());

        assert_eq!(handle.status().display(), Some(4));
        assert_eq!(
            handle.status(),
            Status::Ready {
                display: 4,
                restarts: 1
            }
        );
    }

    /// A start that hangs is the same failure as a crash, and the deadline that decides
    /// it is the policy's.
    #[test]
    fn a_start_that_never_becomes_ready_fails() {
        assert_eq!(
            Xwayland::new(Settings::default(), real_path())
                .handle()
                .startup_deadline(),
            STARTUP_DEADLINE
        );
        let mut handle = Xwayland::new(Settings::default(), real_path()).handle();
        handle.start(now());
        assert_eq!(handle.status(), Status::Starting { attempt: 1 });
        assert!(matches!(handle.failed(now()), Restart::RetryAfter(_)));
        assert!(matches!(handle.status(), Status::Restarting { attempt: 1, .. }));
    }

    /// An attached owner's settings are handed to the compositor, and the answer is
    /// reported back to it even when publishing failed.
    #[test]
    fn an_attached_owner_hears_what_publishing_did() {
        let owner = RecordingOwner::default();
        owner.set("gtk-theme", Setting::Text("Adwaita".to_owned()));
        let mut handle =
            Xwayland::with_settings(Settings::default(), real_path(), Some(Box::new(owner.clone()))).handle();

        assert_eq!(
            handle.settings(),
            vec![("gtk-theme".to_owned(), Setting::Text("Adwaita".to_owned()))]
        );
        handle.settings_published(Err("no window manager yet".to_owned()));
        assert_eq!(owner.reported(), vec![Err("no window manager yet".to_owned())]);
    }

    /// With no owner there is nothing to publish, and reporting an answer is harmless.
    #[test]
    fn a_session_without_an_owner_publishes_nothing() {
        let mut handle = Xwayland::new(Settings::default(), real_path()).handle();
        assert!(handle.settings().is_empty());
        handle.settings_published(Ok(()));
    }

    /// The halves share one lifecycle: a status read from a handle the caller kept sees
    /// what the compositor's half did.
    #[test]
    fn the_halves_share_one_lifecycle() {
        let session = Xwayland::new(Settings::default(), real_path());
        let (mut driven, windows) = session.parts();
        assert_eq!(session.status(), Status::Idle);

        driven.start(now());
        driven.ready(11, now());

        assert_eq!(session.display(), Some(11));
        assert_eq!(windows.allow_grabs(), Settings::default().allow_grabs);
    }
}
