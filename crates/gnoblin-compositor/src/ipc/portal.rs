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
//!
//! # Failure modes
//!
//! - Request id already held by an open or queued request: refused `request-taken`.
//! - `portal.cancel` from a connection that holds nothing: refused `request-unknown`.
//! - `portal.key` naming a key the dialog grammar does not know: refused
//!   `unknown-key`, and nothing reaches the dialog.
//! - Admission refused by the queue or the lock: settles `error(busy|locked)` to the
//!   submitter, not a refusal, so the requester learns the verdict it waits for.

use gnoblin_client::wire::{self, PortalRequest};
use gnoblin_diag::portal::{Consent, Dialog, Grant, Interface, ScreenKey};
use gnoblin_proto::Envelope;
use gnoblin_types::{ClientId, RequestId, Tick};

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
    pub code: &'static str,
    pub message: String,
}

/// The connection that submitted an admitted request, and the envelope id its
/// outcome is sent under.
struct HeldRequest {
    request: RequestId,
    client: ClientId,
    envelope_id: u64,
}

/// The portal UI session and the routing of its requests to their owners.
pub struct PortalService {
    ui: PortalUi,
    owners: Vec<HeldRequest>,
}

impl Default for PortalService {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalService {
    #[must_use]
    pub fn new() -> Self {
        Self {
            ui: PortalUi::new(FRAME_WIDTH, FRAME_HEIGHT),
            owners: Vec::new(),
        }
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
        let dialog = Dialog::Consent(Consent::open(request.app.clone(), request.prompt.clone(), now));
        match self
            .ui
            .submit(id, &request.app, interface, dialog, request.timeout_ms, now)
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

    /// `portal.key`: route one key to the open dialog.
    pub fn key(&mut self, name: &str, repeat: bool) -> Result<Vec<Reply>, PortalRefusal> {
        let Some(key) = parse_key(name) else {
            return Err(PortalRefusal {
                code: UNKNOWN_KEY,
                message: format!("`{name}` is not a key the portal dialog takes"),
            });
        };
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
    /// No session-lock state source feeds this yet (a documented follow-up), so the
    /// lock path is exercised by this module's unit tests only.
    #[allow(dead_code, reason = "no lock-state source yet; exercised by unit tests")]
    pub fn set_locked(&mut self, locked: bool) -> Vec<Reply> {
        self.ui.set_locked(locked);
        self.settle()
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

#[cfg(test)]
mod tests {
    use gnoblin_model::Fields;
    use gnoblin_types::ClientId;

    use super::*;

    fn screenshot() -> PortalRequest {
        PortalRequest {
            interface: "Screenshot".to_owned(),
            app: "org.example.Requester".to_owned(),
            prompt: "Share the screen?".to_owned(),
            timeout_ms: 30_000,
            options: Fields::default(),
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
}
