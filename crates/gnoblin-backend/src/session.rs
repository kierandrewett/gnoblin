//! The seat session: the thing that owns the devices, and what happens when it is taken
//! away.
//!
//! A hardware session does not open `/dev/dri/card0` and `/dev/input/event*` itself and it
//! does not survive a VT switch by trying harder: the *seat* owns those devices, and
//! `libseat` is the library that lends them to us. This module is the one place that talks
//! to it, and it has two halves:
//!
//! - [`Seat`] is the glue: it opens the session, opens and closes devices through it, and
//!   turns the library's event source into a queue the backend drains. Nothing here
//!   decides anything.
//! - [`SessionState`] is the decision: which transitions a pause and a resume event mean
//!   once repeats and duplicates are taken into account. It reads no clock, owns no
//!   descriptor and touches no device, so it is unit-tested on a machine with no seat at
//!   all.
//!
//! # Why pause is a state machine and not a flag
//!
//! A VT switch sends two events per switch and libseat is allowed to coalesce or repeat
//! them: a compositor that treated each event as an edge would release DRM master twice,
//! or re-enable every output while the session is already the visible one — a modeset on
//! a screen that is already correct, which is a visible flash. The rules the plan states
//! are therefore state transitions, and the ones that matter are:
//!
//! 1. **A pause stops frames.** [`SessionState::submits`] is false from the pause until
//!    the resume, and the backend presents nothing in between (a page flip against a
//!    master we no longer hold fails, and committing to a device another VT owns is
//!    exactly what libseat exists to prevent).
//! 2. **A pause gives DRM master back and a resume re-enables every output.** That is the
//!    backend's job, driven by [`Transition::Pause`] and [`Transition::Resume`]; this
//!    module only says which one arrived.
//! 3. **A resume damages everything.** Nothing the previous frames drew can be trusted
//!    after another VT has drawn on the display, so the resume carries a new generation
//!    number and the backend redraws every output in full.
//!
//! # Failure modes first
//!
//! 1. **A repeated pause releases the device twice.** Pausing an already paused session is
//!    [`Transition::None`], so the second event cannot reach the device:
//!    `a_repeated_pause_does_nothing`.
//! 2. **The first activation re-enables outputs that are already enabled.** An activation
//!    for a session that was never paused is [`Transition::None`]:
//!    `the_first_activation_changes_nothing`.
//! 3. **A resume is reported without a new generation**, so the backend repaints only the
//!    damage it had and leaves stale pixels beside it:
//!    `a_resume_bumps_the_generation_and_damages_everything`.
//! 4. **A frame is submitted while the seat is gone.** [`SessionState::submits`] is false
//!    for the whole pause: `no_frame_is_submitted_from_a_pause_until_its_resume`.
//! 5. **A pause and resume pair that arrive twice deliver two generations**, which would
//!    make the second redraw pointless: `a_pause_and_resume_pair_advances_one_generation`.
//!
//! The glue adds the failures a unit test cannot reach, which is why they are named here
//! rather than in a test: a seat that cannot be opened is [`SessionError::Libseat`], a
//! device the seat refuses is [`SessionError::Open`], and a VT switch the seat refuses is
//! [`SessionError::Switch`]. Each is logged with the path or VT it names.

use std::collections::VecDeque;
use std::io;
use std::os::unix::io::OwnedFd;
use std::path::Path;
use std::sync::{Arc, Mutex};

use smithay::backend::session::Session as _;
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::rustix::fs::OFlags;
use tracing::{debug, warn};

/// Whether this process is the one driving the seat's devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activity {
    /// The seat is ours: devices are open and frames may be submitted.
    Active,
    /// Another VT or another session owns the seat: devices are released and no frame may
    /// be submitted.
    Paused,
}

/// What a session event asks the backend to do.
///
/// It is a value rather than a set of callbacks so the backend can act on it inside its own
/// dispatch, where it already holds its devices, and so a repeated event can be recognised
/// as a no-op.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    /// The event repeated the state the session was already in.
    None,
    /// The seat is gone: release DRM master, stop submitting, keep nothing pending.
    Pause,
    /// The seat is ours again. Every output must be re-enabled and damaged in full, which
    /// is what `generation` distinguishes: two resumes never share one.
    Resume {
        /// The redraw epoch the resume starts.
        generation: u64,
    },
}

/// Which state the seat session is in, and how many resumes it has seen.
///
/// It owns no device and reads no clock, so the whole pause/resume protocol of the plan is
/// decided here and tested on a machine with no seat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SessionState {
    activity: Activity,
    generation: u64,
}

impl Default for SessionState {
    /// A session starts active: `libseat` hands the devices over when the session opens,
    /// and the first activation event describes the state we are already in.
    fn default() -> Self {
        Self {
            activity: Activity::Active,
            generation: 0,
        }
    }
}

impl SessionState {
    /// The state a session that was never paused starts in.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this process currently drives the seat.
    #[must_use]
    pub fn activity(self) -> Activity {
        self.activity
    }

    /// Whether the backend may submit frames.
    #[must_use]
    pub fn submits(self) -> bool {
        self.activity == Activity::Active
    }

    /// The redraw epoch, incremented by every resume.
    #[must_use]
    pub fn generation(self) -> u64 {
        self.generation
    }

    /// The seat was taken away.
    ///
    /// A pause for a session that is already paused is [`Transition::None`]: the device was
    /// released by the first event, and releasing it twice is an error the hardware reports.
    pub fn paused(&mut self) -> Transition {
        if self.activity == Activity::Paused {
            return Transition::None;
        }
        self.activity = Activity::Paused;
        Transition::Pause
    }

    /// The seat is ours again.
    ///
    /// A resume for a session that is already active is [`Transition::None`]: the outputs
    /// are already enabled with the intended modes, and re-enabling them is a visible
    /// flash for nothing.
    pub fn resumed(&mut self) -> Transition {
        if self.activity == Activity::Active {
            return Transition::None;
        }
        self.activity = Activity::Active;
        self.generation = self.generation.saturating_add(1);
        Transition::Resume {
            generation: self.generation,
        }
    }
}

/// What the seat told us, in the terms the backend acts on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeatEvent {
    /// The session lost the seat: another VT, or another session on this one.
    Paused,
    /// The session has the seat again.
    Activated,
}

/// Why the seat could not be used.
#[derive(Debug)]
pub enum SessionError {
    /// `libseat` could not open a session. A session with no seat (a test, a nested
    /// session, a boot without a display manager) reaches this.
    Libseat(String),
    /// The seat refused to open a device.
    Open {
        /// The device node.
        path: String,
        /// The reason the seat gave.
        reason: String,
    },
    /// The seat refused a VT switch.
    Switch {
        /// The VT that was asked for.
        vt: i32,
        /// The reason the seat gave.
        reason: String,
    },
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Libseat(reason) => write!(formatter, "no seat session: {reason}"),
            Self::Open { path, reason } => write!(formatter, "the seat refused to open {path}: {reason}"),
            Self::Switch { vt, reason } => write!(formatter, "the seat refused VT {vt}: {reason}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// The session's seat, as this process sees it.
///
/// It owns the `libseat` handle, the seat's name and the queue of events its event source
/// fills. The backend drains [`Seat::take_events`] during its own dispatch, so no event
/// reaches the devices from inside the loop callback that received it.
pub struct Seat {
    session: LibSeatSession,
    name: String,
    events: Arc<Mutex<VecDeque<SeatEvent>>>,
    notifier: Option<LibSeatSessionNotifier>,
}

impl std::fmt::Debug for Seat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Seat")
            .field("name", &self.name)
            .field("active", &self.session.is_active())
            .field("registered", &self.notifier.is_none())
            .finish_non_exhaustive()
    }
}

impl Seat {
    /// Opens the session this process runs in.
    ///
    /// # Errors
    ///
    /// [`SessionError::Libseat`] when there is no seat to open: a session started outside a
    /// logind session, a container, or a machine whose seat is already taken.
    pub fn open() -> Result<Self, SessionError> {
        let (session, notifier) = LibSeatSession::new().map_err(|error| SessionError::Libseat(format!("{error:?}")))?;
        let name = session.seat();
        debug!(seat = %name, "[session] opened the seat session");
        Ok(Self {
            session,
            name,
            events: Arc::new(Mutex::new(VecDeque::new())),
            notifier: Some(notifier),
        })
    }

    /// The seat's name, which is what a udev enumeration is scoped to.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the session currently holds the seat.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.session.is_active()
    }

    /// Opens a device through the seat.
    ///
    /// The descriptor is the seat's to police, so it is the only way a compositor may reach
    /// a device node: opening the path directly would succeed on a machine whose session is
    /// not the active one, and the following ioctls would then fault or fight the real
    /// owner of the display.
    ///
    /// # Errors
    ///
    /// [`SessionError::Open`] when the seat refuses the node, which is what a device that
    /// was removed, a node the session has no permission for, or a paused session reports.
    pub fn open_device(&mut self, path: &Path) -> Result<OwnedFd, SessionError> {
        let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY;
        self.session.open(path, flags).map_err(|error| {
            warn!(path = %path.display(), ?error, "[session] the seat refused a device");
            SessionError::Open {
                path: path.display().to_string(),
                reason: format!("{error:?}"),
            }
        })
    }

    /// Switches to a virtual terminal by number.
    ///
    /// # Errors
    ///
    /// [`SessionError::Switch`] when the seat refuses, which a session with one VT reports.
    pub fn change_vt(&mut self, vt: i32) -> Result<(), SessionError> {
        self.session.change_vt(vt).map_err(|error| {
            warn!(vt, ?error, "[session] the seat refused a VT switch");
            SessionError::Switch {
                vt,
                reason: format!("{error:?}"),
            }
        })
    }

    /// Registers the seat's event source with the compositor's loop.
    ///
    /// The source owns a clone of the seat's descriptor and nothing else, and its callback
    /// only queues what happened: the devices are driven from the backend's own dispatch,
    /// so a VT switch cannot re-enter the render path from inside the loop callback that
    /// noticed it.
    ///
    /// # Errors
    ///
    /// [`io::Error`] when the loop refuses the source. The seat's events are then never
    /// delivered, which is a session that cannot be switched away from: the caller reports
    /// it and stops rather than running a session that would keep the device on a VT
    /// switch.
    pub fn register<S: 'static>(&mut self, loop_handle: &LoopHandle<'static, S>) -> io::Result<()> {
        let Some(notifier) = self.notifier.take() else {
            return Ok(());
        };
        let events = Arc::clone(&self.events);
        loop_handle
            .insert_source(notifier, move |event, _, _data| {
                let queued = match event {
                    smithay::backend::session::Event::PauseSession => Some(SeatEvent::Paused),
                    smithay::backend::session::Event::ActivateSession => Some(SeatEvent::Activated),
                };
                if let Some(event) = queued {
                    let mut queue = events.lock().unwrap_or_else(|error| error.into_inner());
                    queue.push_back(event);
                }
            })
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(())
    }

    /// Everything the seat reported since the last call, oldest first.
    pub fn take_events(&self) -> Vec<SeatEvent> {
        let mut queue = self.events.lock().unwrap_or_else(|error| error.into_inner());
        queue.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_session_is_active_and_submits() {
        let state = SessionState::new();
        assert_eq!(state.activity(), Activity::Active);
        assert!(state.submits());
        assert_eq!(state.generation(), 0);
    }

    #[test]
    fn a_repeated_pause_does_nothing() {
        // Releasing DRM master twice is an error the kernel reports, so the second pause of
        // one switch must not reach the device.
        let mut state = SessionState::new();
        assert_eq!(state.paused(), Transition::Pause);
        assert_eq!(state.paused(), Transition::None);
        assert_eq!(state.activity(), Activity::Paused);
    }

    #[test]
    fn the_first_activation_changes_nothing() {
        // libseat activates the session it just opened; every output is already enabled at
        // the intended mode, so re-enabling them here would be a flash for no change.
        let mut state = SessionState::new();
        assert_eq!(state.resumed(), Transition::None);
        assert_eq!(state.generation(), 0);
    }

    #[test]
    fn a_resume_bumps_the_generation_and_damages_everything() {
        // A resume is a new redraw epoch: the backend keys "that output is stale" off the
        // generation, so two resumes must never share one.
        let mut state = SessionState::new();
        state.paused();
        assert_eq!(state.resumed(), Transition::Resume { generation: 1 });
        state.paused();
        assert_eq!(state.resumed(), Transition::Resume { generation: 2 });
    }

    #[test]
    fn no_frame_is_submitted_from_a_pause_until_its_resume() {
        let mut state = SessionState::new();
        assert!(state.submits());
        state.paused();
        assert!(!state.submits());
        state.paused();
        assert!(!state.submits());
        state.resumed();
        assert!(state.submits());
    }

    #[test]
    fn a_pause_and_resume_pair_advances_one_generation() {
        let mut state = SessionState::new();
        state.paused();
        state.paused();
        state.resumed();
        state.resumed();
        assert_eq!(state.generation(), 1);
    }
}
