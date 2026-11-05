//! What a session captures, who owns it, and where it stands.
//!
//! A session is the compositor's object for one client's stream of frames: the
//! output or window it exports, the buffer shape the client negotiated, how many
//! frames it has had, and the damage it still owes. The *rules* about frames live in
//! [`crate::table`]; this module is the value they act on.

use gnoblin_lock::CaptureStream;
use gnoblin_types::{ClientId, OutputId, Rect, Tick, WindowId};

use crate::format::Format;

/// One compositor identity around a session's number.
///
/// Sessions are numbered from one and a number is never reused, so a frame that
/// outlives its session can be told apart from a frame of the session that replaced
/// it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(u64);

impl SessionId {
    /// Wraps a token, for a wire decoder or a test.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw token.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "SessionId({})", self.0)
    }
}

/// The caller's name for the buffer a frame copies into.
///
/// The compositor numbers the client's buffers; this crate only needs to tell two
/// of them apart, so that a client that re-attaches a buffer which is still being
/// copied into is refused instead of tearing the frame it is still reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferId(u64);

impl BufferId {
    /// Wraps a token, for a wire decoder or a test.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw token.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// What a session exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// A whole output, identified the way the layout numbers it.
    Output(OutputId),
    /// One window, which follows the window as it moves and ends with it.
    Window(WindowId),
}

impl std::fmt::Display for Target {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Output(id) => write!(formatter, "output {id}"),
            Self::Window(id) => write!(formatter, "window {id}"),
        }
    }
}

/// Why a session can produce no further frames.
///
/// A stop is final. Nothing resumes a stopped session, because every reason for one
/// is a decision that must not be undone by a later event: the lock embargo may be
/// released, the window may reopen and the output may be replugged, but a stream
/// that was cut for any of them gets a new session, with a new consent decision,
/// rather than silently continuing as the old one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stop {
    /// The lock embargo refused a frame while the session was live.
    Embargoed,
    /// The output or window it captured is gone.
    TargetGone,
}

impl Stop {
    /// The name a log line and a test use.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Embargoed => "embargoed",
            Self::TargetGone => "target-gone",
        }
    }
}

/// Why a request or a frame produced nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// No session has this identity: the client closed it or went away.
    Closed,
    /// The session was stopped.
    Stopped(Stop),
    /// The lock embargo forbade this frame.
    ///
    /// This is the only refusal a client is told about as `failed` rather than as a
    /// protocol error, because it is not the client's fault and it is not a bug.
    Locked,
    /// This session already produced a frame in this tick.
    Paced,
    /// This session still has a claim out, so a second one would copy into a buffer
    /// the client may already be reading.
    Busy,
    /// The claim belongs to an earlier request, which this session superseded.
    Stale,
    /// The same frame object was resolved once already.
    Used,
    /// The source had no pixels for the region.
    Source,
    /// The target no longer exists.
    TargetGone,
}

impl Refusal {
    /// The stable machine-readable reason a protocol event carries.
    ///
    /// The strings are the ones the capture protocols' own failure vocabulary is
    /// mapped onto in `gnoblin-wayland`, and they never change: a client, a test and
    /// a log reader key off them.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Stopped(_) => "stopped",
            Self::Locked => "locked",
            Self::Paced => "paced",
            Self::Busy => "busy",
            Self::Stale => "stale",
            Self::Used => "used",
            Self::Source => "source",
            Self::TargetGone => "target-gone",
        }
    }
}

/// One session's state.
///
/// It is a value the table owns; a caller reads it to know which session to stop,
/// what it captures and why it ended, and never mutates it directly.
#[derive(Clone, Debug)]
pub struct Session {
    /// The session's identity.
    pub(crate) id: SessionId,
    /// The connection that opened it, so a disconnecting client takes its sessions.
    pub(crate) owner: ClientId,
    /// What it captures.
    pub(crate) target: Target,
    /// Whether the session holds a consent grant.
    ///
    /// A session opened straight from a capture protocol is
    /// [`CaptureStream::Requested`]: it never holds a grant, so the lock embargo
    /// refuses every frame of it. Only a caller that recorded the user's consent —
    /// the portal path — opens a `Granted` one.
    pub(crate) stream: CaptureStream,
    /// The buffer shape the client negotiated.
    pub(crate) format: Format,
    /// How many frames have been copied.
    pub(crate) frames: u32,
    /// The tick the last frame was copied at, which is what paces the stream.
    pub(crate) last_tick: Option<Tick>,
    /// The tick the outstanding claim was taken at.
    pub(crate) claim_tick: Option<Tick>,
    /// The outstanding claim: the buffer and the frame number it will be copied as.
    pub(crate) claim: Option<(BufferId, u32)>,
    /// The damage owed to the next frame.
    pub(crate) damage: Vec<Rect>,
    /// Why the session may produce no further frames.
    pub(crate) stop: Option<Stop>,
}

impl Session {
    /// A session that has produced nothing yet.
    pub(crate) const fn new(
        id: SessionId,
        owner: ClientId,
        target: Target,
        stream: CaptureStream,
        format: Format,
    ) -> Self {
        Self {
            id,
            owner,
            target,
            stream,
            format,
            frames: 0,
            last_tick: None,
            claim_tick: None,
            claim: None,
            damage: Vec::new(),
            stop: None,
        }
    }

    /// Ends the session for `reason`, if it is still live.
    pub(crate) fn end(&mut self, reason: Stop) {
        if self.stop.is_none() {
            self.stop = Some(reason);
        }
        self.claim = None;
        self.claim_tick = None;
        self.damage.clear();
    }

    /// Drops the outstanding claim, if it is this frame's.
    pub(crate) fn release(&mut self, sequence: u32) {
        if self.claim.is_some_and(|(_, claimed)| claimed == sequence) {
            self.claim = None;
            self.claim_tick = None;
        }
    }

    /// The whole target, for a frame that must repaint everything it returns.
    pub(crate) const fn full(&self) -> Rect {
        Rect::from_size(self.format.size())
    }

    /// The damage the next frame reports.
    ///
    /// A first frame, a frame of the lock scene and a frame after a reveal all
    /// repaint the whole region: none of them can assume the client's buffer still
    /// holds what it held for the previous frame. Every other frame reports exactly
    /// what was damaged since it, clipped to the region being copied.
    pub(crate) fn take_damage(&mut self, region: Rect, whole: bool) -> Vec<Rect> {
        if whole || self.frames == 0 {
            self.damage.clear();
            return vec![region];
        }
        let damage = std::mem::take(&mut self.damage);
        damage
            .into_iter()
            .filter_map(|rect| rect.intersection(region))
            .collect()
    }

    /// Puts damage back after a frame that produced nothing.
    pub(crate) fn restore_damage(&mut self, damage: Vec<Rect>) {
        for rect in damage {
            if !self.damage.contains(&rect) {
                self.damage.push(rect);
            }
        }
    }

    /// The session's identity.
    #[must_use]
    pub const fn id(&self) -> SessionId {
        self.id
    }

    /// The connection that opened it.
    #[must_use]
    pub const fn owner(&self) -> ClientId {
        self.owner
    }

    /// What it captures.
    #[must_use]
    pub const fn target(&self) -> Target {
        self.target
    }

    /// The consent it holds.
    #[must_use]
    pub const fn stream(&self) -> CaptureStream {
        self.stream
    }

    /// The buffer shape its client negotiated.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// How many frames it has copied.
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// Why it may produce no further frames, if it may not.
    #[must_use]
    pub const fn stopped(&self) -> Option<Stop> {
        self.stop
    }

    /// Whether the session may still be asked for a frame.
    pub(crate) const fn live(&self) -> bool {
        self.stop.is_none()
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_lock::CaptureStream;
    use gnoblin_types::{ClientId, OutputId, Rect, Size};

    use crate::format::{Format, PixelFormat};
    use crate::session::{Session, SessionId, Stop, Target};

    fn session() -> Session {
        let format = Format::new(Size::new(8, 4), 32, PixelFormat::Argb8888).expect("a valid format");
        Session::new(
            SessionId::new(1),
            ClientId::new(7),
            Target::Output(OutputId::new(1)),
            CaptureStream::Requested,
            format,
        )
    }

    /// Failure mode: a stopped session is resumed by a later event. A stop is final,
    /// and the first reason is the one recorded.
    #[test]
    fn a_stop_is_final_and_keeps_the_first_reason() {
        let mut session = session();
        session.end(Stop::Embargoed);
        session.end(Stop::TargetGone);
        assert_eq!(session.stop, Some(Stop::Embargoed));
        assert!(!session.live());
    }

    /// Failure mode: damage owed is reported twice, so a client repaints the same
    /// rectangle for ever, or is dropped, so a change is never reported.
    #[test]
    fn damage_is_reported_once() {
        let mut session = session();
        session.damage.push(Rect::new(1, 1, 2, 2));
        let region = session.full();
        // A first frame reports the whole region, whatever was noted for it.
        assert_eq!(session.take_damage(region, false), vec![region]);
        session.frames = 1;
        session.damage.push(Rect::new(1, 1, 2, 2));
        assert_eq!(session.take_damage(region, false), vec![Rect::new(1, 1, 2, 2)]);
        assert!(session.damage.is_empty(), "the damage was taken");
    }

    /// Failure mode: damage outside the copied region is reported, and the client
    /// clips a rectangle it should never have seen.
    #[test]
    fn damage_outside_the_region_is_clipped_away() {
        let mut session = session();
        session.frames = 1;
        session.damage.push(Rect::new(100, 100, 4, 4));
        session.damage.push(Rect::new(2, 0, 4, 4));
        let region = Rect::new(0, 0, 8, 4);
        assert_eq!(session.take_damage(region, false), vec![Rect::new(2, 0, 4, 4)]);
    }
}
