//! The seam between the portal's negotiation and whatever draws its dialogs.
//!
//! The compositor queues and validates portal requests and settles every one of them with exactly one verdict. It draws
//! nothing itself: the screen the user answers is a [`UiProvider`]'s. The trait lives here, beside the frame, the style
//! and the scheme it speaks in, so the compositor (which drives it) and the library that implements the stock dialogs
//! (`gnoblin-std-impl`) name the one definition. With no provider the session runs `NoUi`, which refuses every request
//! `no-ui`, so a request the user cannot be asked about is never granted.

use gnoblin_portal_core::model::{Dialog, Interface, ScreenKey};
use gnoblin_portal_core::{PortalError, Verdict};
use gnoblin_types::{RequestId, Tick};

use crate::paint::Frame;
use crate::style::DialogStyle;
use crate::ui::DialogScheme;

/// Whether a request was started now or waits behind the open dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The dialog is open now.
    Started,
    /// Admitted; it opens when the requests ahead of it settle.
    Queued,
}

/// What shows portal dialogs to the user, one at a time, and reports how each request ended.
///
/// The provider is driven from the compositor loop with the time passed in and never reads a clock: the owner feeds keys
/// and pointer presses to the open dialog, calls [`UiProvider::tick`] once per frame, draws what
/// [`UiProvider::take_frame`] returns and hands [`UiProvider::take_outcomes`] to the reply path. Every admitted request
/// ends in exactly one outcome. The service that drives it lives on the dispatcher's thread, so a provider is `Send`.
pub trait UiProvider: Send {
    /// The frame size in pixels: the coordinate space for pointer input. It is the open dialog's size, which fits its
    /// content, or the bounds when nothing is open.
    fn size(&self) -> (usize, usize);

    /// Sets the visuals later dialogs are drawn with. An open dialog keeps the scheme it opened with, so a configuration
    /// change never restyles a dialog mid-request.
    fn set_scheme(&mut self, scheme: DialogScheme);

    /// Sets the look later dialogs are drawn with, already clamped. As with the scheme, an open dialog keeps the look it
    /// opened with.
    fn set_style(&mut self, style: DialogStyle);

    /// Sets the largest frame a dialog may take, in pixels. The owner passes the size of the output the dialog sits on,
    /// so a dialog never outgrows its output.
    fn set_bounds(&mut self, width: usize, height: usize);

    /// The request whose dialog is open, if any.
    fn active_request(&self) -> Option<RequestId>;

    /// Admitted requests waiting behind the open dialog.
    fn waiting(&self) -> usize;

    /// Admits a request for `interface` on behalf of `app_id`, with `dialog` already built from its prompt and options.
    /// `timeout_ms` is how long the dialog may stay open, counted from when it opens.
    ///
    /// # Errors
    ///
    /// [`PortalError::NoUi`] when there is no UI to ask the user with; [`PortalError::UnsupportedUi`] when the interface
    /// has no dialog or `dialog` is the wrong kind for it; [`PortalError::Locked`] while the session is locked;
    /// [`PortalError::Busy`] when a queue cap is reached.
    fn submit(
        &mut self,
        request: RequestId,
        app_id: &str,
        interface: Interface,
        dialog: Dialog,
        timeout_ms: u64,
        now: Tick,
    ) -> Result<SubmitOutcome, PortalError>;

    /// Feeds one key press or repeat to the open dialog.
    fn key(&mut self, key: ScreenKey, repeat: bool);

    /// Feeds a pointer press at frame pixel `(x, y)` to the open dialog.
    fn pointer_press(&mut self, x: f32, y: f32);

    /// Feeds a pointer release at frame pixel `(x, y)` to the open dialog.
    fn pointer_release(&mut self, x: f32, y: f32);

    /// Moves the clock. Animations advance and the open request's deadline is checked; a request past its deadline
    /// settles `cancelled(timeout)`.
    fn tick(&mut self, now: Tick);

    /// The open dialog rasterised, but only when its pixels changed since the last frame returned. `None` when nothing is
    /// open or nothing changed.
    fn take_frame(&mut self) -> Option<Frame>;

    /// Hands the open dialog out again by the next [`UiProvider::take_frame`], even when its pixels did not change. For a
    /// dialog that was covered by another modal and has to be put back on screen.
    fn redraw(&mut self);

    /// Every request that settled since the last call, in settle order.
    fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)>;

    /// The requester for `request` has gone. A dialog open for it closes and a queued one is dropped; either settles
    /// `cancelled(requester-gone)`. Returns `false` when the request is not admitted.
    fn cancel(&mut self, request: RequestId) -> bool;

    /// The session lock changes. Locking settles every admitted request with `error(locked)` and refuses new ones until
    /// it is cleared. Unlocking only lifts the refusal; it does not revive settled requests.
    fn set_locked(&mut self, locked: bool);

    /// The provider is going away (the library that registered it is leaving, or it failed). Settles every request it still
    /// holds and returns the verdicts the owner has not taken, so no request is left waiting on a dialog nobody draws. A
    /// provider that lives in the owner's own process holds nothing past its drop and keeps this default.
    fn close(&mut self) -> Vec<(RequestId, Verdict)> {
        Vec::new()
    }
}
