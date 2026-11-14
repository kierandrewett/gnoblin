//! The compositor's own passphrase and polkit dialogs, and the queue of requests they serve.
//!
//! A stock shell draws the prompt for the keyring when it claims the `prompt.secret` slot. This is the
//! built-in that serves the request when none does, so a session with no shell, a crashed runtime or a
//! script that never claimed the slot can still unlock the keyring (design/js-research/popups-portals.md,
//! row 5, and section 5). The same service draws the polkit authentication dialog for `auth.polkit`, which no
//! shell may claim: the compositor runs polkit's helper and the user's password never reaches a script
//! (row 6, and design/js-research/polkit.md). The request layer decides when a request is the built-in's
//! ([`super::requests`]): it holds the request and its deadline, and this module only draws it and reports how the
//! user ended it.
//!
//! [`SecretService`] never touches a socket. [`SecretService::pump`] and the other entry points return the
//! [`SecretAction`]s the dispatcher turns into an answer or a decline on the request.
//!
//! # What a script cannot do to it
//!
//! - It is a [`Modal`]: the compositor's own overlay, above every window, layer and shell node, so no script
//!   can draw over it, and its pixels are marked secure so a screen capture leaves them out.
//! - While it stands, every key and button goes to it and to no client ([`Modal`] routing), so nothing but the
//!   compositor sees the passphrase. The lock drawn on it is part of its frame.
//! - The passphrase is a [`gnoblin_secret::SecretText`] from the first key to the answer. The modal's key
//!   events are overwritten once routed, and nothing here logs more than a request id.
//!
//! # Failure modes
//!
//! - Several prompts at once: they queue, and the next one opens when the one on screen ends.
//! - The request ends while its dialog is open (deadline, requester gone): [`SecretService::close`] drops the
//!   dialog and its text without an answer, and a polkit dialog's helper is killed with it.
//! - The session locks: every prompt is declined, as the consent dialogs are refused `locked`, and a prompt
//!   that arrives while locked is declined at once.
//! - An Enter on an empty entry is ignored, so a slipped key never answers with nothing.

use std::collections::VecDeque;

use gnoblin_polkit_auth::AuthRequest;
use gnoblin_portal::{SecretResult, SecretUi};
use gnoblin_portal_core::model::ScreenKey;
use gnoblin_portal_ui::style::DialogStyle;
use gnoblin_portal_ui::ui::{DialogScheme, DialogShell, DialogUi, Setup, press_events, release_events};
use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_secret::SecretText;
use gnoblin_types::{KeyState, OutputId, Rect, RequestId, Size, Tick};
use gnoblin_wayland::{Modal, ModalEvent};

use super::polkit::{HelperPath, PolkitDialog, PolkitOutcome};
use super::portal::{dialog_placement, frame_point, screen_key};
use super::requests::SecretPrompt;
use crate::log;

/// The dialog frame's bounds before an output says how large it may be, in pixels.
const FRAME_WIDTH: usize = 640;
const FRAME_HEIGHT: usize = 360;

/// The evdev code of the left mouse button. Only it presses the dialog's buttons.
const BTN_LEFT: u32 = 0x110;

/// How the user ended a prompt, for the dispatcher to finish the request with.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SecretAction {
    /// The user confirmed with this passphrase.
    Answer {
        /// The request the passphrase answers.
        request: RequestId,
        /// The passphrase, wiped when it drops.
        secret: SecretText,
    },
    /// polkit's helper said the user authenticated as this identity (`unix-user:alice`). The helper has already told
    /// polkitd; the answer lets the agent end its `BeginAuthentication`.
    Authenticated {
        /// The request that was authenticated.
        request: RequestId,
        /// Who authenticated, as polkit writes an identity.
        identity: String,
    },
    /// The user declined, or the dialog could not be shown.
    Decline(RequestId),
}

/// What one pump changes on the compositor.
#[derive(Default)]
pub(crate) struct SecretScene {
    /// `Some(Some(modal))` shows the dialog, `Some(None)` clears it, and `None` leaves the modal as it is.
    pub modal: Option<Option<Modal>>,
    /// How prompts ended during the pump.
    pub actions: Vec<SecretAction>,
}

/// A request the built-in dialogs serve, before its dialog is on screen.
enum BuiltinPrompt {
    Secret(SecretPrompt),
    Polkit(AuthRequest),
}

/// The card on screen: what draws and takes the keys.
enum Card {
    Secret(SecretUi),
    Polkit(Box<PolkitDialog>),
}

impl Card {
    /// The card as the dialog shell draws it.
    fn ui(&mut self) -> &mut dyn DialogUi {
        match self {
            Card::Secret(ui) => ui,
            Card::Polkit(dialog) => dialog.ui_mut(),
        }
    }
}

/// The prompt on screen.
struct Active {
    request: RequestId,
    card: Card,
    /// Set by any input and by a new prompt; cleared when the frame is drawn.
    dirty: bool,
}

/// The built-in passphrase and polkit dialog session.
pub(crate) struct SecretService {
    shell: DialogShell,
    setup: Setup,
    now: Tick,
    active: Option<Active>,
    /// Prompts waiting for the one on screen to end, oldest first.
    pending: VecDeque<(RequestId, BuiltinPrompt)>,
    /// The output the modal is shown on, while it is shown.
    shown: Option<OutputId>,
    locked: bool,
    /// Where polkit's helper is, for the polkit dialogs.
    helper: HelperPath,
}

impl SecretService {
    /// A service with no prompt open, drawing dark.
    pub(crate) fn new() -> Self {
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
            now: Tick::ZERO,
            active: None,
            pending: VecDeque::new(),
            shown: None,
            locked: false,
            helper: HelperPath::default(),
        }
    }

    /// Runs polkit's helper from `helper` instead of looking for it.
    pub(crate) fn with_helper(mut self, helper: HelperPath) -> Self {
        self.helper = helper;
        self
    }

    /// Takes the dialog scheme from a committed configuration, as the consent dialog does.
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
        if let Some(active) = self.active.as_mut() {
            active.dirty = true;
        }
    }

    /// Whether a prompt is on screen. The modal belongs to it, so a consent dialog stands aside meanwhile.
    pub(crate) fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// Whether the modal needs routing: a prompt is open, or its modal is still set and must be cleared.
    pub(crate) fn wants_pump(&self) -> bool {
        self.active.is_some() || self.shown.is_some()
    }

    /// The clock moved. Only the dialog's drawing reads it: the request's deadline is the broker's.
    pub(crate) fn tick(&mut self, now: Tick) {
        self.now = now;
    }

    /// Opens the prompt for `request`, or queues it behind the one on screen.
    pub(crate) fn open(&mut self, request: RequestId, prompt: SecretPrompt) -> Vec<SecretAction> {
        self.enqueue(request, BuiltinPrompt::Secret(prompt))
    }

    /// Opens the polkit dialog for `request`, or queues it behind the one on screen.
    pub(crate) fn open_polkit(&mut self, request: RequestId, auth: AuthRequest) -> Vec<SecretAction> {
        self.enqueue(request, BuiltinPrompt::Polkit(auth))
    }

    fn enqueue(&mut self, request: RequestId, prompt: BuiltinPrompt) -> Vec<SecretAction> {
        if self.locked {
            return vec![SecretAction::Decline(request)];
        }
        log::debug!(
            area = "secret",
            request = request.get(),
            queued = self.active.is_some(),
            "the built-in dialog took a prompt"
        );
        if self.active.is_some() {
            self.pending.push_back((request, prompt));
        } else {
            self.start(request, prompt);
        }
        Vec::new()
    }

    /// The request ended without the dialog: its deadline passed or its host left. The dialog and what was
    /// typed in it are dropped, and the next queued prompt opens.
    pub(crate) fn close(&mut self, request: RequestId) {
        self.pending.retain(|(queued, _)| *queued != request);
        if self.active.as_ref().is_some_and(|active| active.request == request) {
            self.active = None;
            self.start_next();
        }
    }

    /// The session lock changed. Locking declines every prompt, because the lock cover owns the output.
    pub(crate) fn set_locked(&mut self, locked: bool) -> Vec<SecretAction> {
        self.locked = locked;
        if !locked {
            return Vec::new();
        }
        let mut actions: Vec<SecretAction> = self
            .active
            .take()
            .map(|active| SecretAction::Decline(active.request))
            .into_iter()
            .collect();
        actions.extend(
            std::mem::take(&mut self.pending)
                .into_iter()
                .map(|(request, _)| SecretAction::Decline(request)),
        );
        actions
    }

    /// Feeds one key to the open prompt, as `portal.key` and the modal's own keys do. Keys the entry does
    /// not use are ignored.
    pub(crate) fn key(&mut self, key: ScreenKey) -> Vec<SecretAction> {
        let Some(active) = self.active.as_mut() else {
            return Vec::new();
        };
        match &mut active.card {
            Card::Secret(ui) => match key {
                ScreenKey::Escape => ui.cancel(),
                ScreenKey::Enter => ui.submit(),
                ScreenKey::Backspace => ui.backspace(),
                ScreenKey::Space => ui.type_char(' '),
                ScreenKey::Text(character) => ui.type_char(character),
                ScreenKey::Tab | ScreenKey::Up | ScreenKey::Down => return Vec::new(),
            },
            Card::Polkit(dialog) => dialog.key(key),
        }
        active.dirty = true;
        self.settle()
    }

    /// Routes the modal events the compositor queued and says what the modal must show now. The dialog is
    /// drawn centred on `output` while a prompt is open. The events are overwritten afterwards: they hold
    /// the characters that were typed.
    pub(crate) fn pump(&mut self, output: Option<(OutputId, Rect)>, events: &mut [ModalEvent]) -> SecretScene {
        if let Some((_, rect)) = output {
            let width = usize::try_from(rect.width).unwrap_or(usize::MAX);
            let height = usize::try_from(rect.height).unwrap_or(usize::MAX);
            if (self.setup.width, self.setup.height) != (width, height) {
                self.setup.width = width;
                self.setup.height = height;
                self.shell.set_bounds(width, height);
                if let Some(active) = self.active.as_mut() {
                    active.dirty = true;
                }
            }
        }
        let placement = self.placement(output);
        // What polkit's helper said since the last pump: a prompt, a message, its verdict.
        let mut actions = self.settle();
        for event in events.iter() {
            actions.extend(self.route(event, placement));
        }
        for event in events.iter_mut() {
            *event = ModalEvent::Key {
                keycode: 0,
                state: KeyState::Released,
            };
        }
        let modal = match output {
            Some((id, rect)) if self.active.is_some() => self.frame_modal(id, rect),
            _ => self.clear_modal(),
        };
        SecretScene { modal, actions }
    }

    fn start(&mut self, request: RequestId, prompt: BuiltinPrompt) {
        let mut card = match prompt {
            BuiltinPrompt::Secret(prompt) => Card::Secret(SecretUi::new(prompt.title, prompt.message, prompt.subject)),
            BuiltinPrompt::Polkit(auth) => Card::Polkit(Box::new(PolkitDialog::new(auth, self.helper.clone()))),
        };
        self.shell.layout(card.ui(), self.now);
        self.active = Some(Active {
            request,
            card,
            dirty: true,
        });
    }

    fn start_next(&mut self) {
        if let Some((request, prompt)) = self.pending.pop_front() {
            self.start(request, prompt);
        }
    }

    /// Takes how the open prompt ended, if it did, and opens the next one. A polkit dialog is also asked what its
    /// helper said, so this is what moves it along.
    fn settle(&mut self) -> Vec<SecretAction> {
        let Some(active) = self.active.as_mut() else {
            return Vec::new();
        };
        let request = active.request;
        let action = match &mut active.card {
            Card::Secret(ui) => match ui.take_result() {
                Some(SecretResult::Answered(secret)) => SecretAction::Answer { request, secret },
                Some(SecretResult::Declined) => SecretAction::Decline(request),
                None => return Vec::new(),
            },
            Card::Polkit(dialog) => {
                let step = dialog.advance();
                if step.changed {
                    active.dirty = true;
                }
                match step.outcome {
                    Some(PolkitOutcome::Authenticated { identity }) => {
                        SecretAction::Authenticated { request, identity }
                    }
                    Some(PolkitOutcome::Declined) => SecretAction::Decline(request),
                    None => return Vec::new(),
                }
            }
        };
        self.active = None;
        self.start_next();
        log::debug!(
            area = "secret",
            request = request.get(),
            "the built-in dialog ended a prompt"
        );
        vec![action]
    }

    /// The rectangle the open dialog sits at on `output`, or `None` when no prompt is open.
    fn placement(&self, output: Option<(OutputId, Rect)>) -> Option<Rect> {
        let (_, rect) = output?;
        self.active.as_ref()?;
        let (width, height) = self.shell.size();
        Some(dialog_placement(
            rect,
            u32::try_from(width).ok()?,
            u32::try_from(height).ok()?,
        ))
    }

    fn route(&mut self, event: &ModalEvent, placement: Option<Rect>) -> Vec<SecretAction> {
        match *event {
            ModalEvent::Key { keycode, state } => {
                // Space arrives as the character it types, so the key edge is not read twice.
                match screen_key(keycode) {
                    Some(key) if state == KeyState::Pressed && key != ScreenKey::Space => self.key(key),
                    _ => Vec::new(),
                }
            }
            ModalEvent::Text { character } => self.key(ScreenKey::Text(character)),
            ModalEvent::PointerButton { x, y, button, state } => {
                let Some(placement) = placement.filter(|_| button == BTN_LEFT) else {
                    return Vec::new();
                };
                let Some((fx, fy)) = frame_point(placement, x, y) else {
                    return Vec::new();
                };
                let now = self.now;
                let Some(active) = self.active.as_mut() else {
                    return Vec::new();
                };
                // Hit testing needs the frame that laid the buttons out.
                self.shell.layout(active.card.ui(), now);
                let events = match state {
                    KeyState::Pressed => press_events(fx, fy),
                    KeyState::Released => release_events(fx, fy),
                };
                let _ = self.shell.run(active.card.ui(), events, now);
                active.dirty = true;
                self.settle()
            }
        }
    }

    /// The dialog's frame as a modal, when it changed since the last one.
    fn frame_modal(&mut self, output: OutputId, rect: Rect) -> Option<Option<Modal>> {
        let active = self.active.as_mut()?;
        if !active.dirty && self.shown == Some(output) {
            return None;
        }
        active.dirty = false;
        let frame = self.shell.render(active.card.ui(), self.now);
        let width = u32::try_from(frame.width()).ok()?;
        let height = u32::try_from(frame.height()).ok()?;
        let image = Image::new(Size::new(width, height), frame.width() * 4, frame.bytes().to_vec())?;
        let overlay = Overlay::new(dialog_placement(rect, width, height), &image)?.with_secure(true);
        self.shown = Some(output);
        Some(Some(Modal::new(output, overlay).with_text()))
    }

    /// Clears the modal once it was shown, and only then.
    fn clear_modal(&mut self) -> Option<Option<Modal>> {
        self.shown.take().map(|_| None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: RequestId = RequestId::new(1);
    const SECOND: RequestId = RequestId::new(2);

    fn prompt(subject: &str) -> SecretPrompt {
        SecretPrompt {
            title: "Unlock the keyring".to_owned(),
            message: "Enter your passphrase".to_owned(),
            subject: subject.to_owned(),
        }
    }

    fn type_text(service: &mut SecretService, text: &str) {
        for character in text.chars() {
            assert!(service.key(ScreenKey::Text(character)).is_empty());
        }
    }

    fn output() -> Option<(OutputId, Rect)> {
        Some((OutputId::new(1), Rect::new(0, 0, 1280, 720)))
    }

    #[test]
    fn typing_and_enter_answers_the_request_with_the_passphrase() {
        let mut service = SecretService::new();
        assert!(service.open(FIRST, prompt("keyring")).is_empty());
        assert!(service.is_open());
        type_text(&mut service, "correct horse");
        let actions = service.key(ScreenKey::Enter);
        assert_eq!(
            actions,
            vec![SecretAction::Answer {
                request: FIRST,
                secret: SecretText::new("correct horse"),
            }]
        );
        assert!(!service.is_open(), "the prompt is gone once answered");
    }

    #[test]
    fn escape_declines() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        type_text(&mut service, "half typed");
        assert_eq!(service.key(ScreenKey::Escape), vec![SecretAction::Decline(FIRST)]);
        assert!(!service.is_open());
    }

    #[test]
    fn enter_on_an_empty_entry_answers_nothing() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        assert!(service.key(ScreenKey::Enter).is_empty());
        assert!(service.is_open());
    }

    #[test]
    fn modal_events_type_and_answer_and_are_overwritten() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        let mut events = vec![
            ModalEvent::Key {
                keycode: 30,
                state: KeyState::Pressed,
            },
            ModalEvent::Text { character: 'a' },
            ModalEvent::Key {
                keycode: 57,
                state: KeyState::Pressed,
            },
            ModalEvent::Text { character: ' ' },
            ModalEvent::Text { character: 'b' },
            ModalEvent::Key {
                keycode: 28,
                state: KeyState::Pressed,
            },
        ];
        let scene = service.pump(output(), &mut events);
        assert_eq!(
            scene.actions,
            vec![SecretAction::Answer {
                request: FIRST,
                secret: SecretText::new("a b"),
            }],
            "the space is typed once, from its character"
        );
        assert!(
            events.iter().all(|event| !matches!(event, ModalEvent::Text { .. })),
            "the typed characters do not outlive the pump"
        );
    }

    #[test]
    fn the_modal_takes_text_and_is_cleared_when_the_prompt_ends() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        let scene = service.pump(output(), &mut []);
        let modal = scene.modal.expect("the dialog is shown").expect("as a modal");
        assert!(
            modal.takes_text(),
            "the dialog has an entry, so the compositor sends it text"
        );
        assert!(
            service.pump(output(), &mut []).modal.is_none(),
            "nothing changed, so the modal stays"
        );
        let mut escape = [ModalEvent::Key {
            keycode: 1,
            state: KeyState::Pressed,
        }];
        let scene = service.pump(output(), &mut escape);
        assert_eq!(scene.actions, vec![SecretAction::Decline(FIRST)]);
        assert!(matches!(scene.modal, Some(None)), "the modal is cleared");
        assert!(!service.wants_pump());
    }

    #[test]
    fn typing_redraws_the_modal() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        let _ = service.pump(output(), &mut []);
        let mut events = [ModalEvent::Text { character: 'x' }];
        let scene = service.pump(output(), &mut events);
        assert!(
            matches!(scene.modal, Some(Some(_))),
            "the entry shows a mark for the character"
        );
    }

    #[test]
    fn prompts_queue_behind_the_one_on_screen() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("one"));
        service.open(SECOND, prompt("two"));
        type_text(&mut service, "a");
        let first = service.key(ScreenKey::Enter);
        assert!(matches!(first.as_slice(), [SecretAction::Answer { request, .. }] if *request == FIRST));
        assert!(service.is_open(), "the second prompt opened");
        type_text(&mut service, "b");
        let second = service.key(ScreenKey::Enter);
        assert!(matches!(second.as_slice(), [SecretAction::Answer { request, .. }] if *request == SECOND));
    }

    #[test]
    fn a_request_that_ended_elsewhere_closes_its_dialog_and_opens_the_next() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("one"));
        service.open(SECOND, prompt("two"));
        type_text(&mut service, "stale");
        service.close(FIRST);
        assert!(service.is_open(), "the queued prompt took over");
        assert!(
            service.key(ScreenKey::Enter).is_empty(),
            "and it starts with an empty entry"
        );
        service.close(SECOND);
        assert!(!service.is_open());
    }

    #[test]
    fn locking_declines_every_prompt_and_a_locked_session_declines_new_ones() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("one"));
        service.open(SECOND, prompt("two"));
        assert_eq!(
            service.set_locked(true),
            vec![SecretAction::Decline(FIRST), SecretAction::Decline(SECOND)]
        );
        assert_eq!(
            service.open(RequestId::new(3), prompt("late")),
            vec![SecretAction::Decline(RequestId::new(3))]
        );
        assert!(service.set_locked(false).is_empty());
        assert!(service.open(RequestId::new(4), prompt("again")).is_empty());
    }

    #[test]
    fn nothing_the_service_shows_in_debug_holds_the_passphrase() {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        type_text(&mut service, "swordfish");
        let actions = service.key(ScreenKey::Enter);
        let shown = format!("{actions:?}");
        assert!(!shown.contains("swordfish"), "{shown}");
        let event = format!("{:?}", ModalEvent::Text { character: 's' });
        assert!(!event.contains("'s'"), "{event}");
    }

    #[test]
    fn no_log_line_holds_the_passphrase() {
        let logged = gnoblin_log::event::capture::logged(|| {
            let mut service = SecretService::new();
            service.open(FIRST, prompt("keyring"));
            let mut events: Vec<ModalEvent> = "swordfish"
                .chars()
                .map(|character| ModalEvent::Text { character })
                .collect();
            events.push(ModalEvent::Key {
                keycode: 28,
                state: KeyState::Pressed,
            });
            let scene = service.pump(output(), &mut events);
            assert_eq!(scene.actions.len(), 1, "the prompt was answered");
        });
        for line in &logged {
            let shown = format!("{} {} {}", line.component, line.message, line.fields_line());
            assert!(!shown.contains("swordfish"), "{shown}");
        }
    }

    /// Clicks across the bottom row of the dialog, from the left or the right, until a click ends the prompt.
    /// The buttons are laid out by egui, so the test finds them the way a pointer would.
    fn click_buttons(service: &mut SecretService, placed: Rect, from_right: bool) -> Vec<SecretAction> {
        let bottom = f64::from(placed.y) + f64::from(placed.height);
        let width = f64::from(placed.width);
        for row in 0..8_u8 {
            let y = bottom - 8.0 - f64::from(row) * 4.0;
            for step in 0..60_u8 {
                let offset = 6.0 + f64::from(step) * 5.0;
                let x = f64::from(placed.x) + if from_right { width - offset } else { offset };
                let mut click = [
                    ModalEvent::PointerButton {
                        x,
                        y,
                        button: BTN_LEFT,
                        state: KeyState::Pressed,
                    },
                    ModalEvent::PointerButton {
                        x,
                        y,
                        button: BTN_LEFT,
                        state: KeyState::Released,
                    },
                ];
                let actions = service.pump(output(), &mut click).actions;
                if !actions.is_empty() {
                    return actions;
                }
            }
        }
        Vec::new()
    }

    /// A service with a prompt open, `pw` typed, and its first frame drawn, and where that frame sits.
    fn shown_with_text() -> (SecretService, Rect) {
        let mut service = SecretService::new();
        service.open(FIRST, prompt("keyring"));
        type_text(&mut service, "pw");
        let modal = service
            .pump(output(), &mut [])
            .modal
            .expect("the dialog is shown")
            .expect("as a modal");
        assert!(modal.overlay().is_secure(), "a capture leaves the dialog out");
        let placed = modal.overlay().dst();
        (service, placed)
    }

    #[test]
    fn clicking_unlock_answers() {
        let (mut service, placed) = shown_with_text();
        let actions = click_buttons(&mut service, placed, true);
        assert!(
            matches!(actions.as_slice(), [SecretAction::Answer { secret, .. }] if secret.expose() == "pw"),
            "{actions:?}"
        );
    }

    #[test]
    fn clicking_cancel_declines() {
        let (mut service, placed) = shown_with_text();
        let actions = click_buttons(&mut service, placed, false);
        assert_eq!(actions, vec![SecretAction::Decline(FIRST)]);
    }

    #[test]
    fn a_click_outside_the_dialog_does_nothing() {
        let (mut service, _) = shown_with_text();
        let mut click = [
            ModalEvent::PointerButton {
                x: 2.0,
                y: 2.0,
                button: BTN_LEFT,
                state: KeyState::Pressed,
            },
            ModalEvent::PointerButton {
                x: 2.0,
                y: 2.0,
                button: BTN_LEFT,
                state: KeyState::Released,
            },
        ];
        assert!(service.pump(output(), &mut click).actions.is_empty());
        assert!(service.is_open());
    }

    // --- polkit authentication ------------------------------------------------------------------------------------

    /// Pumps the service until `ready` holds or a prompt ended, and returns how prompts ended, ten seconds at most.
    fn pump_for(service: &mut SecretService, mut ready: impl FnMut(&SecretService) -> bool) -> Vec<SecretAction> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut actions = Vec::new();
        while actions.is_empty() && !ready(service) {
            actions.extend(service.pump(output(), &mut []).actions);
            assert!(std::time::Instant::now() < deadline, "the dialog did not get there");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        actions
    }

    fn asking(service: &SecretService) -> bool {
        matches!(service.active.as_ref().map(|active| &active.card), Some(Card::Polkit(dialog)) if dialog.is_asking())
    }

    /// A service with a polkit dialog open for this process's own account, behind the fake helper that accepts
    /// `hunter2`, and the account's name.
    fn polkit_open() -> (SecretService, super::super::polkit::fake::Fake, String) {
        let (uid, name) = super::super::polkit::fake::me();
        let helper = super::super::polkit::fake::password_helper(&name);
        let mut service = SecretService::new().with_helper(helper.path());
        assert!(
            service
                .open_polkit(FIRST, super::super::polkit::fake::request(uid))
                .is_empty()
        );
        (service, helper, name)
    }

    #[test]
    fn a_polkit_dialog_authenticates_only_on_the_helpers_success() {
        let (mut service, _helper, name) = polkit_open();
        assert!(service.is_open());
        // A key typed before PAM asks is nobody's answer, so the test waits for the prompt first.
        assert!(pump_for(&mut service, asking).is_empty());
        type_text(&mut service, "hunter2");
        let mut actions = service.key(ScreenKey::Enter);
        actions.extend(pump_for(&mut service, |_| false));
        assert_eq!(
            actions,
            vec![SecretAction::Authenticated {
                request: FIRST,
                identity: format!("unix-user:{name}"),
            }]
        );
        assert!(!service.is_open(), "the dialog is gone once the helper said so");
    }

    #[test]
    fn a_wrong_polkit_password_is_not_an_authentication() {
        let (mut service, _helper, name) = polkit_open();
        pump_for(&mut service, asking);
        type_text(&mut service, "nope");
        assert!(
            service.key(ScreenKey::Enter).is_empty(),
            "a password is not an answer until the helper says so"
        );
        // The helper failed, so the dialog is still open and asks again; the right password then succeeds.
        assert!(pump_for(&mut service, asking).is_empty());
        assert!(service.is_open(), "a failure is not an answer");
        type_text(&mut service, "hunter2");
        let mut actions = service.key(ScreenKey::Enter);
        actions.extend(pump_for(&mut service, |_| false));
        assert_eq!(
            actions,
            vec![SecretAction::Authenticated {
                request: FIRST,
                identity: format!("unix-user:{name}"),
            }]
        );
    }

    #[test]
    fn escape_declines_a_polkit_dialog_and_a_lock_declines_it_too() {
        let (mut service, _helper, _) = polkit_open();
        pump_for(&mut service, asking);
        assert_eq!(service.key(ScreenKey::Escape), vec![SecretAction::Decline(FIRST)]);
        assert!(!service.is_open());

        let (mut service, _helper, _) = polkit_open();
        pump_for(&mut service, asking);
        assert_eq!(service.set_locked(true), vec![SecretAction::Decline(FIRST)]);
    }

    #[test]
    fn the_polkit_dialog_is_a_secure_modal_and_closing_it_ends_it_without_an_answer() {
        let (mut service, _helper, _) = polkit_open();
        pump_for(&mut service, asking);
        // A key marks the frame changed, so the next pump hands the modal over again.
        service.key(ScreenKey::Text('x'));
        let modal = service
            .pump(output(), &mut [])
            .modal
            .expect("the dialog is shown")
            .expect("as a modal");
        assert!(modal.overlay().is_secure(), "a capture leaves the dialog out");
        service.close(FIRST);
        assert!(!service.is_open());
        assert!(service.pump(output(), &mut []).actions.is_empty());
    }

    #[test]
    fn no_log_line_and_no_debug_output_holds_the_polkit_password() {
        let logged = gnoblin_log::event::capture::logged(|| {
            let (mut service, _helper, _) = polkit_open();
            pump_for(&mut service, asking);
            let mut events: Vec<ModalEvent> = "hunter2"
                .chars()
                .map(|character| ModalEvent::Text { character })
                .collect();
            events.push(ModalEvent::Key {
                keycode: 28,
                state: KeyState::Pressed,
            });
            let scene = service.pump(output(), &mut events);
            let shown = format!("{:?} {events:?}", scene.actions);
            assert!(!shown.contains("hunter2"), "{shown}");
            let actions = pump_for(&mut service, |_| false);
            assert_eq!(actions.len(), 1, "the password was accepted");
            assert!(!format!("{actions:?}").contains("hunter2"));
        });
        for line in &logged {
            let shown = format!("{} {} {}", line.component, line.message, line.fields_line());
            assert!(!shown.contains("hunter2"), "{shown}");
        }
    }
}
