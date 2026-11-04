//! The identifiers authority is decided over.
//!
//! Every value here is supplied by the caller. The machine never reads a clock
//! (`Tick` comes from the compositor's monotonic source), never reads
//! `/proc`, and never invents an identifier, so a sequence of events replays to
//! the same decisions.

/// A point on the compositor's monotonic clock.
///
/// Authority measures every lifetime against ticks the caller supplies, never
/// against a clock of its own: the caller owns time, so tests drive expiry
/// exactly and a suspended machine cannot make a token look fresh.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    /// Wraps a monotonic reading.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw reading, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The tick `ticks` after this one, saturating so a lifetime can never wrap
    /// into the past.
    #[must_use]
    pub const fn after(self, ticks: u64) -> Self {
        Self(self.0.saturating_add(ticks))
    }
}

/// A Wayland client connection.
///
/// A connection, not a process: it is what the compositor can observe, route to
/// and close, and a forked child never shares one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClientId(u64);

impl ClientId {
    /// Wraps the compositor's stable client index.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw index, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A Wayland surface, as the compositor names it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SurfaceId(u64);

impl SurfaceId {
    /// Wraps the compositor's stable surface index.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw index, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The process group leader of a launched process, from `SO_PEERCRED`.
///
/// This is the kernel's answer to "which session started this connection", and
/// it is the strongest identity a compositor has for a wayland client. It is
/// never sufficient on its own: [`crate::Claimant::token`] must accompany it,
/// because every process of one desktop shell shares a leader and a leader is a
/// recycled process id.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionLeader(u32);

impl SessionLeader {
    /// Wraps the leader's process id.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw process id, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An application identifier or X11 `WM_CLASS`.
///
/// A label an application chooses for itself. It is useful for window rules and
/// for showing a human which program is asking, and it is never authority: two
/// processes can claim the same one, so nothing here grants anything on it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AppId(String);

impl AppId {
    /// Wraps a claimed label.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The claimed label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An activation token.
///
/// The compositor generates the value (128 random bits on `xdg_activation_v1`),
/// so the machine only needs an identity to key on and never derives one from a
/// counter that a client could guess.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TokenId(u128);

impl TokenId {
    /// Wraps a token the compositor minted.
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    /// The raw value, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

/// One input gesture, and the handle for the trusted context it creates.
///
/// The compositor allocates it while it dispatches the gesture, which is why a
/// context can never outlive the gesture that produced it: both are keyed by the
/// same value.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GestureId(u64);

impl GestureId {
    /// Wraps the compositor's gesture serial.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw serial, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A portal or D-Bus caller, named by the peer credentials the portal resolved.
///
/// The caller does not choose this value, which is what makes a grant belong to
/// the process that asked rather than to a string it presented.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequesterId(u64);

impl RequesterId {
    /// Wraps the portal's caller handle.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw handle, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One consent request, so an answer can be matched to what it answers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId(u64);

impl RequestId {
    /// Wraps the portal's request handle.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw handle, for logs and test assertions.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}
