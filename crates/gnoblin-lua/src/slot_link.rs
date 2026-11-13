//! The shell's second connection to the compositor: the slots it claims and the requests it is offered.
//!
//! The tree socket carries the `tree` namespace only. A slot is claimed, and a request offered to it is
//! claimed, answered or declined, on the `gnoblin` namespace of the owner socket
//! (design/rust-compositor.md, "Providers and slots"). The shell VM therefore holds a second owner
//! connection, a [`SlotLink`], beside its tree link. Lua reaches it through five calls on the `core` table
//! (`slot_claim`, `slot_release`, `request_claim`, `request_answer`, `request_decline`) and receives what the
//! compositor sends as the fifth argument of the tick (`offers`), both wrapped by the `slots` global of
//! `data/lua/gnoblin/node.lua`.
//!
//! An answer may be a passphrase. It crosses this module as bytes that are wiped after the send, are never
//! logged, and are never part of an error this module builds.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_model::{DeclineReason, Provider, RequestCompletion};
use gnoblin_secret::SecretText;
use gnoblin_types::RequestId;
use mlua::{Lua, Table};

use crate::requests::parse_reason;
use gnoblin_tree_client::tree_mirror::LinkError;

/// How long a poll waits for bytes that have not arrived. Short, so a frame never stalls on it.
const READ_WAIT: Duration = Duration::from_millis(1);

/// How long a request waits for the compositor's reply. The compositor answers a slot or request call at
/// once, so a reply later than this is a compositor that is stuck.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// Something the compositor sent on the slot connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotEvent {
    /// A request was offered to a slot this connection claimed.
    Offered {
        request: u64,
        /// When the compositor takes the request back, on its own clock in milliseconds.
        deadline: u64,
        slot: String,
        title: String,
        message: String,
        /// What the request is about, as the offering client chose it: for `portal.ui`, the app id the portal
        /// backend verified.
        subject: Vec<u8>,
    },
    /// A request ended: answered, ended by its requester, refused, or timed out.
    Finished {
        request: u64,
        /// `answered`, `ended`, `refused` or `timed-out`.
        completion: &'static str,
    },
    /// The connection is gone. The compositor released every claim and ended every offer with it, and the
    /// built-in dialogs serve from here on. A link never sends this; the VM does when `poll` fails.
    Lost,
}

/// The shell's connection for slots and requests.
pub trait SlotLink {
    /// Claim `slot` as the shell. A refusal names why (another holder, not the owner's connection).
    fn claim_slot(&mut self, slot: &str) -> Result<(), LinkError>;
    /// Give a claimed slot back.
    fn release_slot(&mut self, slot: &str) -> Result<(), LinkError>;
    /// Claim an offered request, so only this connection answers it.
    fn claim_request(&mut self, request: u64) -> Result<(), LinkError>;
    /// Answer a claimed request. `answer` is the slot's own encoding: `allow` or `deny` for `portal.ui`, the
    /// passphrase for `prompt.secret`.
    fn answer(&mut self, request: u64, answer: &[u8]) -> Result<(), LinkError>;
    /// Decline a claimed request: this shell will not answer it, so the compositor serves it another way. `reason`
    /// says why, as the requester sees it; `None` is a plain decline.
    fn decline(&mut self, request: u64, reason: Option<DeclineReason>) -> Result<(), LinkError>;
    /// Everything that has arrived, oldest first. An error is the loss of the connection.
    fn poll(&mut self) -> Result<Vec<SlotEvent>, LinkError>;
}

/// The slot link over a handshaken compositor connection.
pub struct ClientSlotLink<T: Transport> {
    client: Client<T>,
}

impl<T: Transport> ClientSlotLink<T> {
    /// Wrap a client whose handshake is done.
    pub fn new(client: Client<T>) -> Result<Self, LinkError> {
        let mut link = Self { client };
        link.wait(READ_WAIT)?;
        Ok(link)
    }

    fn wait(&mut self, timeout: Duration) -> Result<(), LinkError> {
        self.client
            .set_read_timeout(Some(timeout))
            .map_err(|error| LinkError(format!("the slot connection refuses a read deadline: {error}")))
    }

    /// Runs `call` with the long reply deadline, then puts the short poll deadline back.
    fn replying<R>(&mut self, call: impl FnOnce(&mut Client<T>) -> Result<R, ClientError>) -> Result<R, LinkError> {
        self.wait(REPLY_WAIT)?;
        let result = call(&mut self.client);
        self.wait(READ_WAIT)?;
        result.map_err(refusal)
    }
}

impl ClientSlotLink<UnixTransport> {
    /// Connect to the compositor's owner socket at `path` and complete the handshake.
    pub fn connect(path: &std::path::Path, version: gnoblin_types::Version) -> Result<Self, LinkError> {
        let transport = UnixTransport::connect(path)
            .map_err(|error| LinkError(format!("cannot connect to {}: {error}", path.display())))?;
        let client = Client::connect(transport, version).map_err(|error| LinkError(error.to_string()))?;
        Self::new(client)
    }
}

/// The message a failed call carries to Lua: the schema's code first, so a script can match on it. The
/// compositor's refusal text never carries an answer, and an error built here never has one either.
fn refusal(error: ClientError) -> LinkError {
    match error {
        ClientError::Refused { code, message, .. } => LinkError(format!("{code}: {message}")),
        other => LinkError(other.to_string()),
    }
}

impl<T: Transport> SlotLink for ClientSlotLink<T> {
    fn claim_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.replying(|client| client.claim_slot(slot, Provider::Shell))
    }

    fn release_slot(&mut self, slot: &str) -> Result<(), LinkError> {
        self.replying(|client| client.release_slot(slot))
    }

    fn claim_request(&mut self, request: u64) -> Result<(), LinkError> {
        self.replying(|client| client.claim(RequestId::new(request)))
    }

    fn answer(&mut self, request: u64, answer: &[u8]) -> Result<(), LinkError> {
        self.replying(|client| client.answer(RequestId::new(request), answer))
    }

    fn decline(&mut self, request: u64, reason: Option<DeclineReason>) -> Result<(), LinkError> {
        self.replying(|client| client.decline_with(RequestId::new(request), reason))
    }

    fn poll(&mut self) -> Result<Vec<SlotEvent>, LinkError> {
        let mut events = Vec::new();
        loop {
            match self.client.next_event() {
                Ok(Event::RequestOffered {
                    request,
                    deadline,
                    slot,
                    title,
                    message,
                    subject,
                }) => events.push(SlotEvent::Offered {
                    request: request.get(),
                    deadline: deadline.get(),
                    slot,
                    title,
                    message,
                    subject,
                }),
                Ok(Event::RequestFinished { request, completion }) => events.push(SlotEvent::Finished {
                    request: request.get(),
                    completion: completion_name(&completion),
                }),
                // The shell is neither a host that offers requests nor a subscriber to the graph.
                Ok(Event::RequestAnswered { .. } | Event::Model(_) | Event::Lock(_)) => {}
                Err(ClientError::Idle) => return Ok(events),
                Err(error) => return Err(refusal(error)),
            }
        }
    }
}

fn completion_name(completion: &RequestCompletion) -> &'static str {
    match completion {
        RequestCompletion::Answered => "answered",
        RequestCompletion::Ended => "ended",
        RequestCompletion::Refused(_) => "refused",
        RequestCompletion::TimedOut => "timed-out",
    }
}

/// The slot link the shell VM holds, shared with the Lua functions that call it. `None` until a link is set, and
/// again once the connection is lost.
pub type SharedSlots = Rc<RefCell<Option<Box<dyn SlotLink>>>>;

fn no_link() -> mlua::Error {
    mlua::Error::RuntimeError("no-slot-connection: the shell has no connection to the compositor for slots".to_owned())
}

fn lua_error(error: LinkError) -> mlua::Error {
    mlua::Error::RuntimeError(error.0)
}

/// Puts `slot_claim`, `slot_release`, `request_claim`, `request_answer` and `request_decline` on the `core` table.
/// Each one is a call into the shared link, and fails with the compositor's code when it refuses.
pub(crate) fn install(lua: &Lua, core: &Table, slots: &SharedSlots) -> mlua::Result<()> {
    fn request_id(id: i64) -> mlua::Result<u64> {
        u64::try_from(id).map_err(|_| mlua::Error::RuntimeError("a request id is not negative".to_owned()))
    }

    let shared = Rc::clone(slots);
    core.set(
        "slot_claim",
        lua.create_function(move |_, name: String| {
            let mut link = shared.borrow_mut();
            link.as_mut().ok_or_else(no_link)?.claim_slot(&name).map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "slot_release",
        lua.create_function(move |_, name: String| {
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .release_slot(&name)
                .map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_claim",
        lua.create_function(move |_, id: i64| {
            let id = request_id(id)?;
            let mut link = shared.borrow_mut();
            link.as_mut().ok_or_else(no_link)?.claim_request(id).map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_answer",
        lua.create_function(move |_, (id, answer): (i64, mlua::LuaString)| {
            let id = request_id(id)?;
            // The text is copied out of Lua for the send and wiped when `secret` drops. Lua's own copy lives
            // until its collector frees it, as the lock screen's password does.
            let text = answer
                .to_str()
                .map_err(|_| mlua::Error::RuntimeError("an answer is UTF-8 text".to_owned()))?;
            let secret = SecretText::new(text.to_string());
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .answer(id, secret.expose().as_bytes())
                .map_err(lua_error)
        })?,
    )?;
    let shared = Rc::clone(slots);
    core.set(
        "request_decline",
        lua.create_function(move |_, (id, reason): (i64, Option<String>)| {
            let id = request_id(id)?;
            let reason = reason.as_deref().map(parse_reason).transpose()?;
            let mut link = shared.borrow_mut();
            link.as_mut()
                .ok_or_else(no_link)?
                .decline(id, reason)
                .map_err(lua_error)
        })?,
    )?;
    Ok(())
}
