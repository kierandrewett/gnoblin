//! What a provider or the built-in UI returns, and what the caller finally gets.

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
        }
    }
}

/// What the user granted. A reply can grant no more than the request asked
/// (checked by [`crate::validate_reply`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    /// Consent given (Screenshot, Wallpaper, Background, Secret first use, ...).
    Allowed,
    /// FileChooser: the chosen paths and the filter index in effect.
    Files { paths: Vec<String>, filter: usize },
    /// AppChooser: the offered desktop file id, and whether to remember it.
    App { desktop_file_id: String, remember: bool },
    /// Print: copies and an inclusive page range.
    Print {
        copies: u32,
        first_page: u32,
        last_page: u32,
    },
    /// ScreenCast: the cursor mode and the source count granted.
    Capture { cursor_mode: u32, sources: u32 },
    /// RemoteDesktop: the device types granted (bitmask).
    Devices { device_types: u32 },
}
