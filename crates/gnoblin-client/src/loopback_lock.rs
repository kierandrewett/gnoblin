//! The loopback compositor's tree: one lock node, served the way the real tree serves an observer.
//!
//! A test publishes the lock's state with [`crate::Server::publish_lock`] and reads the `lock` calls the
//! clients made with [`crate::Server::lock_calls`]. What an observer sees is the real wire: an attach reply,
//! the snapshot, then `changes` batches in serial order. Nothing here decides policy; the lock rules are the
//! compositor's and are tested there.

use gnoblin_model::OwnerId;
use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{change, snapshot_node};
use gnoblin_proto::{Envelope, StreamFrame};

use crate::lock::TREE;

/// The node id the loopback gives the lock, and the root above it.
const ROOT: u64 = 1;
const LOCK: u64 = 2;

/// The loopback's lock node and who observes it.
#[derive(Debug, Default)]
pub(crate) struct LoopLock {
    /// The node's `state`; `None` when the tree has no lock node (a compositor older than the class).
    state: Option<String>,
    serial: u64,
    observers: Vec<OwnerId>,
    calls: u64,
    refusal: Option<String>,
}

impl LoopLock {
    pub(crate) fn calls(&self) -> u64 {
        self.calls
    }

    pub(crate) fn deny_calls(&mut self, code: &str) {
        self.refusal = Some(code.to_owned());
    }

    /// Sets the state and returns the frames each observer is owed, with the owner they go to.
    pub(crate) fn publish(&mut self, state: &str) -> Vec<(OwnerId, StreamFrame)> {
        let created = self.state.is_none();
        self.state = Some(state.to_owned());
        self.serial += 1;
        let mut changes = Vec::new();
        if created {
            changes.push(change::Properties {
                kind: "created".to_owned(),
                node: LOCK,
                class: "lock".to_owned(),
                ..Default::default()
            });
        }
        changes.push(state_change(state));
        let batch = ChangesPayload {
            serial: self.serial,
            changes,
        };
        self.observers
            .iter()
            .map(|owner| {
                (
                    *owner,
                    StreamFrame::Envelope(Envelope::event_message(TREE, "changes", &batch)),
                )
            })
            .collect()
    }

    /// Answers a tree request from `owner`.
    pub(crate) fn handle(&mut self, owner: OwnerId, envelope: &Envelope, out: &mut Vec<StreamFrame>) {
        match envelope.member.as_str() {
            "attach" => {
                let observer = envelope
                    .payload_as::<AttachArgs>()
                    .is_some_and(|args| args.mode == "observer");
                if !observer {
                    out.push(error(envelope, "not-owner", "the loopback tree serves observers only"));
                    return;
                }
                if !self.observers.contains(&owner) {
                    self.observers.push(owner);
                }
                out.push(StreamFrame::Envelope(Envelope::reply_message(
                    TREE,
                    "attach",
                    envelope.request_id,
                    &AttachedPayload {
                        epoch: 1,
                        root: ROOT,
                        serial: self.serial,
                    },
                )));
                let mut nodes = vec![node(ROOT, 0, "", Vec::new())];
                if let Some(state) = &self.state {
                    let props = if state == "not-locked" {
                        Vec::new()
                    } else {
                        vec![state_change(state)]
                    };
                    nodes.push(node(LOCK, ROOT, "lock", props));
                }
                out.push(StreamFrame::Envelope(Envelope::event_message(
                    TREE,
                    "snapshot",
                    &SnapshotPayload {
                        serial: self.serial,
                        chunk: 0,
                        last: true,
                        nodes,
                    },
                )));
            }
            "transact" => {
                let args = envelope.payload_as::<TransactArgs>().unwrap_or_default();
                let lock_call = args.ops.len() == 1
                    && args.ops[0].kind == "call"
                    && args.ops[0].method == "lock"
                    && args.ops[0].node == LOCK
                    && self.observers.contains(&owner);
                let code = if let Some(code) = self.refusal.clone().filter(|_| lock_call) {
                    code
                } else if lock_call {
                    self.calls += 1;
                    String::new()
                } else {
                    "not-holder".to_owned()
                };
                out.push(StreamFrame::Envelope(Envelope::reply_message(
                    TREE,
                    "transact",
                    envelope.request_id,
                    &ReplyPayload {
                        serial: self.serial,
                        applied: code.is_empty(),
                        failed_op: if code.is_empty() { -1 } else { 0 },
                        code,
                        removed: Vec::new(),
                    },
                )));
            }
            _ => out.push(error(
                envelope,
                "unknown-member",
                "the loopback tree serves attach and transact",
            )),
        }
    }
}

fn state_change(state: &str) -> change::Properties {
    change::Properties {
        kind: "set".to_owned(),
        node: LOCK,
        key: "state".to_owned(),
        value_kind: "text".to_owned(),
        value_text: state.to_owned(),
        ..Default::default()
    }
}

fn node(id: u64, parent: u64, class: &str, props: Vec<change::Properties>) -> snapshot_node::Properties {
    snapshot_node::Properties {
        id,
        parent,
        class: class.to_owned(),
        props,
        ..Default::default()
    }
}

fn error(envelope: &Envelope, code: &str, message: &str) -> StreamFrame {
    StreamFrame::Envelope(Envelope::error(
        TREE,
        envelope.member.clone(),
        envelope.request_id,
        code,
        message,
    ))
}
