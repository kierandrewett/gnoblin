//! `gnoblin.authentication` on the Rust side: the shell's claim on the `auth.polkit` slot and the answers it gives.
//!
//! When polkit needs an authentication agent the services role offers the request to the slot's provider. A shell that
//! claims the slot is that provider: it is offered each request, shows its own prompt and answers. The answer this slot
//! reads is the identity the user authenticated as (any non-empty UTF-8 text); a decline is a refusal. This module
//! sends nothing else, and it never sends an answer for a request the script has not settled.
//!
//! The script's side is `data/js/gnoblin/authentication.ts`. The slot is claimed while the running generation has a
//! provider and released when it has none, when it is retired (a reload or a shutdown), and by the compositor when the
//! connection closes, so a crashed shell leaves no provider behind and the services role's agent serves again.
//! A request the shell holds when it is retired is declined, so polkit's caller is not left to wait for the deadline.

use gnoblin_model::DeclineReason;
use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink, parse_decline_reason};
use rquickjs::{Array, Object};

gnoblin_log::logger!(log = "js");

/// The slot a shell claims to be the session's authentication agent.
pub(crate) const SLOT: &str = "auth.polkit";

/// The longest identity an answer may carry, in bytes.
const MAX_IDENTITY: usize = 256;

/// How the script settled one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JsAuthReply {
    /// The user authenticated as this identity.
    Allow { request: u64, identity: String },
    /// The provider refused.
    Decline { request: u64, reason: DeclineReason },
}

impl JsAuthReply {
    fn request(&self) -> u64 {
        match self {
            Self::Allow { request, .. } | Self::Decline { request, .. } => *request,
        }
    }
}

/// The shell's connection for the authentication slot, and what it holds on it.
pub(crate) struct JsAuth {
    link: Option<Box<dyn SlotLink>>,
    /// Whether this connection holds the slot.
    claimed: bool,
    /// Requests the compositor offered that the script has not settled yet.
    held: Vec<u64>,
    /// A claim was refused, or there is no connection to claim on, and that has been reported: it is not tried
    /// again (and not reported again) until the generation has no provider, or a new one takes over.
    gave_up: bool,
}

impl JsAuth {
    pub(crate) fn new() -> Self {
        Self {
            link: None,
            claimed: false,
            held: Vec::new(),
            gave_up: false,
        }
    }

    pub(crate) fn set_link(&mut self, link: Box<dyn SlotLink>) {
        self.link = Some(link);
    }

    /// Whether the shell holds the slot or a request: the frame must keep running its side of the exchange.
    pub(crate) fn is_active(&self) -> bool {
        self.claimed || !self.held.is_empty()
    }

    /// Claims the slot while the running generation has a provider and gives it back when it has none. Giving it back
    /// declines what the shell still holds first: the compositor ends them when the slot changes hands in any case.
    pub(crate) fn sync(&mut self, wanted: bool, diagnostics: &mut Vec<String>) {
        if !wanted {
            self.gave_up = false;
        }
        if wanted == self.claimed || self.gave_up {
            return;
        }
        if !wanted {
            self.decline_held();
        }
        let Some(link) = self.link.as_mut() else {
            self.gave_up = true;
            diagnostics.push(
                "the shell has no slot connection for authentication, so the provider never hears a request; the \
                 built-in agent serves them"
                    .to_owned(),
            );
            return;
        };
        if wanted {
            match link.claim_slot(SLOT) {
                Ok(()) => self.claimed = true,
                Err(error) => {
                    self.gave_up = true;
                    diagnostics.push(format!(
                        "{SLOT} was not claimed, so the authentication provider never hears a request: {error}"
                    ));
                }
            }
        } else {
            self.claimed = false;
            if let Err(error) = link.release_slot(SLOT) {
                log::debug!(area = "auth", "{SLOT} was not released: {error}");
            }
        }
    }

    /// What the compositor sent on the connection since the last frame, for the requests of the slot. A lost
    /// connection is dropped and reported once, and passed on as [`SlotEvent::Lost`]: the compositor released the claim
    /// with it and the built-in agent serves from here on.
    pub(crate) fn poll(&mut self, diagnostics: &mut Vec<String>) -> Vec<SlotEvent> {
        let Some(link) = self.link.as_mut() else {
            return Vec::new();
        };
        match link.poll() {
            Ok(events) => {
                let mut seen = Vec::with_capacity(events.len());
                for event in events {
                    match &event {
                        SlotEvent::Offered { slot, request, .. } if slot == SLOT => self.held.push(*request),
                        // A request of a slot this shell never claimed is not its to answer.
                        SlotEvent::Offered { .. } => continue,
                        SlotEvent::Finished { request, .. } => self.held.retain(|held| held != request),
                        SlotEvent::Lost => {}
                    }
                    seen.push(event);
                }
                seen
            }
            Err(error) => {
                self.link = None;
                self.gave_up = true;
                self.claimed = false;
                self.held.clear();
                diagnostics.push(format!(
                    "the authentication connection is lost ({error}); the built-in agent serves requests again"
                ));
                vec![SlotEvent::Lost]
            }
        }
    }

    /// Gives the compositor the script's answer: the request is claimed and answered or declined. A request that has
    /// already ended is not an error: the requester may have withdrawn it while the user was reading the prompt.
    pub(crate) fn reply(&mut self, reply: &JsAuthReply, diagnostics: &mut Vec<String>) {
        let request = reply.request();
        if !self.held.contains(&request) {
            return;
        }
        self.held.retain(|held| *held != request);
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let sent = match reply {
            JsAuthReply::Allow { identity, .. } => {
                if identity.is_empty() || identity.len() > MAX_IDENTITY {
                    diagnostics.push(format!(
                        "the identity of request {request} is {} bytes; an answer carries 1 to {MAX_IDENTITY}, so the \
                         request is declined",
                        identity.len()
                    ));
                    link.claim_request(request)
                        .and_then(|()| link.decline(request, Some(DeclineReason::Denied)))
                } else {
                    log::info!(area = "auth", request, "the authentication provider allowed a request");
                    link.claim_request(request)
                        .and_then(|()| link.answer(request, identity.as_bytes()))
                }
            }
            JsAuthReply::Decline { reason, .. } => {
                log::info!(area = "auth", request, "the authentication provider declined a request");
                link.claim_request(request)
                    .and_then(|()| link.decline(request, Some(*reason)))
            }
        };
        if let Err(error) = sent {
            log::debug!(area = "auth", request, "the answer was not delivered: {error}");
        }
    }

    /// Declines each request the shell holds and has not answered, so polkit's caller learns now.
    fn decline_held(&mut self) {
        let held = std::mem::take(&mut self.held);
        let Some(link) = self.link.as_mut() else {
            return;
        };
        for request in held {
            if let Err(error) = link
                .claim_request(request)
                .and_then(|()| link.decline(request, Some(DeclineReason::Unavailable)))
            {
                log::debug!(area = "auth", request, "the decline was not delivered: {error}");
            }
        }
    }

    /// Ends the generation's provider: each request it was offered and has not answered is declined, and the slot is
    /// released. The next generation claims it again when it has a provider.
    pub(crate) fn retire(&mut self) {
        self.decline_held();
        if self.claimed {
            self.claimed = false;
            if let Some(link) = self.link.as_mut() {
                if let Err(error) = link.release_slot(SLOT) {
                    log::debug!(area = "auth", "{SLOT} was not released: {error}");
                }
            }
        }
        self.gave_up = false;
    }
}

/// The replies a script's frame produced, as the module's `takeAuthenticationReplies` returned them.
pub(crate) fn replies_from_js(list: &Array<'_>) -> rquickjs::Result<Vec<Result<JsAuthReply, String>>> {
    let mut replies = Vec::new();
    for item in list.iter::<Object<'_>>() {
        let item = item?;
        let request: f64 = item.get("request")?;
        let kind: String = item.get("kind")?;
        let request = request as u64;
        replies.push(match kind.as_str() {
            "allow" => Ok(JsAuthReply::Allow {
                request,
                identity: item.get("identity")?,
            }),
            _ => {
                let reason: String = item.get("reason")?;
                parse_decline_reason(&reason).map(|reason| JsAuthReply::Decline { request, reason })
            }
        });
    }
    Ok(replies)
}
