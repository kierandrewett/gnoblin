//! States, the embargo the lock holds, and the answers its gates give.

/// Where the session lock controller is.
///
/// The C controller calls these `UNLOCKED`, `COVERING`, `LOCKED`, `FAILSAFE`.
/// `Locking` is the plan's "locking-awaiting-cover" and `CoverFailed` is
/// `FAILSAFE` generalised: it covers a locker that crashed, a locker that
/// abandoned its request, a cover that could not be rendered, and the protocol
/// being withdrawn mid-lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// No cover, no embargo, ordinary routing.
    Unlocked,
    /// A cover exists and the compositor is waiting for a protected frame on
    /// every output before it may report `locked`.
    Locking,
    /// Every output of the current layout has presented a frame newer than the
    /// barrier, so the locker owns the lock.
    Locked,
    /// The owner authenticated and the cover is coming down. The session is
    /// still embargoed until [`crate::Event::RevealComplete`].
    Unlocking,
    /// The cover is up and nothing may be revealed, but no owner can proceed.
    /// A replacement locker may take over; it must present a fresh cover first.
    CoverFailed,
}

impl State {
    /// Whether anything is hidden. This is exactly `ScreenSaver.GetActive`:
    /// false only while unlocked, true for every covering state.
    #[must_use]
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Unlocked)
    }

    /// Whether the lock belongs to a live owner that may still reveal the
    /// session.
    #[must_use]
    pub const fn is_owned(self) -> bool {
        matches!(self, Self::Locking | Self::Locked | Self::Unlocking)
    }
}

/// What the lock takes away from covered subjects.
///
/// One value for all of it, so a consumer that missed an event can compare what
/// it holds against [`crate::Lock::embargo`] instead of tracking three booleans.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Embargo {
    /// Covered subjects are not returned from any capture path. A granted
    /// monitor stream shows the lock scene instead, once it is presented.
    pub capture: bool,
    /// Covered subjects cannot read or write the selection, and their drag
    /// sources and data-control offers never reach the desktop.
    pub clipboard: bool,
    /// Input injected from a remote session is refused.
    pub remote_input: bool,
}

impl Embargo {
    /// Nothing is embargoed: the unlocked session.
    pub const RELEASED: Self = Self {
        capture: false,
        clipboard: false,
        remote_input: false,
    };

    /// Everything is embargoed: any covering state, including `CoverFailed`.
    pub const HELD: Self = Self {
        capture: true,
        clipboard: true,
        remote_input: true,
    };

    /// Whether any part of the embargo is held.
    #[must_use]
    pub const fn holds(self) -> bool {
        self.capture || self.clipboard || self.remote_input
    }
}

/// Where the compositor sends an input event, or that it consumes it.
///
/// A discarded event is not merely unfocused: the C filter returns
/// `CLUTTER_EVENT_STOP` on a miss, so shortcuts, grabs and gestures never see it
/// either. [`Delivery::Discard`] means the same thing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// Deliver to the named subject.
    Deliver,
    /// Consume: no client, no binding, no grab and no gesture sees it.
    Discard,
}

/// Where an input event came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputSource {
    /// A device on this seat, delivered by the compositor's own input path:
    /// keyboard, pointer, touch, tablet, or an input method driven by them.
    Local,
    /// Injected by a remote or portal session over EIS.
    Remote,
}

/// Who is asking for captured pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStream {
    /// A request that does not hold a consent grant yet.
    Requested,
    /// A stream that was granted before the lock began.
    Granted,
}

/// What a capture path may return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capture {
    /// Normal session content.
    Desktop,
    /// The presented lock scene. Never the session content underneath it.
    LockScene,
    /// No frames at all.
    Refused,
}
