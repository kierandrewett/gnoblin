//! What a provider or the built-in UI returns, and what the caller finally gets.
//!
//! [`Verdict`] is the internal result of one portal request. [`Verdict::Declined`] is
//! never sent to the caller: a provider decline starts the built-in fallback instead.
//! The wire names come from `as_str` and must match `portal-call-reply`.

use gnoblin_portal_core::model::Grant;

/// The result a provider or the built-in UI returns for one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The request was granted; the grant is what the user chose.
    Handled(Grant),
    /// The user or the provider stopped the request.
    Cancelled(CancelReason),
    /// The provider confirms it did not act. Only a provider can say this; it
    /// triggers the built-in fallback and never reaches the caller.
    Declined,
    /// The request failed.
    Error(PortalError),
}

/// Why a request was cancelled (the `cancelled { reason }` of `portal-call-reply`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    User,
    Timeout,
    UiFailed,
    RequesterGone,
}

impl CancelReason {
    /// The wire name used in `portal-call-reply`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Timeout => "timeout",
            Self::UiFailed => "ui-failed",
            Self::RequesterGone => "requester-gone",
        }
    }
}

/// The `error { code }` of `portal-call-reply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalError {
    NoUi,
    Locked,
    Busy,
    ProviderFailed,
    NotSupported,
    UnknownApp,
    UnsupportedUi,
}

impl PortalError {
    /// The wire name used in `portal-call-reply`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoUi => "no-ui",
            Self::Locked => "locked",
            Self::Busy => "busy",
            Self::ProviderFailed => "provider-failed",
            Self::NotSupported => "not-supported",
            Self::UnknownApp => "unknown-app",
            Self::UnsupportedUi => "unsupported-ui",
        }
    }
}
