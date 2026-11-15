//! The capture table against the real session lock, through whole lock cycles.
//!
//! `gnoblin-lock` answers what a capture path may return and `gnoblin-capture` applies the answer at every frame.
//! Each crate tests its half with a stand-in for the other (the table with a hand-written [`Capture`] argument, the
//! lock with its own queries), so neither proves the property the plan states for the pair: while the session is not
//! unlocked, no capture reads the desktop. These tests join them. The lock is the real [`Lock`] driven by its public
//! events, the table is the real [`Table`], and the source records every copy with the state the lock was in when it
//! was made, so a leak is a recorded fact rather than an inference from a refusal.
//!
//! The failure-mode list these belong to is in the crate documentation of `gnoblin-capture`.

use gnoblin_capture::{
    BufferId, CaptureOutcome, CaptureSource, Copied, Format, PixelFormat, Refusal, SessionId, Stop, Table, Target,
};
use gnoblin_lock::{CaptureStream, CoverFailure, Event, Lock, State};
use gnoblin_types::{ClientId, NodeId, OutputId, Rect, Size, Tick, WindowId};

const LOCKER: ClientId = ClientId::new(1);
const REPLACEMENT: ClientId = ClientId::new(2);
const CLIENT: ClientId = ClientId::new(9);
const OUTPUT: OutputId = OutputId::new(1);
const SIZE: Size = Size::new(16, 8);

/// One copy the source was asked for, with what the lock said at that moment.
#[derive(Clone, Copy, Debug)]
struct Taken {
    target: Target,
    state: State,
}

/// A source that has pixels for every target and records each copy with the lock's state.
struct Recorder<'a> {
    lock: &'a Lock,
    copies: Vec<Taken>,
}

impl CaptureSource for Recorder<'_> {
    fn bounds(&self, _target: Target) -> Option<Rect> {
        Some(Rect::from_size(SIZE))
    }

    fn copy(&mut self, target: Target, _region: Rect, _format: Format) -> Copied {
        self.copies.push(Taken {
            target,
            state: self.lock.state(),
        });
        Copied::Copied
    }
}

/// The real lock, the real table and the tick the loop is on.
struct World {
    lock: Lock,
    table: Table,
    tick: u64,
    buffer: u64,
    copies: Vec<Taken>,
}

impl World {
    /// An unlocked session with one output.
    fn new() -> Self {
        let mut lock = Lock::new();
        lock.apply(Event::OutputsChanged { outputs: vec![OUTPUT] });
        Self {
            lock,
            table: Table::new(),
            tick: 0,
            buffer: 0,
            copies: Vec::new(),
        }
    }

    fn format() -> Format {
        Format::new(SIZE, SIZE.width * 4, PixelFormat::Argb8888).expect("a valid format")
    }

    fn open(&mut self, target: Target, stream: CaptureStream) -> SessionId {
        self.table.open(CLIENT, target, stream, Self::format())
    }

    /// One loop iteration: every session in `sessions` asks for a frame, and the lock's answer for each session's own
    /// stream is applied exactly as the compositor applies it. The result is each session's outcome, in order.
    fn frame(&mut self, sessions: &[SessionId]) -> Vec<CaptureOutcome> {
        self.tick += 1;
        let tick = Tick::new(self.tick);
        let mut outcomes = Vec::new();
        for &session in sessions {
            let Some(stream) = self.table.get(session).map(|entry| entry.stream()) else {
                outcomes.push(CaptureOutcome::Refused(Refusal::Closed));
                continue;
            };
            self.buffer += 1;
            let claim = match self.table.request(session, BufferId::new(self.buffer), tick) {
                Ok(claim) => claim,
                Err(refusal) => {
                    outcomes.push(CaptureOutcome::Refused(refusal));
                    continue;
                }
            };
            let capture = self.lock.capture(stream);
            let mut source = Recorder {
                lock: &self.lock,
                copies: Vec::new(),
            };
            outcomes.push(self.table.capture(claim, tick, capture, &mut source));
            self.copies.extend(source.copies);
        }
        outcomes
    }

    fn apply(&mut self, event: Event) {
        self.lock.apply(event);
    }

    /// The lock request, the locker's surface and the presented cover: `Locked`, owned by `locker`.
    fn lock_as(&mut self, locker: ClientId, generation: u64) {
        self.apply(Event::LockRequested { locker });
        assert_eq!(self.lock.state(), State::Locking);
        self.apply(Event::LockerSurfaceMapped { locker, output: OUTPUT });
        self.apply(Event::CoverPresented {
            generation,
            output: OUTPUT,
        });
        assert_eq!(self.lock.state(), State::Locked);
    }

    fn stopped(&self, session: SessionId) -> Option<Stop> {
        self.table.get(session).and_then(|entry| entry.stopped())
    }
}

fn is_copied(outcome: &CaptureOutcome) -> bool {
    matches!(outcome, CaptureOutcome::Copied(_))
}

/// The plan's property for the pair: through a whole cycle (lock requested, locked, unlock requested, revealed),
/// every desktop frame was copied while the lock was `Unlocked`, and every frame copied at any other time was the
/// lock scene of a granted output.
#[test]
fn no_session_copies_the_desktop_while_the_lock_is_not_released() {
    let mut world = World::new();
    let requested = world.open(Target::Output(OUTPUT), CaptureStream::Requested);
    let granted_output = world.open(Target::Output(OUTPUT), CaptureStream::Granted);
    let granted_window = world.open(Target::Window(WindowId::new(3)), CaptureStream::Granted);
    let granted_view = world.open(Target::View(NodeId::new(4)), CaptureStream::Granted);
    let all = [requested, granted_output, granted_window, granted_view];

    assert!(
        world.frame(&all).iter().all(is_copied),
        "an unlocked desktop is captured by every session"
    );
    let unlocked_copies = world.copies.len();
    assert_eq!(unlocked_copies, 4);

    // The lock engages and the loop runs before the cover is proven: the state is `Locking`.
    world.apply(Event::LockRequested { locker: LOCKER });
    assert_eq!(world.lock.state(), State::Locking);
    let outcomes = world.frame(&all);
    assert!(
        outcomes
            .iter()
            .all(|outcome| *outcome == CaptureOutcome::Refused(Refusal::Locked)),
        "no session, granted or not, is served while the cover is being proven: {outcomes:?}"
    );
    for session in all {
        assert_eq!(
            world.stopped(session),
            Some(Stop::Embargoed),
            "the lock stops {session}"
        );
    }

    world.apply(Event::LockerSurfaceMapped {
        locker: LOCKER,
        output: OUTPUT,
    });
    world.apply(Event::CoverPresented {
        generation: 1,
        output: OUTPUT,
    });
    assert_eq!(world.lock.state(), State::Locked);
    assert!(
        !world.frame(&all).iter().any(is_copied),
        "a session the lock stopped is not resumed by the lock settling"
    );

    // The owner authenticates: the desktop is coming back but is not back until the reveal completes.
    world.apply(Event::UnlockRequested { locker: LOCKER });
    assert_eq!(world.lock.state(), State::Unlocking);
    assert!(!world.frame(&all).iter().any(is_copied));
    world.apply(Event::RevealComplete);
    assert_eq!(world.lock.state(), State::Unlocked);
    assert!(
        !world.frame(&all).iter().any(is_copied),
        "releasing the lock does not resume a stream the user was not asked about again"
    );

    // Every copy made in the whole cycle was made while unlocked: the four from the first frame.
    assert_eq!(world.copies.len(), unlocked_copies);
    assert!(
        world.copies.iter().all(|copy| copy.state == State::Unlocked),
        "{:?}",
        world.copies
    );

    // A session opened after the reveal is a new consent decision, and it sees the desktop again.
    let fresh = world.open(Target::Output(OUTPUT), CaptureStream::Requested);
    assert!(world.frame(&[fresh]).iter().all(is_copied));
}

/// A granted stream of an output that no frame was asked of while the lock was `Locking` (the loop did not run in
/// that state) shows the lock scene once `Locked`, never the desktop, and is refused again the moment the owner asks
/// to unlock. A granted stream of a window or a View is refused at once: the lock scene is not a window.
#[test]
fn a_granted_output_sees_the_lock_scene_only_and_only_while_locked() {
    let mut world = World::new();
    let output = world.open(Target::Output(OUTPUT), CaptureStream::Granted);
    let window = world.open(Target::Window(WindowId::new(3)), CaptureStream::Granted);
    let view = world.open(Target::View(NodeId::new(4)), CaptureStream::Granted);
    world.lock_as(LOCKER, 1);

    let outcomes = world.frame(&[output, window, view]);
    let CaptureOutcome::Copied(report) = &outcomes[0] else {
        panic!("a granted output shows the lock scene, got {:?}", outcomes[0]);
    };
    assert!(report.lock_scene, "what it shows is the lock scene");
    assert_eq!(report.damage, vec![Rect::from_size(SIZE)], "the whole scene is new");
    assert_eq!(outcomes[1], CaptureOutcome::Refused(Refusal::Locked));
    assert_eq!(outcomes[2], CaptureOutcome::Refused(Refusal::Locked));
    assert_eq!(world.copies.len(), 1);
    assert_eq!(world.copies[0].state, State::Locked);
    assert_eq!(
        world.copies[0].target,
        Target::Output(OUTPUT),
        "and it was the output's"
    );

    world.apply(Event::UnlockRequested { locker: LOCKER });
    assert_eq!(
        world.frame(&[output])[0],
        CaptureOutcome::Refused(Refusal::Locked),
        "an unlock that has not completed shows nothing"
    );
    assert_eq!(world.stopped(output), Some(Stop::Embargoed));
    world.apply(Event::RevealComplete);
    assert_eq!(
        world.frame(&[output])[0],
        CaptureOutcome::Refused(Refusal::Stopped(Stop::Embargoed)),
        "and the reveal does not bring the stream back"
    );
}

/// A cover that fails keeps the embargo whole: a granted output that was being shown the lock scene is refused, a
/// replacement locker's takeover does not give the old stream back, and the stream the replacement's lock allows is a
/// new session with its own consent.
#[test]
fn a_failed_cover_and_a_takeover_never_hand_a_stopped_stream_back() {
    let mut world = World::new();
    let old = world.open(Target::Output(OUTPUT), CaptureStream::Granted);
    world.lock_as(LOCKER, 1);
    assert!(
        is_copied(&world.frame(&[old])[0]),
        "the lock scene is shown while the cover stands"
    );

    world.apply(Event::CoverFailed {
        generation: 1,
        reason: CoverFailure::RenderFailed,
    });
    assert_eq!(world.lock.state(), State::CoverFailed);
    assert_eq!(
        world.frame(&[old])[0],
        CaptureOutcome::Refused(Refusal::Locked),
        "a cover that failed shows nothing of the session, not even the scene"
    );
    assert_eq!(world.stopped(old), Some(Stop::Embargoed));

    // The locker dies; another takes over and presents a fresh cover.
    world.apply(Event::LockerGone { locker: LOCKER });
    world.apply(Event::LockRequested { locker: REPLACEMENT });
    assert_eq!(world.lock.state(), State::Locking);
    world.apply(Event::CoverPresented {
        generation: 2,
        output: OUTPUT,
    });
    assert_eq!(world.lock.state(), State::Locked);
    assert_eq!(
        world.frame(&[old])[0],
        CaptureOutcome::Refused(Refusal::Stopped(Stop::Embargoed)),
        "the takeover does not inherit the old stream"
    );

    let fresh = world.open(Target::Output(OUTPUT), CaptureStream::Granted);
    assert!(is_copied(&world.frame(&[fresh])[0]));
    assert!(world.copies.iter().all(|copy| copy.state == State::Locked));
}
