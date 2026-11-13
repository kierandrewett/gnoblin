//! The shell's second connection to the compositor: the slots it claims and the requests it is offered.
//!
//! The tree socket carries the `tree` namespace only. A slot is claimed, and a request offered to it is
//! claimed, answered or declined, on the `gnoblin` namespace of the owner socket
//! (design/rust-compositor.md, "Providers and slots"). The shell VM therefore holds a second owner
//! connection, a [`SlotLink`], beside its tree link. A script runtime drives it through five calls
//! (`claim_slot`, `release_slot`, `claim_request`, `answer`, `decline`) and receives what the compositor
//! sends as [`SlotEvent`]s. Each runtime wraps those in its own vocabulary.
//!
//! An answer may be a passphrase. It crosses this module as bytes that are wiped after the send, are never
//! logged, and are never part of an error this module builds.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_model::{DeclineReason, Provider, RequestCompletion};
use gnoblin_types::RequestId;

use crate::tree_mirror::LinkError;

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
    /// built-in dialogs serve from here on. A link never sends this; the runtime does when `poll` fails.
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

/// Reads the wire name of a decline reason: `locked`, `denied` or `unavailable`. The error names the three
/// accepted spellings, so a runtime can pass it to its caller as it stands.
pub fn parse_decline_reason(name: &str) -> Result<DeclineReason, String> {
    match name {
        "locked" => Ok(DeclineReason::Locked),
        "denied" => Ok(DeclineReason::Denied),
        "unavailable" => Ok(DeclineReason::Unavailable),
        other => Err(format!(
            "a decline reason is `locked`, `denied` or `unavailable`, found `{other}`"
        )),
    }
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

/// The message a failed call carries: the schema's code first, so a caller can match on it. The
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

/// The slot link a runtime holds, shared with the functions that call it. `None` until a link is set, and
/// again once the connection is lost.
pub type SharedSlots = Rc<RefCell<Option<Box<dyn SlotLink>>>>;
