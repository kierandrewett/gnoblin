//! `gnoblin.dialogs` on the Rust side: the shell's claim on `dialogs.observe` and the verdicts it gives.
//!
//! The compositor draws its own consent and passphrase dialogs. A shell that claims the `dialogs.observe` slot is
//! shown each request for a short window (250 ms on the compositor's side) before the dialog opens, and
//! may refuse it by policy with the answer `deny`. Anything else, and silence, leaves the request to the built-in
//! dialog. The slot offers no way to allow a request and never carries a passphrase, so a script cannot do either:
//! this module sends the two fixed byte strings `deny` and `pass` and nothing a script wrote.
//!
//! The script's side is `data/js/gnoblin/dialogs.ts`: listeners run as queued calls under the usual per-call
//! budget, and the frame hands what it decided back here as verdicts. The slot is claimed while the running
//! generation has a listener and released when it has none, when it is retired (a reload or a shutdown), and by the
//! compositor when the connection closes, so a crashed shell leaves no observer behind.

use gnoblin_tree_client::slot_link::{SlotEvent, SlotLink};
use rquickjs::{Array, Ctx, Object};

gnoblin_log::logger!(log = "js");

/// The slot a shell claims to observe the built-in dialogs.
pub(crate) const SLOT: &str = "dialogs.observe";

/// The verdict that refuses a request by policy.
const DENY: &[u8] = b"deny";

/// The verdict of a listener that raised no objection. The compositor reads anything but `deny` as that; the
/// word is for the log.
const PASS: &[u8] = b"pass";

/// What a script decided about one observed request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JsVerdict {
    pub request: u64,
    /// A listener declined it.
    pub deny: bool,
    /// The reason the listener gave, for the log.
    pub reason: String,
}

/// The shell's slot connection and what it holds on it.
pub(crate) struct JsDialogs {
    link: Option<Box<dyn SlotLink>>,
    /// Whether this connection holds `dialogs.observe`.
    claimed: bool,
    /// Requests the compositor offered that the shell has not answered yet.
    held: Vec<u64>,
    /// A claim was refused, or there is no connection to claim on, and that has been reported: it is not tried
    /// again (and not reported again) until the generation has no listener, or a new one takes over.
    gave_up: bool,
}

impl JsDialogs {
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

    /// Claims the slot while the running generation has a listener and gives it back when it has none.
    pub(crate) fn sync(&mut self, wanted: bool, diagnostics: &mut Vec<String>) {
        if !wanted {
            self.gave_up = false;
        }
        if wanted == self.claimed || self.gave_up {
            return;
        }
        let Some(link) = self.link.as_mut() else {
            self.gave_up = true;
            diagnostics.push(
                "the shell has no slot connection, so dialog listeners never run; the built-in dialogs serve every \
                 request"
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
                        "{SLOT} was not claimed, so dialog listeners never run: {error}"
                    ));
                }
            }
        } else {
            self.claimed = false;
            if let Err(error) = link.release_slot(SLOT) {
                log::debug!(area = "dialogs", "{SLOT} was not released: {error}");
            }
        }
    }

    /// What the compositor sent on the slot connection since the last frame, for the requests of the slot this
    /// shell observes. A lost connection is dropped and reported once, and passed on as [`SlotEvent::Lost`]: the
    /// compositor released the claim with it and the built-in dialogs serve from here on.
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
                // Reported here, so the missing connection is not reported again.
                self.gave_up = true;
                self.claimed = false;
                self.held.clear();
                diagnostics.push(format!(
                    "the slot connection is lost ({error}); the built-in dialogs serve every request again"
                ));
                vec![SlotEvent::Lost]
            }
        }
    }

    /// Gives the compositor the verdict: the request is claimed and answered `deny` or `pass`. A request that has
    /// already ended is not an error: the observation window is short.
    pub(crate) fn answer(&mut self, verdict: &JsVerdict) {
        if !self.held.contains(&verdict.request) {
            return;
        }
        self.held.retain(|held| *held != verdict.request);
        let Some(link) = self.link.as_mut() else {
            return;
        };
        if verdict.deny {
            log::info!(
                area = "dialogs",
                request = verdict.request,
                reason = %verdict.reason,
                "a dialog listener declined a request by policy"
            );
        }
        let answer = if verdict.deny { DENY } else { PASS };
        if let Err(error) = link
            .claim_request(verdict.request)
            .and_then(|()| link.answer(verdict.request, answer))
        {
            log::debug!(
                area = "dialogs",
                request = verdict.request,
                "the verdict was not delivered: {error}"
            );
        }
    }

    /// Ends the generation's observation: each request it was shown and has not answered is answered `pass`, and
    /// the slot is released. The next generation claims it again when it has a listener.
    pub(crate) fn retire(&mut self) {
        self.hand_over();
        let Some(link) = self.link.as_mut() else {
            return;
        };
        if self.claimed {
            self.claimed = false;
            if let Err(error) = link.release_slot(SLOT) {
                log::debug!(area = "dialogs", "{SLOT} was not released: {error}");
            }
        }
    }

    /// Ends the generation's observation for a reload: each request it was shown and has not answered is answered
    /// `pass`, and the slot stays claimed. The generation that takes over runs `sync` in its first frame, which keeps
    /// the claim when it has a dialog listener and releases it when it has none, so the compositor never offers a
    /// request to nobody in between: a request that came while the slot was released would be served by the built-in
    /// dialog without the new listener seeing it.
    pub(crate) fn hand_over(&mut self) {
        let held = std::mem::take(&mut self.held);
        self.gave_up = false;
        let Some(link) = self.link.as_mut() else {
            return;
        };
        for request in held {
            if let Err(error) = link.claim_request(request).and_then(|()| link.answer(request, PASS)) {
                log::debug!(area = "dialogs", request, "the pass was not delivered: {error}");
            }
        }
    }
}

/// The slot events as script objects, for the module's `dialogs` entry.
pub(crate) fn events_to_js<'js>(ctx: &Ctx<'js>, events: &[SlotEvent]) -> rquickjs::Result<Array<'js>> {
    let list = Array::new(ctx.clone())?;
    for (index, event) in events.iter().enumerate() {
        let item = Object::new(ctx.clone())?;
        match event {
            SlotEvent::Offered {
                request,
                deadline,
                title,
                message,
                subject,
                ..
            } => {
                item.set("kind", "offered")?;
                item.set("request", *request as f64)?;
                item.set("deadline", *deadline as f64)?;
                item.set("title", title.as_str())?;
                item.set("message", message.as_str())?;
                item.set("subject", String::from_utf8_lossy(subject).into_owned())?;
            }
            SlotEvent::Finished { request, completion } => {
                item.set("kind", "finished")?;
                item.set("request", *request as f64)?;
                item.set("completion", *completion)?;
            }
            SlotEvent::Lost => item.set("kind", "lost")?,
        }
        list.set(index, item)?;
    }
    Ok(list)
}

/// The verdicts a script's frame produced, as the module's `takeVerdicts` returned them.
pub(crate) fn verdicts_from_js(list: &Array<'_>) -> rquickjs::Result<Vec<JsVerdict>> {
    let mut verdicts = Vec::new();
    for item in list.iter::<Object<'_>>() {
        let item = item?;
        let request: f64 = item.get("request")?;
        verdicts.push(JsVerdict {
            request: request as u64,
            deny: item.get("deny")?,
            reason: item.get("reason")?,
        });
    }
    Ok(verdicts)
}
