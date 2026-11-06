//! The contract between the compositor and whatever draws the window menu.
//!
//! # Shape
//!
//! D29 names two provider modes: model mode, where the provider receives the
//! model and returns a choice, and surface mode, where it draws on its own
//! layer. Both must receive the same request and return the same answer, so
//! this crate defines one trait for both. The trait is event-driven, not a
//! single blocking call. A command provider, or a shell drawing its own
//! surface, cannot answer synchronously: the menu must be open for the user
//! while the answer is still on its way, and the answer can come after the
//! user has already dismissed it. So the compositor calls [`MenuProvider::open`]
//! once and then polls [`MenuProvider::poll`] until it sees a result.
//!
//! This crate performs no I/O. The compositor owns the clock, the sockets and
//! the layer, and calls these functions. The trait is the seam that lets the
//! built-in egui provider and an external one sit behind the same contract.

use std::time::Duration;

use gnoblin_types::Rect;

use crate::item::ItemId;
use crate::menu::Menu;

/// How long a provider has to answer before the built-in serves the request.
///
/// D29 proposes 2 seconds. It applies before the menu opens. After opening, the
/// provider is not timed: a surface provider holds the menu until the user
/// decides, and only a disconnect closes it.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Why a menu closed without a choice, or after a choice was refused.
///
/// The compositor logs the reason. None of these runs an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissReason {
    /// Escape closed the menu at its root.
    Escape,
    /// The user clicked outside the menu. The click is consumed.
    ClickOutside,
    /// Focus moved away from the menu.
    FocusLost,
    /// The window the menu was for closed or unmapped.
    TargetClosed,
    /// The output the menu was on was removed.
    OutputRemoved,
    /// The session locked.
    SessionLocked,
    /// The provider did not answer within the reply timeout after the menu was shown.
    Timeout,
    /// The provider disconnected or crashed after the menu was shown.
    ProviderLost,
}

/// Everything a provider needs to show one menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    /// The menu to show. Built by [`crate::build_menu`] for the target window.
    pub menu: Menu,
    /// The rectangle the menu opens from: a button, or a point as a zero-size rectangle.
    pub anchor: Rect,
    /// The output's work area, outside exclusive zones. MenuPlacement is clamped into it.
    pub work_area: Rect,
}

/// Why a provider could not open a menu. Each one falls back to the built-in provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The configured provider is missing, or its claim has gone.
    Missing,
    /// The provider command failed to start or exited before replying.
    StartFailed,
    /// No reply arrived within [`REPLY_TIMEOUT`].
    NoReply,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "the configured window menu provider is missing",
            Self::StartFailed => "the window menu provider failed to start",
            Self::NoReply => "the window menu provider did not reply in time",
        })
    }
}

impl std::error::Error for OpenError {}

/// What a provider reports once a menu is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderEvent {
    /// The user chose an item. The compositor still validates it with [`crate::validate_choice`].
    Chose(ItemId),
    /// The provider declined to choose. The menu is closed.
    Cancelled,
    /// The menu closed for a reason the compositor observed.
    Dismissed(DismissReason),
}

/// A menu provider, built-in or external.
///
/// Calls come from one compositor thread, in this order: `open`, then `poll`
/// until it returns an event, then `close` if the compositor closes the menu
/// first. A provider never moves, resizes or closes a window itself. It
/// reports the user's choice, and the compositor runs it.
pub trait MenuProvider {
    /// Starts showing `request`.
    ///
    /// Returns `Ok` once the provider has accepted the menu. An error means the
    /// menu was not shown, so the compositor serves it from the built-in.
    ///
    /// # Errors
    ///
    /// Returns an [`OpenError`] when the provider cannot show the menu.
    fn open(&mut self, request: &OpenRequest) -> Result<(), OpenError>;

    /// Returns the next event, or `None` when there is nothing new yet.
    ///
    /// Must not block. After the menu has been shown, a provider that has
    /// disconnected reports `Dismissed(DismissReason::ProviderLost)`.
    fn poll(&mut self) -> Option<ProviderEvent>;

    /// Closes the menu for `reason`. Called when the compositor closes it first.
    fn close(&mut self, reason: DismissReason);
}

/// Which provider serves a request, decided once at open time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSource {
    /// The configured provider opened the menu.
    Configured,
    /// The built-in provider opened the menu, because of the given error.
    Builtin(OpenError),
}

/// Decides which provider serves a request from the result of its `open` call.
///
/// The built-in serves the request only when the configured provider could not
/// open it. Once a menu is open, the fallback never applies: a disconnect closes
/// the menu, and the built-in does not take over.
#[must_use]
pub fn source_for(open: Result<(), OpenError>) -> ProviderSource {
    match open {
        Ok(()) => ProviderSource::Configured,
        Err(error) => ProviderSource::Builtin(error),
    }
}

/// True when a provider has taken at least [`REPLY_TIMEOUT`] to answer, as measured by the caller.
///
/// Takes the elapsed time rather than reading a clock, so the rule stays pure.
#[must_use]
pub fn reply_overdue(waited: Duration) -> bool {
    waited >= REPLY_TIMEOUT
}
