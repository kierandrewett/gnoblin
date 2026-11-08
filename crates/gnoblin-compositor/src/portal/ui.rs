//! The built-in portal UI session: one dialog on screen at a time, a FIFO of
//! admitted requests behind it, and the outcome of each request once it settles
//! (D19 "UI step", "Consent in the fallback").
//!
//! [`PortalUi`] is the only place that owns a [`PortalScreen`]. It is pure: the
//! caller passes the time in ([`Tick`]), feeds keys and pointer presses in, and
//! collects what happened:
//!
//! - [`PortalUi::submit`] admits a request. It starts at once when no dialog is
//!   open, and waits in the FIFO otherwise. Admission uses the same [`Queue`]
//!   caps as the rest of the negotiation, so a full queue refuses with `busy`.
//! - [`PortalUi::key`], [`PortalUi::pointer_press`] and [`PortalUi::pointer_release`]
//!   drive the open dialog. A settled dialog ends its request and the next one
//!   starts.
//! - [`PortalUi::tick`] advances the clock. It moves the dialog's animations and
//!   enforces the request's deadline (`cancelled { timeout }`).
//! - [`PortalUi::take_frame`] returns the rasterised dialog, and only when its
//!   pixels changed since the last frame it returned.
//! - [`PortalUi::take_outcomes`] returns every request that reached its one
//!   terminal verdict since the last call, in settle order.
//! - [`PortalUi::cancel`] is the requester going away. [`PortalUi::set_locked`]
//!   is the session lock. Both are terminal for the request they touch.
//!
//! Each request runs through a [`Negotiation`] with [`PortalProvider::Builtin`], so the
//! first terminal verdict wins and every later event is ignored.
//!
//! # Dialog mapping
//!
//! | Interface                                  | Dialog                    |
//! | ------------------------------------------ | ------------------------- |
//! | ScreenCast, Screenshot, RemoteDesktop, InputCapture | consent screen   |
//! | FileChooser                                | `Dialog::Files`           |
//! | AppChooser                                 | `Dialog::App`             |
//! | any other interface                        | refused: `unsupported-ui` |
//!
//! A dialog whose kind does not match its interface is refused the same way.
//!
//! # Verdicts
//!
//! - Consent `Allow`, or a chooser's `Granted`: `Handled(grant)`.
//! - Consent `Deny`, Escape, or a chooser's cancel: `Cancelled(User)`.
//! - Deadline reached while the dialog is open: `Cancelled(Timeout)`.
//! - Requester gone ([`PortalUi::cancel`]): `Cancelled(RequesterGone)`.
//! - Session locked while open or queued: `Error(Locked)`.
//! - Refused at admission: no outcome; [`PortalUi::submit`] returns the error.
//!
//! # Failure modes
//!
//! - Unsupported interface or dialog mismatch: `submit` returns `unsupported-ui`
//!   and nothing is queued.
//! - Queue full (total, per app, or per-app rate): `submit` returns `busy`.
//! - Locked: `submit` returns `locked`; open and queued requests settle `locked`.
//! - Requester leaves mid-dialog: the dialog closes, the outcome is
//!   `cancelled(requester-gone)`. The caller drops the late reply.
//! - A chooser asked for a folder listing that never arrives: the request ends at
//!   its deadline (`cancelled(timeout)`).
//!
//! # Frame and input API for the scene glue
//!
//! The compositor's scene glue owns the output. It calls [`PortalUi::tick`]
//! once per frame, forwards keys with [`PortalUi::key`] and pointer presses with
//! [`PortalUi::pointer_press`] / [`PortalUi::pointer_release`] (coordinates in
//! frame pixels of [`PortalUi::size`]), and draws whatever [`PortalUi::take_frame`]
//! returns. After each call it hands [`PortalUi::take_outcomes`] to the IPC reply
//! path. Nothing here reads a clock, touches a socket or draws to a screen.

use std::collections::VecDeque;

use gnoblin_portal::PortalScreen;
use gnoblin_portal_core::model::{Dialog, Interface, ScreenKey, Settled};
use gnoblin_portal_ui::paint::Frame;
use gnoblin_portal_ui::ui::{DialogScheme, Setup};
use gnoblin_types::{RequestId, Tick};

use super::negotiate::{Effect, Event, Negotiation, PortalProvider};
use super::outcome::{CancelReason, PortalError, Verdict};
use super::queue::{Queue, Refusal};

/// Whether the built-in UI serves `interface` with the dialog `dialog`.
fn serves(interface: Interface, dialog: &Dialog) -> bool {
    matches!(
        (interface, dialog),
        (
            Interface::ScreenCast
                | Interface::Screenshot
                | Interface::RemoteDesktop
                | Interface::InputCapture
                | Interface::Secret
                | Interface::Access,
            Dialog::Consent(_),
        ) | (Interface::FileChooser, Dialog::Files(_))
            | (Interface::AppChooser, Dialog::App(_))
    )
}

/// Whether a request was started now or waits behind the open dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// The dialog is open now.
    Started,
    /// Admitted; it opens when the requests ahead of it settle.
    Queued,
}

/// A request that has been admitted and is waiting for its turn.
struct PortalPending {
    request: RequestId,
    interface: Interface,
    dialog: Dialog,
    timeout_ms: u64,
}

/// The request on screen.
struct Active {
    request: RequestId,
    negotiation: Negotiation,
    screen: PortalScreen,
    /// Set by any input or clock change; cleared by [`PortalUi::take_frame`].
    dirty: bool,
    /// The pixels last returned by [`PortalUi::take_frame`].
    shown: Option<Vec<u8>>,
}

/// The built-in portal UI session. See the module docs.
pub struct PortalUi {
    queue: Queue,
    locked: bool,
    setup: Setup,
    now: Tick,
    active: Option<Active>,
    pending: VecDeque<PortalPending>,
    outcomes: Vec<(RequestId, Verdict)>,
}

impl PortalUi {
    /// A session that draws its dialogs at `width` by `height` pixels, dark. Reduced
    /// motion is off.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            queue: Queue::new(),
            locked: false,
            setup: Setup {
                width,
                height,
                reduced_motion: false,
                scheme: DialogScheme::Dark,
            },
            now: Tick::default(),
            active: None,
            pending: VecDeque::new(),
            outcomes: Vec::new(),
        }
    }

    /// The frame size in pixels: the coordinate space for pointer input. It is the open
    /// dialog's size, which fits its content, or the bounds when nothing is open.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        match self.active.as_ref() {
            Some(active) => active.screen.size(),
            None => (self.setup.width, self.setup.height),
        }
    }

    /// Sets the visuals later dialogs are drawn with. An open dialog keeps the scheme
    /// it opened with, so a configuration change never restyles a dialog mid-request.
    pub fn set_scheme(&mut self, scheme: DialogScheme) {
        self.setup.scheme = scheme;
    }

    /// Sets the largest frame a dialog may take, in pixels. The owner passes the size
    /// of the output the dialog sits on, so a dialog never outgrows its output. An open
    /// dialog is redrawn at its new bounds on the next `take_frame`.
    pub fn set_bounds(&mut self, width: usize, height: usize) {
        if self.setup.width == width && self.setup.height == height {
            return;
        }
        self.setup.width = width;
        self.setup.height = height;
        if let Some(active) = self.active.as_mut() {
            active.screen.set_bounds(width, height);
            active.dirty = true;
        }
    }

    /// The request whose dialog is open, if any.
    #[must_use]
    pub fn active_request(&self) -> Option<RequestId> {
        self.active.as_ref().map(|active| active.request)
    }

    /// Admitted requests waiting behind the open dialog.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.pending.len()
    }

    /// Admits a request for `interface` on behalf of `app_id`, with `dialog`
    /// already built from its prompt and options. `timeout_ms` is how long the
    /// dialog may stay open, counted from when it opens.
    ///
    /// # Errors
    ///
    /// [`PortalError::UnsupportedUi`] when the interface has no built-in dialog or
    /// `dialog` is the wrong kind for it; [`PortalError::Locked`] while the session
    /// is locked; [`PortalError::Busy`] when a queue cap is reached.
    pub fn submit(
        &mut self,
        request: RequestId,
        app_id: &str,
        interface: Interface,
        dialog: Dialog,
        timeout_ms: u64,
        now: Tick,
    ) -> Result<SubmitOutcome, PortalError> {
        if !serves(interface, &dialog) {
            return Err(PortalError::UnsupportedUi);
        }
        self.now = now;
        self.queue
            .admit(request, app_id, now, self.locked)
            .map_err(Refusal::error)?;
        let pending = PortalPending {
            request,
            interface,
            dialog,
            timeout_ms,
        };
        if self.active.is_some() {
            self.pending.push_back(pending);
            return Ok(SubmitOutcome::Queued);
        }
        self.open(pending);
        Ok(SubmitOutcome::Started)
    }

    /// Feeds one key press or repeat to the open dialog.
    pub fn key(&mut self, key: ScreenKey, repeat: bool) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.dirty = true;
        if let Some(settled) = active.screen.key(key, repeat) {
            self.settle_with(&settled);
        }
    }

    /// Feeds a pointer press at frame pixel `(x, y)` to the open dialog.
    pub fn pointer_press(&mut self, x: f32, y: f32) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.dirty = true;
        if let Some(settled) = active.screen.pointer_press(x, y) {
            self.settle_with(&settled);
        }
    }

    /// Feeds a pointer release at frame pixel `(x, y)` to the open dialog.
    pub fn pointer_release(&mut self, x: f32, y: f32) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.dirty = true;
        if let Some(settled) = active.screen.pointer_release(x, y) {
            self.settle_with(&settled);
        }
    }

    /// Moves the clock. Animations advance and the open request's deadline is
    /// checked; a request past its deadline settles `cancelled(timeout)`.
    pub fn tick(&mut self, now: Tick) {
        self.now = now;
        let Some(active) = self.active.as_mut() else {
            return;
        };
        active.screen.tick(now);
        active.dirty = true;
        let effects = active.negotiation.handle(Event::Tick, now);
        self.apply(effects);
    }

    /// The open dialog rasterised, but only when its pixels changed since the
    /// last frame returned. `None` when nothing is open or nothing changed.
    pub fn take_frame(&mut self) -> Option<Frame> {
        let active = self.active.as_mut()?;
        if !active.dirty {
            return None;
        }
        active.dirty = false;
        let frame = active.screen.render();
        if active.shown.as_deref() == Some(frame.bytes()) {
            return None;
        }
        active.shown = Some(frame.bytes().to_vec());
        Some(frame)
    }

    /// Every request that settled since the last call, in settle order.
    pub fn take_outcomes(&mut self) -> Vec<(RequestId, Verdict)> {
        std::mem::take(&mut self.outcomes)
    }

    /// The requester for `request` has gone. A dialog open for it closes and a
    /// queued one is dropped; either settles `cancelled(requester-gone)`. Returns
    /// `false` when the request is not admitted.
    pub fn cancel(&mut self, request: RequestId) -> bool {
        if self.active_request() == Some(request) {
            let effects = self
                .active
                .as_mut()
                .map(|active| active.negotiation.handle(Event::RequesterGone, self.now))
                .unwrap_or_default();
            self.apply(effects);
            return true;
        }
        let Some(index) = self.pending.iter().position(|pending| pending.request == request) else {
            return false;
        };
        self.pending.remove(index);
        self.queue.complete(request);
        self.outcomes
            .push((request, Verdict::Cancelled(CancelReason::RequesterGone)));
        true
    }

    /// The session lock changes. Locking settles every admitted request with
    /// `error(locked)` and refuses new ones until it is cleared. Unlocking only
    /// lifts the refusal; it does not revive settled requests.
    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
        if !locked {
            return;
        }
        let dropped = self.queue.lock();
        let mut open = false;
        for request in dropped {
            if self.active_request() == Some(request) {
                open = true;
            } else if let Some(index) = self.pending.iter().position(|pending| pending.request == request) {
                self.pending.remove(index);
                self.outcomes.push((request, Verdict::Error(PortalError::Locked)));
            }
        }
        if open {
            let effects = self
                .active
                .as_mut()
                .map(|active| active.negotiation.handle(Event::Locked, self.now))
                .unwrap_or_default();
            self.apply(effects);
        }
    }

    /// Starts `pending` now: its negotiation opens the built-in UI and the dialog
    /// is built at `self.now`.
    fn open(&mut self, pending: PortalPending) {
        // The built-in provider's only effect is `OpenUi`, which the dialog
        // opened here already is, so the effects are not needed.
        let (negotiation, _) = Negotiation::start(
            pending.request,
            pending.interface,
            PortalProvider::Builtin,
            self.now,
            0,
            pending.timeout_ms,
        );
        let screen = PortalScreen::new(pending.dialog, self.setup, self.now);
        self.active = Some(Active {
            request: pending.request,
            negotiation,
            screen,
            dirty: true,
            shown: None,
        });
    }

    /// Ends the open request with the verdict its dialog reached.
    fn settle_with(&mut self, settled: &Settled) {
        let verdict = match settled {
            Settled::Granted(grant) => Verdict::Handled(grant.clone()),
            Settled::Denied | Settled::Cancelled => Verdict::Cancelled(CancelReason::User),
        };
        let effects = self
            .active
            .as_mut()
            .map(|active| active.negotiation.handle(Event::UiVerdict(verdict), self.now))
            .unwrap_or_default();
        self.apply(effects);
    }

    /// Carries out the effects of a negotiation step. A `Finish` ends the open
    /// request and starts the next one in line.
    fn apply(&mut self, effects: Vec<Effect>) {
        let finished = effects.into_iter().find_map(|effect| match effect {
            Effect::Finish(verdict) => Some(verdict),
            _ => None,
        });
        let Some(verdict) = finished else {
            return;
        };
        if let Some(active) = self.active.take() {
            self.queue.complete(active.request);
            self.outcomes.push((active.request, verdict));
        }
        if !self.locked
            && let Some(next) = self.pending.pop_front()
        {
            self.open(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_portal_core::model::{Consent, FileChooser, FileChooserMode};

    const UI_TIMEOUT_MS: u64 = 60_000;

    fn at(raw: u64) -> Tick {
        Tick::new(raw)
    }

    fn id(raw: u64) -> RequestId {
        RequestId::new(raw)
    }

    fn consent(now: Tick) -> Dialog {
        Dialog::Consent(Consent::open("org.example.App", "Share your screen?", now))
    }

    fn files() -> Dialog {
        Dialog::Files(FileChooser::new(
            FileChooserMode::Open,
            false,
            "/home/user",
            Vec::new(),
            "",
        ))
    }

    /// Submits a consent request for `app` at `now`; the caller checks whether it started or queued.
    fn submit_consent(ui: &mut PortalUi, request: u64, app: &str, now: Tick) -> SubmitOutcome {
        ui.submit(
            id(request),
            app,
            Interface::ScreenCast,
            consent(now),
            UI_TIMEOUT_MS,
            now,
        )
        .expect("admitted")
    }

    /// Submits a consent request that must start at once.
    fn start_consent(ui: &mut PortalUi, request: u64, app: &str, now: Tick) {
        assert_eq!(submit_consent(ui, request, app, now), SubmitOutcome::Started);
    }

    /// Moves focus from Deny to Allow and presses Enter once the dialog is armed.
    fn allow(ui: &mut PortalUi, now: Tick) {
        ui.tick(now);
        ui.key(ScreenKey::Tab, false);
        ui.key(ScreenKey::Enter, false);
    }

    fn outcome_of(ui: &mut PortalUi, request: u64) -> Verdict {
        let mut outcomes = ui.take_outcomes();
        let index = outcomes
            .iter()
            .position(|(done, _)| *done == id(request))
            .unwrap_or_else(|| panic!("request {request} did not settle: {outcomes:?}"));
        outcomes.swap_remove(index).1
    }

    #[test]
    fn consent_allow_after_arm_delay_is_handled() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        allow(&mut ui, at(600));
        assert!(matches!(outcome_of(&mut ui, 1), Verdict::Handled(_)));
        assert_eq!(ui.active_request(), None);
    }

    #[test]
    fn enter_on_default_focus_denies() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        ui.tick(at(600));
        ui.key(ScreenKey::Enter, false);
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::User));
    }

    #[test]
    fn escape_cancels_as_user() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        ui.key(ScreenKey::Escape, false);
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::User));
    }

    #[test]
    fn allow_before_arm_delay_does_not_grant() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        allow(&mut ui, at(100));
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::User));
    }

    #[test]
    fn deadline_cancels_with_timeout() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        ui.tick(at(UI_TIMEOUT_MS - 1));
        assert!(ui.take_outcomes().is_empty());
        ui.tick(at(UI_TIMEOUT_MS));
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::Timeout));
    }

    #[test]
    fn second_request_queues_and_starts_when_first_settles() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        let queued = ui
            .submit(
                id(2),
                "org.other.App",
                Interface::ScreenCast,
                consent(at(0)),
                UI_TIMEOUT_MS,
                at(0),
            )
            .expect("admitted");
        assert_eq!(queued, SubmitOutcome::Queued);
        assert_eq!(ui.waiting(), 1);

        ui.key(ScreenKey::Escape, false);
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::User));
        assert_eq!(ui.active_request(), Some(id(2)));
        assert_eq!(ui.waiting(), 0);

        ui.key(ScreenKey::Escape, false);
        assert_eq!(outcome_of(&mut ui, 2), Verdict::Cancelled(CancelReason::User));
    }

    #[test]
    fn unsupported_interface_is_refused_without_queueing() {
        let mut ui = PortalUi::new(640, 480);
        let refused = ui.submit(id(1), "app", Interface::Wallpaper, consent(at(0)), UI_TIMEOUT_MS, at(0));
        assert_eq!(refused, Err(PortalError::UnsupportedUi));
        assert_eq!(ui.active_request(), None);
        assert_eq!(ui.waiting(), 0);
    }

    #[test]
    fn dialog_kind_must_match_interface() {
        let mut ui = PortalUi::new(640, 480);
        let mismatch = ui.submit(id(1), "app", Interface::ScreenCast, files(), UI_TIMEOUT_MS, at(0));
        assert_eq!(mismatch, Err(PortalError::UnsupportedUi));
    }

    #[test]
    fn busy_when_one_app_has_too_many_requests() {
        let mut ui = PortalUi::new(640, 480);
        for request in 1..=2 {
            ui.submit(
                id(request),
                "org.example.App",
                Interface::ScreenCast,
                consent(at(0)),
                UI_TIMEOUT_MS,
                at(0),
            )
            .expect("within the per-app cap");
        }
        let refused = ui.submit(
            id(3),
            "org.example.App",
            Interface::ScreenCast,
            consent(at(0)),
            UI_TIMEOUT_MS,
            at(0),
        );
        assert_eq!(refused, Err(PortalError::Busy));
    }

    #[test]
    fn requester_gone_closes_open_dialog_and_serves_next() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        assert_eq!(
            submit_consent(&mut ui, 2, "org.other.App", at(0)),
            SubmitOutcome::Queued
        );
        assert!(ui.cancel(id(1)));
        assert_eq!(outcome_of(&mut ui, 1), Verdict::Cancelled(CancelReason::RequesterGone));
        assert_eq!(ui.active_request(), Some(id(2)));
    }

    #[test]
    fn requester_gone_drops_queued_request_and_frees_its_slot() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        assert_eq!(
            submit_consent(&mut ui, 2, "org.example.App", at(0)),
            SubmitOutcome::Queued
        );
        assert!(ui.cancel(id(2)));
        assert_eq!(outcome_of(&mut ui, 2), Verdict::Cancelled(CancelReason::RequesterGone));
        assert_eq!(ui.waiting(), 0);
        assert!(!ui.cancel(id(2)), "a settled request is not cancelled twice");
    }

    #[test]
    fn lock_settles_open_and_queued_requests_and_refuses_new_ones() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        assert_eq!(
            submit_consent(&mut ui, 2, "org.other.App", at(0)),
            SubmitOutcome::Queued
        );
        ui.set_locked(true);
        let outcomes = ui.take_outcomes();
        assert!(outcomes.contains(&(id(1), Verdict::Error(PortalError::Locked))));
        assert!(outcomes.contains(&(id(2), Verdict::Error(PortalError::Locked))));
        assert_eq!(ui.active_request(), None);

        let refused = ui.submit(
            id(3),
            "org.example.App",
            Interface::ScreenCast,
            consent(at(1)),
            UI_TIMEOUT_MS,
            at(1),
        );
        assert_eq!(refused, Err(PortalError::Locked));

        ui.set_locked(false);
        start_consent(&mut ui, 4, "org.example.App", at(2));
        assert_eq!(ui.active_request(), Some(id(4)));
    }

    #[test]
    fn take_frame_returns_only_changed_pixels() {
        let mut ui = PortalUi::new(640, 480);
        assert!(ui.take_frame().is_none(), "nothing open, nothing to draw");
        start_consent(&mut ui, 1, "org.example.App", at(0));
        let first = ui.take_frame().expect("a new dialog draws");
        assert_eq!(
            (first.width(), first.height()),
            ui.size(),
            "the frame is the dialog's fitted size"
        );
        assert!(ui.take_frame().is_none(), "no input, no new frame");

        ui.tick(at(600));
        ui.take_frame();
        ui.key(ScreenKey::Tab, false);
        assert!(ui.take_frame().is_some(), "moving focus changes pixels");
    }

    #[test]
    fn a_dialog_fits_its_content_and_follows_the_output_bounds() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        ui.take_frame();
        let (width, height) = ui.size();
        assert!(
            (320..=640).contains(&width),
            "the width is at least the floor and at most the bounds"
        );
        assert!(height <= 480, "the height is at most the bounds");

        // An output smaller than the dialog pulls it in: the dialog is redrawn at once
        // within the new bounds, so it never outgrows the output it sits on.
        ui.set_bounds(200, 120);
        let frame = ui.take_frame().expect("new bounds redraw the open dialog");
        assert!(
            frame.width() <= 200 && frame.height() <= 120,
            "the frame fits the output"
        );
        assert_eq!((frame.width(), frame.height()), ui.size());
    }

    #[test]
    fn pointer_press_and_release_drive_the_dialog() {
        let mut ui = PortalUi::new(640, 480);
        start_consent(&mut ui, 1, "org.example.App", at(0));
        ui.tick(at(600));
        // The dialog is sized to its content, so measure it before the pointer acts.
        let size = ui.size();
        // A press and release outside any control does not settle the dialog, and
        // leaves its size alone.
        ui.pointer_press(-1.0, -1.0);
        ui.pointer_release(-1.0, -1.0);
        assert!(ui.take_outcomes().is_empty());
        assert_eq!(ui.active_request(), Some(id(1)));
        assert_eq!(ui.size(), size);
    }
}
