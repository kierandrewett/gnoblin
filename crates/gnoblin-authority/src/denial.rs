//! Why something was refused, and the two-valued answer a gate gives.

/// Why authority was withheld.
///
/// One enum for activation, input contexts and grants: a caller logs the reason
/// with the decision, and a test asserts the exact one, so "refused" never hides
/// which rule fired.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Denial {
    /// No token and no trusted context accompanied the request. An app id, a
    /// bare process id or a session leader alone lands here.
    NoAuthority,
    /// The token was never issued in this session.
    UnknownToken,
    /// The token was already redeemed. A token is accepted at most once.
    ReusedToken,
    /// The token expired before it was redeemed.
    ExpiredToken,
    /// The token belongs to another launch, so a process from a different
    /// session leader cannot redeem it.
    ForeignLaunch,
    /// The compositor presented a token value it already holds.
    DuplicateToken,
    /// The session is covered; nothing may take focus and no gesture may
    /// authorise anything.
    SessionLocked,
    /// The gesture that created the context is over.
    StaleContext,
    /// The context expired before it was used.
    ExpiredContext,
    /// The context was already used. Every context is single-use.
    ConsumedContext,
    /// The context belongs to another client or another surface.
    WrongTarget,
    /// The table for that kind of value is full.
    AtCapacity,
    /// No client claims `portal.ui`, so consent cannot be asked (D14).
    NoPortalUi,
    /// Only the current `portal.ui` provider may answer a consent prompt.
    NotProvider,
    /// Consent exists but is pending: nobody has agreed yet.
    NoConsent,
    /// The user refused.
    Refused,
    /// The prompt timed out before an answer arrived, which is a refusal.
    ConsentExpired,
    /// No grant exists for this requester and capability, or it expired.
    NoGrant,
    /// The session is locked, so capture and input injection are refused.
    Locked,
}

impl Denial {
    /// A stable lower-case name for logs, events and test assertions.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoAuthority => "no-authority",
            Self::UnknownToken => "unknown-token",
            Self::ReusedToken => "reused-token",
            Self::ExpiredToken => "expired-token",
            Self::ForeignLaunch => "foreign-launch",
            Self::DuplicateToken => "duplicate-token",
            Self::SessionLocked => "session-locked",
            Self::StaleContext => "stale-context",
            Self::ExpiredContext => "expired-context",
            Self::ConsumedContext => "consumed-context",
            Self::WrongTarget => "wrong-target",
            Self::AtCapacity => "at-capacity",
            Self::NoPortalUi => "no-portal-ui",
            Self::NotProvider => "not-provider",
            Self::NoConsent => "no-consent",
            Self::Refused => "refused",
            Self::ConsentExpired => "consent-expired",
            Self::NoGrant => "no-grant",
            Self::Locked => "locked",
        }
    }
}

/// The answer to "may this happen".
///
/// Carries the reason rather than a bare `false`, because a refusal that cannot
/// say why is indistinguishable from a bug to whoever reads the log.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    /// Yes.
    Allowed,
    /// No, for this reason.
    Denied(Denial),
}

impl Decision {
    /// Whether the request is allowed.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}
