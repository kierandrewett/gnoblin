//! The compositor's own lock card: a clock and a masked password entry over the lock cover, for a session whose shell
//! does not hold the lock screen.
//!
//! A stock shell draws the lock screen and takes the typed password. This is the built-in that serves when none does:
//! no shell attached, one that died while the session was locked, or one that never engaged the lock. Without it the
//! user sees a black cover with nothing to type into (design/js-research/popups-portals.md, row 9). The session decides
//! when the card is wanted and takes the keys for it (`gnoblin-wayland`, `lock_card.rs`); this module only draws it and
//! turns keys into a password. [`LockCardService::pump`] returns the [`LockCardScene`]: the overlay to show or clear and
//! the passwords to check.
//!
//! # What a script cannot do to it
//!
//! - It is not a tree node. A script reaches the compositor only through the tree, which has no way to set it, draw over
//!   it, move it or close it. It is the last layer of the frame, above the shell's own lock screen.
//! - While it is up every key goes to it and to no shell node, client or binding, so nothing but the compositor sees
//!   the password. [`LockUi`] keeps it in a
//!   wiped [`SecretText`], the key events are overwritten once routed, and the password is checked by the same PAM
//!   worker the shell's `authenticate` uses. Nothing here logs more than that the card appeared.
//! - There is no way to dismiss it: no Escape, no button. It goes when the session unlocks, or when a shell holds the
//!   lock screen.
//!
//! # Failure modes
//!
//! - The shell comes up while the card is up: the session stops wanting the card once the shell has engaged the lock, and
//!   the card, with whatever was typed in it, is dropped. The shell's screen is drawn from the same frame.
//! - The session unlocks while the card is up: the same.
//! - Several wrong passwords: the check's rate limiter answers `rate-limited` with the wait, and the card says so.
//! - An Enter on an empty entry, or while a check runs, is ignored.

use gnoblin_portal::{LockNotice, LockUi};
use gnoblin_portal_core::model::ScreenKey;
use gnoblin_portal_ui::style::DialogStyle;
use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, Setup};
use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_secret::SecretText;
use gnoblin_types::{KeyState, OutputId, Rect, Size, Tick};
use gnoblin_wayland::{LockCardAnswer, LockCardInput, Modal, ModalEvent};

use super::localtime::Zone;
use super::portal::{dialog_placement, screen_key};
use crate::log;

/// The card's bounds before an output says how large it may be, in pixels.
const FRAME_WIDTH: usize = 640;
const FRAME_HEIGHT: usize = 360;

/// What one pump changes on the compositor.
#[derive(Default)]
pub(crate) struct LockCardScene {
    /// `Some(Some(card))` shows the card, `Some(None)` clears it, and `None` leaves it as it is.
    pub card: Option<Option<Modal>>,
    /// The passwords the user confirmed, to check.
    pub submits: Vec<SecretText>,
}

/// The built-in lock card.
pub(crate) struct LockCardService {
    shell: DialogShell,
    setup: Setup,
    zone: Zone,
    /// The card on screen, while it is.
    ui: Option<LockUi>,
    /// The output the card is shown on, while it is shown.
    shown: Option<OutputId>,
    /// The time of day the card shows.
    minute: Option<(u32, u32)>,
    /// Set by any input, answer or clock change; cleared when the frame is drawn.
    dirty: bool,
}

impl LockCardService {
    /// A service with no card up, drawing dark, in the zone of this process.
    pub(crate) fn new() -> Self {
        Self::in_zone(Zone::system())
    }

    fn in_zone(zone: Zone) -> Self {
        let setup = Setup {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
            reduced_motion: true,
            scheme: DialogScheme::Dark,
            style: DialogStyle::default(),
        };
        Self {
            shell: DialogShell::new(setup),
            setup,
            zone,
            ui: None,
            shown: None,
            minute: None,
            dirty: false,
        }
    }

    /// Takes the dialog scheme and style from a committed configuration, as the other built-in dialogs do.
    pub(crate) fn configure(&mut self, config: &gnoblin_config::Config) {
        let scheme = match config
            .appearance
            .as_ref()
            .and_then(|appearance| appearance.color_scheme)
        {
            Some(gnoblin_config::ColorScheme::PreferLight) => DialogScheme::Light,
            Some(gnoblin_config::ColorScheme::Default | gnoblin_config::ColorScheme::PreferDark) | None => {
                DialogScheme::Dark
            }
        };
        self.setup.scheme = scheme;
        self.setup.style = crate::portal::dialog_style(config);
        self.shell = DialogShell::new(self.setup);
        self.dirty = true;
    }

    /// Whether the session needs asking about the card: it is up, or its overlay is still set and must be cleared.
    pub(crate) fn wants_pump(&self) -> bool {
        self.ui.is_some() || self.shown.is_some()
    }

    /// Whether the card is up.
    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.ui.is_some()
    }

    /// Routes what the session took for the card and says what the overlay must show now. The card is drawn centred on
    /// `output` while it is wanted. The events are overwritten afterwards: they hold the characters that were typed.
    pub(crate) fn pump(
        &mut self,
        output: Option<(OutputId, Rect)>,
        mut input: LockCardInput,
        unix_seconds: i64,
        now: Tick,
    ) -> LockCardScene {
        let mut scene = LockCardScene::default();
        let Some((id, rect)) = output.filter(|_| input.wanted) else {
            self.drop_card(&mut input.events);
            scene.card = self.shown.take().map(|_| None);
            return scene;
        };
        self.fit(rect);
        if self.ui.is_none() {
            log::info!(
                area = "lock",
                "no shell holds the lock screen: the built-in lock card is up"
            );
            self.ui = Some(LockUi::new());
            self.minute = None;
            self.dirty = true;
        }
        let minute = self.zone.hour_and_minute(unix_seconds);
        let Some(ui) = self.ui.as_mut() else {
            return scene;
        };
        if self.minute != Some(minute) {
            ui.set_clock(format!("{:02}:{:02}", minute.0, minute.1));
            self.minute = Some(minute);
            self.dirty = true;
        }
        for answer in input.answers.drain(..) {
            ui.set_notice(match answer {
                // The session is unlocking: the card is about to be dropped, and says nothing more.
                LockCardAnswer::Unlocked => LockNotice::Ready,
                LockCardAnswer::Denied => LockNotice::Denied,
                LockCardAnswer::RateLimited { milliseconds } => LockNotice::RateLimited(milliseconds),
                LockCardAnswer::Busy => LockNotice::Busy,
                LockCardAnswer::Failed => LockNotice::Failed,
            });
            self.dirty = true;
        }
        for event in &input.events {
            self.dirty |= route(ui, event);
        }
        ModalEvent::wipe_all(&mut input.events);
        if let Some(password) = ui.take_submitted() {
            scene.submits.push(password);
        }
        if self.dirty || self.shown != Some(id) {
            scene.card = self.frame_modal(id, rect, now).map(Some);
        }
        scene
    }

    /// Drops the card and wipes what was typed into it, and the events that carried it.
    fn drop_card(&mut self, events: &mut [ModalEvent]) {
        if let Some(mut ui) = self.ui.take() {
            ui.wipe();
            log::info!(area = "lock", "the built-in lock card is down");
        }
        ModalEvent::wipe_all(events);
        self.minute = None;
    }

    /// Makes the dialog as large as the output allows.
    fn fit(&mut self, rect: Rect) {
        let width = usize::try_from(rect.width).unwrap_or(usize::MAX);
        let height = usize::try_from(rect.height).unwrap_or(usize::MAX);
        if (self.setup.width, self.setup.height) != (width, height) {
            self.setup.width = width;
            self.setup.height = height;
            self.shell.set_bounds(width, height);
            self.dirty = true;
        }
    }

    /// The card's frame as an overlay modal.
    fn frame_modal(&mut self, output: OutputId, rect: Rect, now: Tick) -> Option<Modal> {
        let ui = self.ui.as_mut()?;
        self.dirty = false;
        let frame = self.shell.render(ui, now);
        let width = u32::try_from(frame.width()).ok()?;
        let height = u32::try_from(frame.height()).ok()?;
        let image = Image::new(Size::new(width, height), frame.width() * 4, frame.bytes().to_vec())?;
        let overlay = Overlay::new(dialog_placement(rect, width, height), &image)?.with_secure(true);
        self.shown = Some(output);
        Some(Modal::new(output, overlay).with_text())
    }
}

/// Feeds one event to the card. Returns whether it changed what the card shows.
fn route(ui: &mut LockUi, event: &ModalEvent) -> bool {
    match *event {
        ModalEvent::Key { keycode, state } => {
            // Space arrives as the character it types, so the key edge is not read twice.
            match screen_key(keycode) {
                Some(ScreenKey::Enter) if state == KeyState::Pressed => ui.submit(),
                Some(ScreenKey::Backspace) if state == KeyState::Pressed => ui.backspace(),
                _ => return false,
            }
            true
        }
        ModalEvent::Text { character } => {
            ui.type_char(character);
            true
        }
        ModalEvent::PointerButton { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> OutputId {
        OutputId::new(1)
    }

    fn output() -> Option<(OutputId, Rect)> {
        Some((id(), Rect::new(0, 0, 1280, 720)))
    }

    fn wanted(events: Vec<ModalEvent>, answers: Vec<LockCardAnswer>) -> LockCardInput {
        LockCardInput {
            wanted: true,
            events,
            answers,
        }
    }

    fn type_text(text: &str) -> Vec<ModalEvent> {
        text.chars().map(|character| ModalEvent::Text { character }).collect()
    }

    fn enter() -> ModalEvent {
        ModalEvent::Key {
            keycode: 28,
            state: KeyState::Pressed,
        }
    }

    fn service() -> LockCardService {
        LockCardService::in_zone(Zone::UTC)
    }

    #[test]
    fn the_card_comes_up_when_wanted_and_is_shown_as_a_modal_that_takes_text() {
        let mut service = service();
        assert!(!service.wants_pump());
        let scene = service.pump(output(), wanted(Vec::new(), Vec::new()), 0, Tick::ZERO);
        let card = scene.card.expect("the card is shown").expect("as a modal");
        assert!(
            card.takes_text(),
            "the card has an entry, so the compositor sends it text"
        );
        assert!(service.is_open() && service.wants_pump());
        assert!(
            service
                .pump(output(), wanted(Vec::new(), Vec::new()), 0, Tick::ZERO)
                .card
                .is_none(),
            "nothing changed, so the overlay stays"
        );
    }

    #[test]
    fn typing_and_enter_hand_the_password_out_and_the_card_says_it_is_checking() {
        let mut service = service();
        service.pump(output(), wanted(Vec::new(), Vec::new()), 0, Tick::ZERO);
        let mut events = type_text("hunter2");
        events.push(enter());
        let scene = service.pump(output(), wanted(events, Vec::new()), 0, Tick::ZERO);
        assert_eq!(scene.submits.len(), 1);
        assert_eq!(scene.submits[0].expose(), "hunter2");
        assert!(scene.card.is_some(), "the card is drawn again for the checking line");
    }

    #[test]
    fn an_enter_on_an_empty_entry_or_during_a_check_hands_nothing_out() {
        let mut service = service();
        service.pump(output(), wanted(Vec::new(), Vec::new()), 0, Tick::ZERO);
        let scene = service.pump(output(), wanted(vec![enter()], Vec::new()), 0, Tick::ZERO);
        assert!(scene.submits.is_empty(), "a slipped Enter sends nothing");
        let mut events = type_text("one");
        events.push(enter());
        events.extend(type_text("two"));
        events.push(enter());
        let scene = service.pump(output(), wanted(events, Vec::new()), 0, Tick::ZERO);
        assert_eq!(scene.submits.len(), 1, "one check runs at a time");
        assert_eq!(scene.submits[0].expose(), "one");
    }

    #[test]
    fn an_answer_reopens_the_entry_for_another_try() {
        let mut service = service();
        service.pump(output(), wanted(Vec::new(), Vec::new()), 0, Tick::ZERO);
        let mut events = type_text("nope");
        events.push(enter());
        service.pump(output(), wanted(events, Vec::new()), 0, Tick::ZERO);
        let mut again = type_text("again");
        again.push(enter());
        let scene = service.pump(output(), wanted(again, vec![LockCardAnswer::Denied]), 0, Tick::ZERO);
        assert_eq!(scene.submits.len(), 1, "after the denial the next password goes out");
        assert_eq!(scene.submits[0].expose(), "again");
    }

    #[test]
    fn the_card_goes_when_it_is_no_longer_wanted_and_what_was_typed_goes_with_it() {
        let mut service = service();
        service.pump(output(), wanted(type_text("half"), Vec::new()), 0, Tick::ZERO);
        assert!(service.is_open());
        let scene = service.pump(output(), LockCardInput::default(), 0, Tick::ZERO);
        assert_eq!(
            scene.card.map(|card| card.is_none()),
            Some(true),
            "the overlay is cleared"
        );
        assert!(!service.is_open() && !service.wants_pump());
        let scene = service.pump(output(), wanted(vec![enter()], Vec::new()), 0, Tick::ZERO);
        assert!(
            scene.submits.is_empty(),
            "a new card starts empty: nothing typed before survived"
        );
    }

    #[test]
    fn the_clock_shows_the_time_of_day_and_the_card_is_drawn_again_each_minute() {
        let mut service = service();
        service.pump(output(), wanted(Vec::new(), Vec::new()), 3_600, Tick::ZERO);
        assert_eq!(
            service.ui.as_ref().map(|ui| ui.clock().to_owned()).as_deref(),
            Some("01:00")
        );
        let same = service.pump(output(), wanted(Vec::new(), Vec::new()), 3_659, Tick::ZERO);
        assert!(same.card.is_none(), "the same minute draws nothing");
        let next = service.pump(output(), wanted(Vec::new(), Vec::new()), 3_660, Tick::ZERO);
        assert!(next.card.is_some(), "a new minute is drawn");
        assert_eq!(
            service.ui.as_ref().map(|ui| ui.clock().to_owned()).as_deref(),
            Some("01:01")
        );
    }

    #[test]
    fn no_output_means_no_card() {
        let mut service = service();
        let scene = service.pump(None, wanted(Vec::new(), Vec::new()), 0, Tick::ZERO);
        assert!(scene.card.is_none() && !service.is_open());
    }
}
