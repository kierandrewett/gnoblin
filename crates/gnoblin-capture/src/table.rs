//! The session table: every rule a frame has to satisfy before it exposes pixels.
//!
//! One table owns every session a compositor has open. A protocol handler never
//! copies pixels itself: it asks the table for a claim, and the table decides
//! whether that claim may become a frame at all. That is deliberate, because the
//! failures this crate exists to prevent are all "a path that forgot a rule", and a
//! path cannot forget a rule it never performs.
//!
//! ## The rules, in the order they are applied
//!
//! 1. **The session must still exist.** A claim that outlives its session copies
//!    nothing.
//! 2. **The session must not be stopped.** A stop is final
//!    ([`crate::session::Stop`]).
//! 3. **The claim must be the session's current one.** A claim that an earlier tick
//!    superseded is stale, so a replay of it cannot write to a buffer the client has
//!    since reused.
//! 4. **The lock embargo is asked at every frame**, not once per session. A session
//!    opened before the lock is refused by the very next frame, and it is stopped
//!    rather than paused, so releasing the lock does not quietly resume a stream the
//!    user was never asked about again.
//! 5. **One frame per tick per session.** A client that asks in a loop gets one
//!    frame and a refusal for the rest of the tick, so it cannot spin the
//!    compositor's copy path.
//! 6. **The target must still exist.** A window that closed and an output that was
//!    unplugged both stop the session, which is what keeps a closed window's last
//!    pixels from being served for ever.
//!
//! ## Failure modes first
//!
//! The four the plan names for this card are below, and each is a test at the bottom
//! of this module:
//!
//! 1. **A stream outlives its session.** `a_stream_that_outlives_its_session_copies_nothing`.
//! 2. **A frame is produced after the lock.** `a_frame_after_the_lock_is_refused_and_stops_the_session`
//!    and `a_stopped_session_never_resumes_after_the_unlock`.
//! 3. **A stale buffer is reused.** `a_buffer_that_is_still_claimed_is_refused` and
//!    `a_claim_from_an_earlier_tick_is_stale`.
//! 4. **A closed window source is captured.** `a_closed_window_stops_its_session`.

use std::collections::BTreeMap;

use gnoblin_lock::{Capture, CaptureStream};
use gnoblin_types::{ClientId, Rect, Tick};

use crate::format::Format;
use crate::session::{BufferId, CaptureSessionEvent, Refusal, Session, SessionId, Stop, Target};
use crate::source::{CaptureSource, Copied};

/// One frame's claim on a session's next copy.
///
/// The claim is a typestate: [`Table::capture`] and [`Table::cancel`] consume it by
/// value, so a caller cannot resolve it twice. A claim that is neither captured nor
/// cancelled leaves its buffer marked as in use, which is why it is `#[must_use]`.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "a claim that is neither captured nor cancelled leaves its buffer marked as in use"]
pub struct FrameClaim {
    /// The session the claim belongs to.
    session: SessionId,
    /// The buffer the copy will write into.
    buffer: BufferId,
    /// The frame number this claim will be copied as.
    sequence: u32,
    /// Where the claim was taken, which is what the protocol reports back.
    tick: Tick,
}

impl FrameClaim {
    /// The session the claim belongs to.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// The buffer the copy will write into.
    #[must_use]
    pub const fn buffer(&self) -> BufferId {
        self.buffer
    }

    /// The frame number the copy is stamped with.
    #[must_use]
    pub const fn sequence(&self) -> u32 {
        self.sequence
    }

    /// The tick the claim was taken at.
    ///
    /// A protocol reports it as the frame's presentation time, because it is the
    /// moment the pixels the copy will read were on screen.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        self.tick
    }
}

/// What a session copied, for the protocol events that follow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameReport {
    /// The part of the target that was copied, in the target's own pixels.
    pub region: Rect,
    /// What changed since the session's previous frame, clipped to `region`.
    pub damage: Vec<Rect>,
    /// Whether the pixels are the lock scene rather than the desktop.
    pub lock_scene: bool,
    /// The frame number the client's buffer now holds.
    pub sequence: u32,
}

/// The answer to a capture or a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    /// The client's buffer holds the frame.
    Copied(FrameReport),
    /// Nothing was copied; the client is told why.
    Refused(Refusal),
}

impl CaptureOutcome {
    /// The refusal, if this outcome is one.
    #[must_use]
    pub const fn refusal(&self) -> Option<Refusal> {
        match self {
            Self::Copied(_) => None,
            Self::Refused(refusal) => Some(*refusal),
        }
    }

    /// Whether a frame reached the client's buffer.
    #[must_use]
    pub const fn is_copied(&self) -> bool {
        matches!(self, Self::Copied(_))
    }
}

/// Every capture session the compositor has open.
#[derive(Debug, Default)]
pub struct Table {
    /// The sessions, in identity order.
    sessions: BTreeMap<SessionId, Session>,
    /// The next identity to hand out. It never repeats, so a stale frame cannot be
    /// mistaken for a frame of the session that replaced it.
    next: u64,
}

impl Table {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a session and answers its identity.
    ///
    /// `stream` is the consent the caller can prove: a protocol handler passes
    /// [`CaptureStream::Requested`] because it has none, and the portal path passes
    /// [`CaptureStream::Granted`] after the user answered.
    pub fn open(&mut self, owner: ClientId, target: Target, stream: CaptureStream, format: Format) -> SessionId {
        self.next = self.next.saturating_add(1);
        let id = SessionId::new(self.next);
        self.sessions
            .insert(id, Session::new(id, owner, target, stream, format));
        id
    }

    /// Ends a session for `reason` without waiting for a frame.
    ///
    /// The compositor calls this when it has to cut a stream the client did not ask
    /// about — the lock going up, or a target disappearing — so the stop is recorded
    /// before the next request rather than on the next copy. Answers whether the
    /// session was still live.
    pub fn stop(&mut self, session: SessionId, reason: Stop) -> bool {
        match self.sessions.get_mut(&session) {
            Some(entry) if entry.live() => {
                entry.end(reason);
                true
            }
            _ => false,
        }
    }

    /// Closes a session. Its outstanding claim copies nothing afterwards.
    pub fn close(&mut self, session: SessionId) -> Option<Session> {
        self.sessions.remove(&session)
    }

    /// Closes every session a connection opened, and answers how many there were.
    ///
    /// A connection that disconnects sends no request, so the compositor calls this
    /// when it notices the client is gone. It is the rule the plan states: a session
    /// ends with the client that opened it, so a crashed adapter cannot leave a
    /// stream running.
    pub fn close_owner(&mut self, owner: ClientId) -> usize {
        let doomed: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|session| session.owner == owner)
            .map(|session| session.id)
            .collect();
        let count = doomed.len();
        for session in doomed {
            self.sessions.remove(&session);
        }
        count
    }

    /// Stops and removes every session that captures `target`.
    ///
    /// A window that closed and an output that was unplugged both call this: the
    /// sessions go, so the pixels of either cannot be served afterwards.
    pub fn close_target(&mut self, target: Target) -> usize {
        let doomed: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|session| session.target == target)
            .map(|session| session.id)
            .collect();
        let count = doomed.len();
        for session in doomed {
            self.sessions.remove(&session);
        }
        count
    }

    /// Notes that part of a target changed.
    ///
    /// Damage is clipped to the target's negotiated buffer, so a notification about
    /// a rectangle the buffer cannot hold never reaches a client: the alternative is
    /// a `damage` event naming pixels outside the buffer it describes.
    pub fn note_damage(&mut self, target: Target, damage: Rect) {
        if damage.is_empty() {
            return;
        }
        for session in self.sessions.values_mut().filter(|session| session.target == target) {
            if !session.live() {
                continue;
            }
            if let Some(clipped) = damage.intersection(session.full())
                && !session.damage.contains(&clipped)
            {
                session.damage.push(clipped);
            }
        }
    }

    /// Takes a claim on the session's next frame, for `buffer`.
    ///
    /// # Errors
    ///
    /// [`Refusal::Closed`] for an identity this table does not hold,
    /// [`Refusal::Stopped`] for a stopped session and [`Refusal::Busy`] when the
    /// session already has a claim out for this tick — which is what a client that
    /// re-attaches a buffer it is still reading gets.
    pub fn request(&mut self, session: SessionId, buffer: BufferId, tick: Tick) -> Result<FrameClaim, Refusal> {
        let Some(entry) = self.sessions.get_mut(&session) else {
            return Err(Refusal::Closed);
        };
        let sequence = entry.frames.saturating_add(1);
        // A claim taken on an earlier tick that the loop never serviced is dropped by
        // the transition itself, so the session is never wedged for ever.
        entry
            .state
            .step(CaptureSessionEvent::Request { buffer, tick, sequence })?;
        Ok(FrameClaim {
            session,
            buffer,
            sequence,
            tick,
        })
    }

    /// Copies the frame the claim describes, applying every rule above first.
    ///
    /// `capture` is what the session lock answers for this session's consent right
    /// now, so the embargo is applied at the moment of the copy rather than when the
    /// session was opened.
    pub fn capture(
        &mut self,
        frame: FrameClaim,
        tick: Tick,
        capture: Capture,
        source: &mut dyn CaptureSource,
    ) -> CaptureOutcome {
        let Some(entry) = self.sessions.get_mut(&frame.session) else {
            return CaptureOutcome::Refused(Refusal::Closed);
        };
        if let Err(refusal) = entry.state.step(CaptureSessionEvent::Resolve {
            buffer: frame.buffer,
            sequence: frame.sequence,
        }) {
            return CaptureOutcome::Refused(refusal);
        }
        let lock_scene = match Self::embargo(entry.stream, entry.target, capture) {
            Ok(lock_scene) => lock_scene,
            Err(refusal) => {
                entry.end(Stop::Embargoed);
                return CaptureOutcome::Refused(refusal);
            }
        };
        if entry.last_tick == Some(tick) {
            return CaptureOutcome::Refused(Refusal::Paced);
        }
        let Some(bounds) = source.bounds(entry.target) else {
            entry.end(Stop::TargetGone);
            return CaptureOutcome::Refused(Refusal::TargetGone);
        };
        let Some(region) = entry.full().intersection(bounds) else {
            // The target exists but nothing of it fits the buffer the client
            // negotiated, which happens when a window shrank below the frame it
            // asked for: serving a partial frame would report pixels it never wrote.
            entry.end(Stop::TargetGone);
            return CaptureOutcome::Refused(Refusal::TargetGone);
        };
        let damage = entry.take_damage(region, lock_scene);
        if source.copy(entry.target, region, entry.format) == Copied::Nothing {
            entry.restore_damage(damage);
            return CaptureOutcome::Refused(Refusal::Source);
        }
        entry.frames = entry.frames.saturating_add(1);
        entry.last_tick = Some(tick);
        CaptureOutcome::Copied(FrameReport {
            region,
            damage,
            lock_scene,
            sequence: frame.sequence,
        })
    }

    /// Gives up a claim without copying: the client destroyed its frame object, or
    /// the loop decided not to service it this tick.
    pub fn cancel(&mut self, frame: FrameClaim) {
        if let Some(entry) = self.sessions.get_mut(&frame.session) {
            entry.release(frame.sequence);
        }
    }

    /// Applies the lock's answer to one session.
    ///
    /// A stream with no grant is refused and stopped. A granted output stream may
    /// continue and receives the lock scene, which is the one thing the session lock
    /// allows a monitor stream to show while it covers the desktop; a granted
    /// *window* stream is refused, because a window is desktop content and the lock
    /// scene is not a window.
    fn embargo(stream: CaptureStream, target: Target, capture: Capture) -> Result<bool, Refusal> {
        match capture {
            Capture::Desktop => Ok(false),
            Capture::LockScene => match (stream, target) {
                (CaptureStream::Granted, Target::Output(_)) => Ok(true),
                _ => Err(Refusal::Locked),
            },
            Capture::Refused => Err(Refusal::Locked),
        }
    }

    /// One session.
    #[must_use]
    pub fn get(&self, session: SessionId) -> Option<&Session> {
        self.sessions.get(&session)
    }

    /// The sessions, in identity order.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// How many sessions are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether no session is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_lock::{Capture, CaptureStream};
    use gnoblin_types::{ClientId, NodeId, OutputId, Rect, Size, Tick, WindowId};

    use crate::format::{Format, PixelFormat};
    use crate::session::{BufferId, Refusal, SessionId, Stop, Target};
    use crate::source::{CaptureSource, Copied};
    use crate::table::{CaptureOutcome, Table};

    const SIZE: Size = Size::new(16, 8);

    /// A source that answers whatever the test asked it to and records the copies it
    /// was asked for, so a test can prove a copy happened *again* rather than being
    /// answered from a remembered frame.
    struct Fake {
        bounds: Option<Rect>,
        copies: Vec<(Target, Rect)>,
        refuse: bool,
    }

    impl Fake {
        fn new(bounds: Option<Rect>) -> Self {
            Self {
                bounds,
                copies: Vec::new(),
                refuse: false,
            }
        }
    }

    impl CaptureSource for Fake {
        fn bounds(&self, _target: Target) -> Option<Rect> {
            self.bounds
        }

        fn copy(&mut self, target: Target, region: Rect, _format: Format) -> Copied {
            self.copies.push((target, region));
            if self.refuse { Copied::Nothing } else { Copied::Copied }
        }
    }

    fn format() -> Format {
        Format::new(SIZE, SIZE.width * 4, PixelFormat::Argb8888).expect("a valid format")
    }

    fn output() -> Target {
        Target::Output(OutputId::new(1))
    }

    fn open(table: &mut Table, stream: CaptureStream) -> SessionId {
        table.open(ClientId::new(9), output(), stream, format())
    }

    /// Failure mode 1: a stream outlives its session and keeps serving the client
    /// that closed it.
    #[test]
    fn a_stream_that_outlives_its_session_copies_nothing() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        table.close(session);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let outcome = table.capture(frame, Tick::new(1), Capture::Desktop, &mut source);
        assert_eq!(outcome, CaptureOutcome::Refused(Refusal::Closed));
        assert!(source.copies.is_empty(), "nothing was copied for a closed session");
        assert!(table.is_empty(), "the session is gone");
    }

    /// Failure mode 2: a frame is produced while the session lock covers the desktop,
    /// and the stream stops for good.
    #[test]
    fn a_frame_after_the_lock_is_refused_and_stops_the_session() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert!(
            table
                .capture(first, Tick::new(1), Capture::Desktop, &mut source)
                .is_copied()
        );
        let second = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        assert_eq!(
            table.capture(second, Tick::new(2), Capture::Refused, &mut source),
            CaptureOutcome::Refused(Refusal::Locked)
        );
        assert_eq!(
            table.get(session).and_then(|session| session.stopped()),
            Some(Stop::Embargoed),
            "the session is stopped, not paused"
        );
        assert_eq!(source.copies.len(), 1, "the refused frame copied nothing");
    }

    /// Failure mode 2, second half: releasing the lock does not resume the stream.
    #[test]
    fn a_stopped_session_never_resumes_after_the_unlock() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert_eq!(
            table.capture(frame, Tick::new(1), Capture::Refused, &mut source),
            CaptureOutcome::Refused(Refusal::Locked)
        );
        assert_eq!(
            table.request(session, BufferId::new(2), Tick::new(2)),
            Err(Refusal::Stopped(Stop::Embargoed))
        );
    }

    /// A granted monitor stream is the one thing the lock lets through, and it gets
    /// the lock scene rather than the desktop it covers.
    #[test]
    fn a_granted_output_stream_sees_the_lock_scene() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Granted);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        let outcome = table.capture(frame, Tick::new(1), Capture::LockScene, &mut source);
        let CaptureOutcome::Copied(report) = outcome else {
            panic!("a granted stream sees the lock scene, got {outcome:?}");
        };
        assert!(report.lock_scene);
        assert_eq!(report.damage, vec![Rect::from_size(SIZE)], "the whole scene is new");
        assert!(table.get(session).is_some_and(|session| session.live()));
    }

    /// A window is desktop content: a granted stream of one cannot be shown the lock
    /// scene, and is stopped instead of silently continuing with stale pixels.
    #[test]
    fn a_granted_window_stream_is_refused_under_the_lock() {
        let mut table = Table::new();
        let session = table.open(
            ClientId::new(9),
            Target::Window(WindowId::new(3)),
            CaptureStream::Granted,
            format(),
        );
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert_eq!(
            table.capture(frame, Tick::new(1), Capture::LockScene, &mut source),
            CaptureOutcome::Refused(Refusal::Locked)
        );
    }

    /// A View is desktop content like a window: a granted View stream is refused the
    /// lock scene and stopped, never fed stale pixels.
    #[test]
    fn a_granted_view_stream_is_refused_under_the_lock() {
        let mut table = Table::new();
        let session = table.open(
            ClientId::new(9),
            Target::View(NodeId::new(4)),
            CaptureStream::Granted,
            format(),
        );
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert_eq!(
            table.capture(frame, Tick::new(1), Capture::LockScene, &mut source),
            CaptureOutcome::Refused(Refusal::Locked)
        );
        assert_eq!(
            table.get(session).and_then(|session| session.stopped()),
            Some(Stop::Embargoed),
            "the View stream is stopped"
        );
        assert!(source.copies.is_empty(), "the refused frame copied nothing");
    }

    /// A View stream that was live when the lock came up is stopped by its next frame,
    /// and releasing the lock does not resume it.
    #[test]
    fn a_view_stream_is_stopped_by_the_lock_and_never_resumes() {
        let mut table = Table::new();
        let session = table.open(
            ClientId::new(9),
            Target::View(NodeId::new(4)),
            CaptureStream::Requested,
            format(),
        );
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert!(
            table
                .capture(first, Tick::new(1), Capture::Desktop, &mut source)
                .is_copied()
        );
        let second = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        assert_eq!(
            table.capture(second, Tick::new(2), Capture::Refused, &mut source),
            CaptureOutcome::Refused(Refusal::Locked)
        );
        assert_eq!(
            table.request(session, BufferId::new(3), Tick::new(3)),
            Err(Refusal::Stopped(Stop::Embargoed)),
            "unlocking does not resume the View stream"
        );
        assert_eq!(source.copies.len(), 1, "the refused frame copied nothing");
    }

    /// Failure mode 3: a buffer that is still claimed is handed out again, and the
    /// client reads a torn frame.
    #[test]
    fn a_buffer_that_is_still_claimed_is_refused() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let _claim = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert_eq!(
            table.request(session, BufferId::new(1), Tick::new(1)),
            Err(Refusal::Busy)
        );
    }

    /// Failure mode 3, the other half: a claim from an earlier tick is superseded, so
    /// a replay of it writes nothing.
    #[test]
    fn a_claim_from_an_earlier_tick_is_stale() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let old = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        let current = table
            .request(session, BufferId::new(2), Tick::new(2))
            .expect("the next tick's claim");
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        assert_eq!(
            table.capture(old, Tick::new(2), Capture::Desktop, &mut source),
            CaptureOutcome::Refused(Refusal::Stale)
        );
        assert!(
            table
                .capture(current, Tick::new(2), Capture::Desktop, &mut source)
                .is_copied()
        );
        assert_eq!(source.copies.len(), 1, "only the current claim copied");
    }

    /// Failure mode 4: a closed window source keeps being served. The session is
    /// stopped and the caller is told the target is gone.
    #[test]
    fn a_closed_window_stops_its_session() {
        let mut table = Table::new();
        let session = table.open(
            ClientId::new(9),
            Target::Window(WindowId::new(4)),
            CaptureStream::Requested,
            format(),
        );
        let mut source = Fake::new(None);
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert_eq!(
            table.capture(frame, Tick::new(1), Capture::Desktop, &mut source),
            CaptureOutcome::Refused(Refusal::TargetGone)
        );
        assert_eq!(
            table.get(session).and_then(|session| session.stopped()),
            Some(Stop::TargetGone)
        );
        assert_eq!(
            table.request(session, BufferId::new(2), Tick::new(2)),
            Err(Refusal::Stopped(Stop::TargetGone))
        );
    }

    /// A stream the compositor cut itself stops before its next frame, which is how
    /// the lock stops a session no client asked about.
    #[test]
    fn a_session_the_compositor_stops_serves_nothing() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        assert!(table.stop(session, Stop::Embargoed));
        assert!(!table.stop(session, Stop::TargetGone), "the first reason stands");
        assert_eq!(
            table.request(session, BufferId::new(1), Tick::new(1)),
            Err(Refusal::Stopped(Stop::Embargoed))
        );
    }

    /// A frame claim is consumed by its first resolution, so a second one cannot be
    /// written: `capture` and `cancel` take the claim by value, and the compiler
    /// refuses a second use. The table then refuses the replay of a claim that has
    /// already been superseded, which is the runtime half of the same rule.
    #[test]
    fn a_frame_is_copied_once() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert!(
            table
                .capture(frame, Tick::new(1), Capture::Desktop, &mut source)
                .is_copied()
        );
        assert_eq!(source.copies.len(), 1);
        // A cancelled claim is consumed too, and releases its slot for the next request.
        let cancelled = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        table.cancel(cancelled);
        assert!(
            table.request(session, BufferId::new(3), Tick::new(3)).is_ok(),
            "a cancelled claim leaves the session ready for the next frame"
        );
    }

    /// A claim taken on an earlier tick that the loop never serviced is dropped by
    /// the next request, so the session is never wedged for ever.
    #[test]
    fn a_stale_claim_from_an_earlier_tick_is_dropped_on_the_next_request() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let _unserviced = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        let next = table
            .request(session, BufferId::new(2), Tick::new(2))
            .expect("the later tick's request drops the unserviced claim");
        assert!(
            table
                .capture(next, Tick::new(2), Capture::Desktop, &mut source)
                .is_copied()
        );
        assert_eq!(source.copies.len(), 1, "only the later claim copied");
    }

    /// A client that asks in a loop gets one frame per tick.
    #[test]
    fn a_session_produces_one_frame_per_tick() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(5)).expect("a claim");
        assert!(
            table
                .capture(first, Tick::new(5), Capture::Desktop, &mut source)
                .is_copied()
        );
        let second = table
            .request(session, BufferId::new(2), Tick::new(5))
            .expect("a claim from the same tick");
        assert_eq!(
            table.capture(second, Tick::new(5), Capture::Desktop, &mut source),
            CaptureOutcome::Refused(Refusal::Paced)
        );
        assert_eq!(source.copies.len(), 1);
    }

    /// The damage the client is told about is the damage noted since the previous
    /// frame, and only that.
    #[test]
    fn the_second_frame_reports_only_the_changed_region() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        let first = table.capture(first, Tick::new(1), Capture::Desktop, &mut source);
        assert!(
            first
                == CaptureOutcome::Copied(crate::table::FrameReport {
                    region: Rect::from_size(SIZE),
                    damage: vec![Rect::from_size(SIZE)],
                    lock_scene: false,
                    sequence: 1,
                })
        );
        table.note_damage(output(), Rect::new(2, 2, 4, 4));
        table.note_damage(output(), Rect::new(100, 100, 4, 4));
        let second = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        let CaptureOutcome::Copied(report) = table.capture(second, Tick::new(2), Capture::Desktop, &mut source) else {
            panic!("the second frame copies");
        };
        assert_eq!(
            report.damage,
            vec![Rect::new(2, 2, 4, 4)],
            "only the changed region inside the buffer is reported"
        );
    }

    /// A source that cannot copy does not lose the damage it owed, or the change
    /// would never be reported to any client.
    #[test]
    fn a_failed_copy_keeps_the_damage_for_the_next_frame() {
        let mut table = Table::new();
        let session = open(&mut table, CaptureStream::Requested);
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        assert!(
            table
                .capture(first, Tick::new(1), Capture::Desktop, &mut source)
                .is_copied()
        );
        table.note_damage(output(), Rect::new(3, 3, 2, 2));
        source.refuse = true;
        let second = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        assert_eq!(
            table.capture(second, Tick::new(2), Capture::Desktop, &mut source),
            CaptureOutcome::Refused(Refusal::Source)
        );
        source.refuse = false;
        let third = table.request(session, BufferId::new(3), Tick::new(3)).expect("a claim");
        let CaptureOutcome::Copied(report) = table.capture(third, Tick::new(3), Capture::Desktop, &mut source) else {
            panic!("the third frame copies");
        };
        assert_eq!(report.damage, vec![Rect::new(3, 3, 2, 2)]);
    }

    /// Two clients' sessions are closed independently: one disconnecting leaves the
    /// other capturing.
    #[test]
    fn closing_an_owner_closes_only_its_sessions() {
        let mut table = Table::new();
        let mine = open(&mut table, CaptureStream::Requested);
        let other = table.open(
            ClientId::new(10),
            Target::Output(OutputId::new(2)),
            CaptureStream::Requested,
            format(),
        );
        assert_eq!(table.close_owner(ClientId::new(9)), 1);
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(mine).map(|_| ()), None);
        assert!(table.get(other).is_some());
    }

    /// A window that closes takes its sessions with it, so nothing serves the pixels
    /// of a window that is no longer on screen.
    #[test]
    fn closing_a_target_closes_its_sessions() {
        let mut table = Table::new();
        let window = Target::Window(WindowId::new(5));
        table.open(ClientId::new(9), window, CaptureStream::Requested, format());
        table.open(ClientId::new(9), output(), CaptureStream::Requested, format());
        assert_eq!(table.close_target(window), 1);
        assert_eq!(table.len(), 1);
    }

    /// Identities never repeat, so a frame of a closed session cannot be replayed
    /// against the session that took its number.
    #[test]
    fn identities_are_never_reused() {
        let mut table = Table::new();
        let first = open(&mut table, CaptureStream::Requested);
        table.close(first);
        let second = open(&mut table, CaptureStream::Requested);
        assert_ne!(first, second);
    }

    /// A View's frames follow the View's own rectangle: after the picture is
    /// resized the next frame covers the new size, and damage from before the resize
    /// is clipped to it rather than delivered against a region the buffer no longer
    /// shows.
    #[test]
    fn a_view_frame_follows_the_view_rect_after_a_resize() {
        let view = Target::View(NodeId::new(4));
        let mut table = Table::new();
        let session = table.open(ClientId::new(9), view, CaptureStream::Requested, format());
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let first = table.request(session, BufferId::new(1), Tick::new(1)).expect("a claim");
        let CaptureOutcome::Copied(report) = table.capture(first, Tick::new(1), Capture::Desktop, &mut source) else {
            panic!("the first View frame copies");
        };
        assert_eq!(report.region, Rect::from_size(SIZE));
        assert_eq!(report.damage, vec![Rect::from_size(SIZE)], "the first frame is whole");

        // No change since the last frame: the next frame delivers no damage.
        let second = table.request(session, BufferId::new(2), Tick::new(2)).expect("a claim");
        let CaptureOutcome::Copied(quiet) = table.capture(second, Tick::new(2), Capture::Desktop, &mut source) else {
            panic!("the quiet View frame still copies");
        };
        assert!(quiet.damage.is_empty(), "an unchanged View reports no damage");

        // The View is damaged across its whole rectangle, then shrinks: the frame
        // covers the smaller rectangle and the damage is clipped to it.
        table.note_damage(view, Rect::from_size(SIZE));
        let shrunk = Rect::new(0, 0, 8, 4);
        source.bounds = Some(shrunk);
        let third = table.request(session, BufferId::new(3), Tick::new(3)).expect("a claim");
        let CaptureOutcome::Copied(resized) = table.capture(third, Tick::new(3), Capture::Desktop, &mut source) else {
            panic!("the resized View frame copies");
        };
        assert_eq!(resized.region, shrunk, "the frame covers the View's new rectangle");
        assert_eq!(resized.damage, vec![shrunk], "damage is clipped to the new rectangle");
    }

    /// A View's damage is delivered to its own sessions only: a window's session and
    /// a View's session with the same numeric id do not share it.
    #[test]
    fn view_damage_does_not_reach_other_targets() {
        let mut table = Table::new();
        let view = table.open(
            ClientId::new(9),
            Target::View(NodeId::new(4)),
            CaptureStream::Requested,
            format(),
        );
        let window = table.open(
            ClientId::new(9),
            Target::Window(WindowId::new(4)),
            CaptureStream::Requested,
            format(),
        );
        let mut source = Fake::new(Some(Rect::from_size(SIZE)));
        let frame = table.request(window, BufferId::new(1), Tick::new(1)).expect("a claim");
        table.capture(frame, Tick::new(1), Capture::Desktop, &mut source);
        table.note_damage(Target::View(NodeId::new(4)), Rect::new(0, 0, 4, 4));
        let frame = table.request(window, BufferId::new(2), Tick::new(2)).expect("a claim");
        let CaptureOutcome::Copied(report) = table.capture(frame, Tick::new(2), Capture::Desktop, &mut source) else {
            panic!("the window frame copies");
        };
        assert!(report.damage.is_empty(), "the window does not see the View's damage");
        let frame = table.request(view, BufferId::new(3), Tick::new(3)).expect("a claim");
        table.capture(frame, Tick::new(3), Capture::Desktop, &mut source);
        table.note_damage(Target::View(NodeId::new(4)), Rect::new(0, 0, 4, 4));
        let frame = table.request(view, BufferId::new(4), Tick::new(4)).expect("a claim");
        let CaptureOutcome::Copied(view_report) = table.capture(frame, Tick::new(4), Capture::Desktop, &mut source)
        else {
            panic!("the View frame copies");
        };
        assert_eq!(view_report.damage, vec![Rect::new(0, 0, 4, 4)]);
    }
}
