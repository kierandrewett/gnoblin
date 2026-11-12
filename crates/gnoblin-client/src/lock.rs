//! The session lock as a client sees it: the compositor's `lock` node on the tree.
//!
//! A service that reads or takes the lock (the screensaver, the compat adapters, `gnoblinctl status`) is
//! an *observer* of the tree, not its holder: it attaches in observer mode, reads the lock node's `state`
//! from the snapshot, and follows the change batches. The only call an observer may make is `lock`, and it
//! has no answer: the node's `state` is it (design/node-core.md, "The `Lock` class and the old `lock`
//! object").
//!
//! # Failure modes first
//!
//! 1. **The tree has no lock node.** [`Client::watch_lock`] is refused `not-found` rather than reading
//!    "not locked": a caller that guarded something on the lock must not take silence for an open desktop.
//! 2. **A state this build does not know.** Read as [`SessionLock::Locked`], the covered answer: the lock
//!    can only have grown a state, and the embargo errs towards covered.
//! 3. **A refused `lock` call.** Returned as [`ClientError::Refused`] with the compositor's code; the
//!    caller does not treat a queued request as a protected screen.

use gnoblin_proto::Envelope;
use gnoblin_proto::tree::change;
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{AttachArgs, ChangesPayload, SnapshotPayload, TransactArgs};

use crate::error::{ClientError, WireError};

/// The tree's namespace.
pub const TREE: &str = "tree";

/// The lock node's `state`: whether the session lock is holding the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLock {
    /// Nothing is hidden: the session is fully unlocked.
    NotLocked,
    /// A lock was taken and the cover is not yet proven on every output.
    Pending,
    /// The cover is up on every output.
    Locked,
}

impl SessionLock {
    /// The state the lock node's `state` text names. An unknown name reads as [`SessionLock::Locked`].
    #[must_use]
    pub fn from_name(name: &str) -> SessionLock {
        match name {
            "not-locked" => SessionLock::NotLocked,
            "pending" => SessionLock::Pending,
            _ => SessionLock::Locked,
        }
    }

    /// The state's name as the lock node's `state` property spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionLock::NotLocked => "not-locked",
            SessionLock::Pending => "pending",
            SessionLock::Locked => "locked",
        }
    }

    /// Whether anything is hidden: true from the moment a lock is taken until the cover is gone. This is
    /// `org.gnome.ScreenSaver.GetActive`, so a caller that misses a change cannot read "unlocking" as an open desktop.
    #[must_use]
    pub const fn covers(self) -> bool {
        !matches!(self, SessionLock::NotLocked)
    }
}

/// What a client has read of the lock node.
#[derive(Debug, Default)]
pub(crate) struct LockView {
    /// The lock node's id, once the snapshot named it.
    node: Option<u64>,
    /// The serial of the last snapshot or change batch read.
    serial: u64,
    /// The state the node last had.
    state: Option<SessionLock>,
    /// Whether the last snapshot chunk arrived.
    complete: bool,
    /// Whether the client attached.
    attached: bool,
}

impl LockView {
    /// The state, if the node was read.
    pub(crate) const fn state(&self) -> Option<SessionLock> {
        self.state
    }

    /// Folds one tree event. Returns the new state when the lock node's `state` changed.
    pub(crate) fn fold(&mut self, envelope: &Envelope) -> Result<Option<SessionLock>, ClientError> {
        let member = envelope.member.as_str();
        match member {
            "snapshot" => {
                let chunk = decode::<SnapshotPayload>(envelope, member)?;
                self.serial = chunk.serial;
                for node in &chunk.nodes {
                    if node.class == "lock" {
                        self.node = Some(node.id);
                        // A property equal to its schema default is not listed: absence is `not-locked`.
                        let name = node
                            .props
                            .iter()
                            .find(|prop| prop.key == "state")
                            .map_or("not-locked", |prop| prop.value_text.as_str());
                        self.state = Some(SessionLock::from_name(name));
                    }
                }
                self.complete |= chunk.last;
                Ok(None)
            }
            "changes" => {
                let batch = decode::<ChangesPayload>(envelope, member)?;
                self.serial = batch.serial;
                let mut changed = None;
                for entry in &batch.changes {
                    changed = self.fold_change(entry).or(changed);
                }
                Ok(changed)
            }
            _ => Ok(None),
        }
    }

    fn fold_change(&mut self, entry: &change::Properties) -> Option<SessionLock> {
        match entry.kind.as_str() {
            "created" if entry.class == "lock" => {
                self.node = Some(entry.node);
                self.state = Some(SessionLock::NotLocked);
                None
            }
            "set" if Some(entry.node) == self.node && entry.key == "state" => {
                let state = SessionLock::from_name(&entry.value_text);
                (self.state.replace(state) != Some(state)).then_some(state)
            }
            _ => None,
        }
    }
}

fn decode<M: prost::Message + Default>(envelope: &Envelope, member: &str) -> Result<M, ClientError> {
    envelope.payload_as::<M>().ok_or_else(|| ClientError::Protocol {
        member: format!("{TREE}.{member}"),
        reason: WireError::Truncated,
    })
}

impl LockView {
    /// The node id and base serial a `lock` call is built on, once attached.
    pub(crate) fn target(&self) -> Option<(u64, u64)> {
        self.node.filter(|_| self.attached).map(|node| (node, self.serial))
    }

    /// Whether the snapshot has been read to its last chunk.
    pub(crate) const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Records that the attach was granted.
    pub(crate) fn attached(&mut self) {
        self.attached = true;
    }
}

/// The attach request of an observer.
pub(crate) fn attach_request(request: u64) -> Envelope {
    Envelope::request_message(
        TREE,
        "attach",
        request,
        &AttachArgs {
            version: 1,
            mode: "observer".to_owned(),
        },
    )
}

/// The transaction that makes the one call an observer may: `lock` on `node`.
pub(crate) fn lock_request(request: u64, node: u64, serial: u64) -> Envelope {
    Envelope::request_message(
        TREE,
        "transact",
        request,
        &TransactArgs {
            base_serial: serial,
            serial: serial + 1,
            ops: vec![op::Properties {
                kind: "call".to_owned(),
                node,
                method: "lock".to_owned(),
                ..Default::default()
            }],
        },
    )
}

/// The outcome of a `lock` transaction: applied, or the compositor's refusal code.
pub(crate) fn lock_reply(payload: &[u8]) -> Result<(), ClientError> {
    let reply = <gnoblin_proto::tree::tree::ReplyPayload as prost::Message>::decode(payload).map_err(|_| {
        ClientError::Protocol {
            member: format!("{TREE}.transact"),
            reason: WireError::Truncated,
        }
    })?;
    if reply.applied {
        Ok(())
    } else {
        Err(ClientError::Refused {
            member: format!("{TREE}.transact"),
            code: reply.code,
            message: "the compositor refused the lock call".to_owned(),
        })
    }
}
