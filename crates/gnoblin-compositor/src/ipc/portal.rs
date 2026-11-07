//! The compositor-side service that connects `portal.ui` requests to the [`PortalUi`]
//! session. It records the owner of each admitted request, settles each outcome to
//! that owner, withdraws a departed connection's requests, and settles open requests on a lock.
//!
//! [`PortalService`] owns the [`PortalUi`] session and the owner of every request
//! it admitted. Each method returns the replies the dispatcher must write, so the
//! service itself never touches a socket. Its rules:
//!
//! - A request is answered by the connection that submitted it, and by no other.
//!   When the owner departs, its requests are withdrawn and no reply is sent for them.
//! - `portal.cancel` withdraws every request the calling connection holds. The
//!   wire carries no target id, so a connection's cancel names its own requests.
//! - `portal.key` routes one key to the open dialog and replies with an empty ok.
//!   Keys the dialog does not use are ignored by the dialog itself.
//! - Interfaces without a built-in dialog, and the file and app choosers (the
//!   wire's `Fields` options are not yet mapped to their typed dialogs), settle
//!   `error(unsupported-ui)`. A session lock settles `error(locked)`.
//! - A `parent_window` is honoured only when the window it names belongs to the
//!   caller's bus pid. The pid is the relay's verified value, never one from the payload,
//!   and an X11 parent is refused until the compositor can map XIDs to windows.
//!
//! # Failure modes
//!
//! - Request id already held by an open or queued request: refused `request-taken`.
//! - `portal.cancel` from a connection that holds nothing: refused `request-unknown`.
//! - `portal.key` naming a key the dialog grammar does not know: refused
//!   `unknown-key`, and nothing reaches the dialog.
//! - Admission refused by the queue or the lock: settles `error(busy|locked)` to the
//!   submitter, not a refusal, so the requester learns the verdict it waits for.

use gnoblin_backend::render::{Image, Overlay};
use gnoblin_client::wire::{self, PortalRequest, VerifiedCaller};
use gnoblin_portal::DialogScheme;
use gnoblin_portal_core::model::{Consent, Dialog, Grant, Interface, ScreenKey};
use gnoblin_proto::Envelope;
use gnoblin_types::{ClientId, KeyState, OutputId, Rect, RequestId, Size, Tick};
use gnoblin_wayland::{Modal, ModalEvent, WindowProcess};

use crate::portal::{CancelReason, PortalError, PortalUi, Verdict};

/// The built-in dialog's frame size, in pixels.
const FRAME_WIDTH: usize = 640;
const FRAME_HEIGHT: usize = 360;

/// The wire code for a key name the dialog grammar does not know.
const UNKNOWN_KEY: &str = "unknown-key";

/// The wire code for a payload that does not decode.
pub const MALFORMED: &str = "malformed";

/// A reply the dispatcher writes: the connection it goes to, and the frame.
pub type Reply = (ClientId, Envelope);

/// A request the service refuses before admitting it: the wire code and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalRefusal {
    /// Stable wire code the client matches on, such as `REQUEST_TAKEN`.
    pub code: &'static str,
    /// Human-readable detail for logs and error dialogs; not part of the contract.
    pub message: String,
}

/// The connection that submitted an admitted request, and the envelope id its
/// outcome is sent under.
struct HeldRequest {
    /// The request the client submitted, so the outcome can be routed back to it.
    request: RequestId,
    /// The connection that submitted the request; replies go only to this client.
    client: ClientId,
    /// The envelope id the outcome is sent under, so the client can pair it with its call.
    envelope_id: u64,
}

/// The portal UI session and the routing of its requests to their owners.
pub struct PortalService {
    /// The dialog UI that shows requests to the user, one dialog at a time.
    ui: PortalUi,
    /// Admitted requests still waiting for an outcome, in admission order.
    owners: Vec<HeldRequest>,
    /// The output the dialog modal is shown on, while one is shown.
    shown: Option<OutputId>,
}

impl Default for PortalService {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalService {
    /// Builds an empty portal service with no owners and no dialog shown.
    ///
    /// The dialog frame is sized to the fixed `FRAME_WIDTH` by `FRAME_HEIGHT`, so every
    /// portal dialog has the same geometry regardless of the request that opened it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ui: PortalUi::new(FRAME_WIDTH, FRAME_HEIGHT),
            owners: Vec::new(),
            shown: None,
        }
    }

    /// Takes the dialog scheme from a committed configuration: `prefer-light` draws
    /// light dialogs, and the default and `prefer-dark` draw dark ones. A configuration
    /// that names no scheme draws dark, as the stock desktop does.
    pub fn configure(&mut self, config: &gnoblin_config::Config) {
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
        self.ui.set_scheme(scheme);
    }

    /// `portal.ui`: admit a request from `client`, which is answered under
    /// `envelope_id`. A request the UI refuses at once replies its error now.
    pub fn submit(
        &mut self,
        client: ClientId,
        envelope_id: u64,
        request: &PortalRequest,
        now: Tick,
    ) -> Result<Vec<Reply>, PortalRefusal> {
        let id = RequestId::new(envelope_id);
        if self.owners.iter().any(|owner| owner.request == id) {
            return Err(PortalRefusal {
                code: wire::code::REQUEST_TAKEN,
                message: format!("portal request {envelope_id} is already held"),
            });
        }
        let Ok(interface) = Interface::from_name(&request.interface) else {
            return Ok(vec![outcome(
                client,
                envelope_id,
                Verdict::Error(PortalError::UnsupportedUi),
            )]);
        };
        // A consent dialog is the only kind offered here: the UI refuses any other
        // interface, including the choosers, as `unsupported-ui`.
        let dialog = Dialog::Consent(Consent::open(
            request.caller.app_id().to_owned(),
            request.prompt.clone(),
            now,
        ));
        match self
            .ui
            .submit(id, request.caller.app_id(), interface, dialog, request.timeout_ms, now)
        {
            Ok(_) => {
                self.owners.push(HeldRequest {
                    request: id,
                    client,
                    envelope_id,
                });
                Ok(Vec::new())
            }
            Err(error) => Ok(vec![outcome(client, envelope_id, Verdict::Error(error))]),
        }
    }

    /// `portal.key`: route one key to the open dialog at `now`. The clock is advanced
    /// first, because input arrives between idle ticks and the dialog's arming and
    /// animations judge the key against the time it was read, not the last tick.
    pub fn key(&mut self, name: &str, repeat: bool, now: Tick) -> Result<Vec<Reply>, PortalRefusal> {
        let Some(key) = parse_key(name) else {
            return Err(PortalRefusal {
                code: UNKNOWN_KEY,
                message: format!("`{name}` is not a key the portal dialog takes"),
            });
        };
        self.ui.tick(now);
        self.ui.key(key, repeat);
        Ok(self.settle())
    }

    /// `portal.cancel`: withdraw every request `client` holds. Refused when it holds
    /// none.
    pub fn cancel(&mut self, client: ClientId) -> Result<Vec<Reply>, PortalRefusal> {
        let (held, kept): (Vec<HeldRequest>, Vec<HeldRequest>) = std::mem::take(&mut self.owners)
            .into_iter()
            .partition(|owner| owner.client == client);
        self.owners = kept;
        if held.is_empty() {
            return Err(PortalRefusal {
                code: wire::code::REQUEST_UNKNOWN,
                message: "this connection holds no portal request".to_owned(),
            });
        }
        let mut replies = Vec::new();
        for owner in held {
            self.ui.cancel(owner.request);
            replies.push(outcome(
                owner.client,
                owner.envelope_id,
                Verdict::Cancelled(CancelReason::User),
            ));
        }
        replies.extend(self.settle());
        Ok(replies)
    }

    /// The connection `client` left: withdraw whatever it held. Its replies are
    /// dropped because the owner is gone.
    pub fn departed(&mut self, client: ClientId) -> Vec<Reply> {
        self.withdraw(client);
        self.settle()
    }

    /// The clock moved: animations advance and deadlines are enforced.
    pub fn tick(&mut self, now: Tick) -> Vec<Reply> {
        self.ui.tick(now);
        self.settle()
    }

    /// The session lock changed. Locking settles every held request `error(locked)`.
    /// `pump` feeds it from the lock state each tick.
    pub fn set_locked(&mut self, locked: bool) -> Vec<Reply> {
        self.ui.set_locked(locked);
        self.settle()
    }

    /// Whether the dialog modal needs routing: a dialog is open, or one is still
    /// shown and must be cleared. Otherwise the compositor need not drain events.
    #[must_use]
    pub fn wants_pump(&self) -> bool {
        self.shown.is_some() || self.ui.active_request().is_some()
    }

    /// Withdraws every request `client` holds from the UI. Returns whether it held any.
    fn withdraw(&mut self, client: ClientId) -> bool {
        let held: Vec<RequestId> = self
            .owners
            .iter()
            .filter(|owner| owner.client == client)
            .map(|owner| owner.request)
            .collect();
        for request in &held {
            self.ui.cancel(*request);
            self.owners.retain(|owner| owner.request != *request);
        }
        !held.is_empty()
    }

    /// The outcomes that settled since the last call, each addressed to its owner.
    /// An outcome with no owner (withdrawn) is dropped.
    fn settle(&mut self) -> Vec<Reply> {
        let mut replies = Vec::new();
        for (request, verdict) in self.ui.take_outcomes() {
            let Some(index) = self.owners.iter().position(|owner| owner.request == request) else {
                continue;
            };
            let owner = self.owners.remove(index);
            replies.push(outcome(owner.client, owner.envelope_id, verdict));
        }
        replies
    }
}

/// The `portal.ui` reply for one verdict.
fn outcome(client: ClientId, envelope_id: u64, verdict: Verdict) -> Reply {
    let envelope = match verdict {
        Verdict::Error(error) => Envelope::error(
            wire::NAMESPACE,
            wire::member::PORTAL_UI,
            envelope_id,
            error.as_str(),
            format!("the portal request settled as `{}`", error.as_str()),
        ),
        Verdict::Handled(grant) => wire::portal_outcome(envelope_id, "handled", grant_name(&grant)),
        Verdict::Cancelled(reason) => wire::portal_outcome(envelope_id, "cancelled", reason.as_str()),
        Verdict::Declined => wire::portal_outcome(envelope_id, "declined", ""),
    };
    (client, envelope)
}

/// The name a grant settles under. A consent grant is `allowed`.
const fn grant_name(grant: &Grant) -> &'static str {
    match grant {
        Grant::Allowed => "allowed",
        Grant::Files { .. } => "files",
        Grant::App { .. } => "app",
        Grant::Print { .. } => "print",
        Grant::Capture { .. } => "capture",
        Grant::Devices { .. } => "devices",
    }
}

/// Whether the window a request names as its `parent_window` belongs to the caller.
///
/// The caller is the relay's verified value: its pid came from the bus, never from the
/// request payload, so a payload that claims another pid is still checked against the bus
/// pid. `owner` is the process the compositor found for the named window. A pid of `0`
/// means the bus could not name the caller, and no process has that id, so nothing
/// matches it. A window with no process the compositor can name, or whose client has gone,
/// is never proof of ownership, so it is refused.
pub(super) fn parent_owner_check(caller: &VerifiedCaller, owner: &WindowProcess) -> Result<(), &'static str> {
    match owner {
        WindowProcess::Connected(identity) if caller.pid() != 0 && identity.pid == caller.pid() => Ok(()),
        WindowProcess::Connected(_) => Err("the parent window belongs to another process"),
        WindowProcess::Unknown => Err("the parent window's process is unknown"),
        WindowProcess::Disconnected => Err("the parent window's client has disconnected"),
    }
}

/// The key a `portal.key` name stands for: `escape`, `enter`, `tab`, `up`, `down`,
/// `space`, `backspace`, or `text:` and one character.
fn parse_key(name: &str) -> Option<ScreenKey> {
    Some(match name {
        "escape" => ScreenKey::Escape,
        "enter" => ScreenKey::Enter,
        "tab" => ScreenKey::Tab,
        "up" => ScreenKey::Up,
        "down" => ScreenKey::Down,
        "space" => ScreenKey::Space,
        "backspace" => ScreenKey::Backspace,
        _ => {
            let mut chars = name.strip_prefix("text:")?.chars();
            let character = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            ScreenKey::Text(character)
        }
    })
}

/// The dialog key an evdev keycode stands for, or `None` for a key the dialog does
/// not use. Shift is not a dialog key, and Shift+Tab reaches the dialog as Tab: the
/// consent grammar has no reverse focus step.
#[must_use]
pub fn screen_key(keycode: u32) -> Option<ScreenKey> {
    Some(match keycode {
        1 => ScreenKey::Escape,
        14 => ScreenKey::Backspace,
        15 => ScreenKey::Tab,
        28 | 96 => ScreenKey::Enter,
        57 => ScreenKey::Space,
        103 => ScreenKey::Up,
        108 => ScreenKey::Down,
        _ => return None,
    })
}

/// The evdev code of the left mouse button. Only it answers the dialog.
const BTN_LEFT: u32 = 0x110;

/// One axis of a centred placement: a frame of `frame` pixels at the middle of
/// `extent`, starting at `origin`. A frame larger than the extent sits at `origin`.
fn centred(origin: i32, extent: u32, frame: u32) -> i32 {
    let offset = extent.saturating_sub(frame) / 2;
    origin.saturating_add(i32::try_from(offset).unwrap_or(i32::MAX))
}

/// Where the dialog frame sits on an output: centred, and never past the output's
/// top left corner.
#[must_use]
pub fn dialog_placement(output: Rect, frame_width: u32, frame_height: u32) -> Rect {
    Rect::new(
        centred(output.x, output.width, frame_width),
        centred(output.y, output.height, frame_height),
        frame_width,
        frame_height,
    )
}

/// The frame pixel under a layout point, or `None` when the point lies outside the
/// placed dialog. The result is in `0..frame` on each axis.
#[must_use]
pub fn frame_point(placement: Rect, x: f64, y: f64) -> Option<(f32, f32)> {
    let left = f64::from(placement.x);
    let top = f64::from(placement.y);
    let right = left + f64::from(placement.width);
    let bottom = top + f64::from(placement.height);
    if x < left || y < top || x >= right || y >= bottom {
        return None;
    }
    Some(((x - left) as f32, (y - top) as f32))
}

/// What one pump changes on the compositor.
#[derive(Default)]
pub struct PortalScene {
    /// `Some(Some(modal))` shows the dialog, `Some(None)` clears it, and `None`
    /// leaves the modal as it is.
    pub modal: Option<Option<Modal>>,
    /// Replies the dispatcher writes.
    pub replies: Vec<Reply>,
}

impl PortalService {
    /// Routes the modal events the compositor queued and says what the modal must
    /// show now. The dialog is drawn centred on `output` while it is open. Keys and
    /// left clicks go to the dialog; a click outside it is swallowed. The caller ticks
    /// the clock first, so a deadline that passed has already settled.
    pub fn pump(&mut self, output: Option<(OutputId, Rect)>, locked: bool, events: &[ModalEvent]) -> PortalScene {
        if locked {
            let replies = self.set_locked(true);
            return PortalScene {
                modal: self.clear_modal(),
                replies,
            };
        }
        // A dialog never outgrows the output it sits on, so the output's size is the
        // bound the dialog fits within. It is set before placement so this frame's
        // pointer routing uses the same bounds the dialog was last drawn within.
        if let Some((_, rect)) = output {
            let width = usize::try_from(rect.width).unwrap_or(usize::MAX);
            let height = usize::try_from(rect.height).unwrap_or(usize::MAX);
            self.ui.set_bounds(width, height);
        }
        let placement = self.placement(output);
        for event in events {
            self.route(event, placement);
        }
        let replies = self.settle();
        let modal = match output {
            Some((output_id, rect)) if placement.is_some() => self.frame_modal(output_id, rect),
            _ => self.clear_modal(),
        };
        PortalScene { modal, replies }
    }

    /// The rectangle the open dialog's frame sits at on `output`, or `None` when no
    /// dialog is open.
    fn placement(&self, output: Option<(OutputId, Rect)>) -> Option<Rect> {
        let (_, rect) = output?;
        self.ui.active_request()?;
        let (width, height) = self.ui.size();
        let width = u32::try_from(width).ok()?;
        let height = u32::try_from(height).ok()?;
        Some(dialog_placement(rect, width, height))
    }

    fn route(&mut self, event: &ModalEvent, placement: Option<Rect>) {
        match *event {
            ModalEvent::Key { keycode, state } => {
                if state == KeyState::Pressed
                    && let Some(key) = screen_key(keycode)
                {
                    self.ui.key(key, false);
                }
            }
            ModalEvent::PointerButton { x, y, button, state } => {
                if button != BTN_LEFT {
                    return;
                }
                let Some(placement) = placement else {
                    return;
                };
                let Some((fx, fy)) = frame_point(placement, x, y) else {
                    return;
                };
                match state {
                    KeyState::Pressed => self.ui.pointer_press(fx, fy),
                    KeyState::Released => self.ui.pointer_release(fx, fy),
                }
            }
        }
    }

    /// Takes the frame the dialog just drew, if its pixels changed, and wraps it as the
    /// modal. `None` when nothing changed, so the modal stays as it is.
    fn frame_modal(&mut self, output: OutputId, rect: Rect) -> Option<Option<Modal>> {
        let frame = self.ui.take_frame()?;
        let width = u32::try_from(frame.width()).ok()?;
        let height = u32::try_from(frame.height()).ok()?;
        // The frame is placed from its own size, not the size the pointer was routed
        // with: a settled or resized dialog may have drawn at a new size in this take.
        let placement = dialog_placement(rect, width, height);
        let image = Image::new(Size::new(width, height), frame.width() * 4, frame.bytes().to_vec())?;
        let overlay = Overlay::new(placement, &image)?;
        self.shown = Some(output);
        Some(Some(Modal::new(output, overlay)))
    }

    /// Clears the modal once it was shown, and only then.
    fn clear_modal(&mut self) -> Option<Option<Modal>> {
        self.shown.take().map(|_| None)
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_model::Fields;

    use gnoblin_wayland::ProcessIdentity;

    use super::*;

    fn screenshot() -> PortalRequest {
        PortalRequest {
            interface: "Screenshot".to_owned(),
            caller: VerifiedCaller::new(0, "org.example.Requester"),
            prompt: "Share the screen?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
            parent_window: String::new(),
        }
    }

    #[test]
    fn locking_settles_the_open_request_as_locked() {
        let mut service = PortalService::new();
        let requester = ClientId::new(1);
        let opened = service.submit(requester, 1, &screenshot(), Tick::ZERO);
        assert!(
            opened.is_ok_and(|replies| replies.is_empty()),
            "the dialog is admitted before the lock"
        );

        let replies = service.set_locked(true);
        assert_eq!(replies.len(), 1, "the held request is settled once");
        assert_eq!(replies[0].0, requester, "the reply goes to the requester");
        assert_eq!(replies[0].1.error_code(), Some("locked"));

        assert!(service.set_locked(false).is_empty(), "unlocking settles nothing");
    }

    #[test]
    fn keycodes_map_to_dialog_keys() {
        assert_eq!(screen_key(1), Some(ScreenKey::Escape));
        assert_eq!(screen_key(15), Some(ScreenKey::Tab));
        assert_eq!(screen_key(28), Some(ScreenKey::Enter));
        assert_eq!(screen_key(96), Some(ScreenKey::Enter), "keypad enter answers too");
        assert_eq!(screen_key(57), Some(ScreenKey::Space));
        assert_eq!(screen_key(103), Some(ScreenKey::Up));
        assert_eq!(screen_key(108), Some(ScreenKey::Down));
        assert_eq!(screen_key(14), Some(ScreenKey::Backspace));
        assert_eq!(screen_key(42), None, "shift is not a dialog key");
        assert_eq!(screen_key(30), None, "letters are not dialog keys");
    }

    #[test]
    fn the_frame_is_centred_on_the_output() {
        let output = Rect::new(100, 50, 1920, 1080);
        assert_eq!(dialog_placement(output, 640, 360), Rect::new(740, 410, 640, 360));
    }

    #[test]
    fn a_frame_larger_than_the_output_sits_at_its_origin() {
        let output = Rect::new(0, 0, 320, 200);
        assert_eq!(dialog_placement(output, 640, 360), Rect::new(0, 0, 640, 360));
    }

    #[test]
    fn pointer_maps_into_the_frame_and_misses_outside() {
        let placement = Rect::new(740, 410, 640, 360);
        assert_eq!(frame_point(placement, 740.0, 410.0), Some((0.0, 0.0)));
        assert_eq!(frame_point(placement, 1000.5, 600.0), Some((260.5, 190.0)));
        assert_eq!(frame_point(placement, 1380.0, 600.0), None, "right edge is exclusive");
        assert_eq!(frame_point(placement, 739.9, 500.0), None);
        assert_eq!(frame_point(placement, 900.0, 770.0), None, "bottom edge is exclusive");
    }

    #[test]
    fn no_dialog_leaves_the_modal_alone() {
        let mut service = PortalService::new();
        let scene = service.pump(Some((OutputId::new(1), Rect::new(0, 0, 1920, 1080))), false, &[]);
        assert!(scene.modal.is_none(), "nothing to show and nothing to clear");
        assert!(scene.replies.is_empty());
    }

    #[test]
    fn a_lock_clears_the_shown_modal_and_settles_the_request() {
        let mut service = PortalService::new();
        let requester = ClientId::new(1);
        let output = OutputId::new(1);
        let rect = Rect::new(0, 0, 1920, 1080);
        assert!(service.submit(requester, 1, &screenshot(), Tick::ZERO).is_ok());
        let shown = service.pump(Some((output, rect)), false, &[]);
        assert!(
            matches!(shown.modal, Some(Some(_))),
            "the dialog is drawn once it is open"
        );

        let locked = service.pump(Some((output, rect)), true, &[]);
        assert!(matches!(locked.modal, Some(None)), "the lock clears the modal");
        assert_eq!(locked.replies.len(), 1);
        assert_eq!(locked.replies[0].1.error_code(), Some("locked"));
    }

    /// A process identity for a connected client, with a fixed start time.
    fn process(pid: u32) -> WindowProcess {
        WindowProcess::Connected(ProcessIdentity { pid, start_time: 7 })
    }

    #[test]
    fn the_owner_check_accepts_a_window_owned_by_the_bus_pid() {
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert_eq!(parent_owner_check(&caller, &process(1000)), Ok(()));
    }

    #[test]
    fn a_payload_claiming_another_pid_is_checked_against_the_bus_pid() {
        // The relay took pid 1000 from the bus. A payload that names a window owned by
        // pid 4242 claims that pid as its own, but the check reads only the bus pid, so
        // the window is refused: it belongs to another process than the caller.
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert_eq!(
            parent_owner_check(&caller, &process(4242)),
            Err("the parent window belongs to another process"),
        );
    }

    #[test]
    fn an_unknown_bus_pid_never_matches_a_window() {
        // Pid 0 means the bus could not name the caller, so no window is proven theirs.
        let caller = VerifiedCaller::new(0, "");
        assert!(parent_owner_check(&caller, &process(0)).is_err());
        assert!(parent_owner_check(&caller, &process(1000)).is_err());
    }

    #[test]
    fn a_window_with_no_nameable_or_live_process_is_refused() {
        let caller = VerifiedCaller::new(1000, "org.example.Requester");
        assert!(parent_owner_check(&caller, &WindowProcess::Unknown).is_err());
        assert!(parent_owner_check(&caller, &WindowProcess::Disconnected).is_err());
    }

    #[test]
    fn the_relay_frame_carries_the_bus_pid_the_check_reads() {
        // The relay writes the bus pid into the frame; decoding returns that same value as
        // the caller, so the check and the relay agree on one number.
        let request = PortalRequest {
            interface: "Screenshot".to_owned(),
            caller: VerifiedCaller::new(1000, "org.example.Requester"),
            prompt: "Share the screen?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
            parent_window: "wayland:abc".to_owned(),
        };
        let frame = gnoblin_client::wire::portal_ui(1, &request);
        let payload = frame.payload().expect("the request carries a payload");
        let decoded = gnoblin_client::wire::read_portal_ui(payload).expect("decodes");
        assert_eq!(decoded.caller, request.caller);
        assert_eq!(decoded.caller.pid(), 1000);
    }
}
