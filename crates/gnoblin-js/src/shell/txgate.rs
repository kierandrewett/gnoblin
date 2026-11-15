//! The transaction gate: one transaction in flight, its reply matched to it by position, and a refused
//! transaction's writes sent again.

use gnoblin_tree_client::tree_mirror::{NodeId, TreeMirror, TxBuilder, TxOp, TxReply};

use super::log;
use super::{JsShell, JsShellError, STALE_RETRIES};
use crate::bindings::HostState;
use crate::workspaces::coalesce_sets;

/// What the sent log keeps of one op: enough to send it again, or to tell its caller that it was lost.
pub(super) enum SentOp {
    /// An op that goes again as it is. A transaction the compositor refuses applies none of its ops (it applies
    /// whole or not at all), so no op of it, a call included, has run: sending it again runs it once.
    Keep(TxOp),
    /// A call whose text the schema marks secret. The text is wiped once the transaction is sent and is held nowhere
    /// else, so the call cannot go again; the token is kept to answer whoever waits for it.
    Secret { method: String, token: u64 },
}

/// The sent-log entry of one transaction: its serial and each op.
pub(super) type Sent = (u64, Vec<SentOp>);

/// What the sent log keeps of each op of a transaction about to be sent. A call is secret when the schema says so
/// for its node's class (`gnoblin_proto::tree_call_is_secret`); the class of a node the transaction makes is read
/// from its `new`. A call on a node of no known class is secret when it carries text: what cannot be placed is not
/// kept.
fn sent_ops(ops: &[TxOp], mirror: &TreeMirror) -> Vec<SentOp> {
    let class_of = |node: NodeId| {
        mirror.node(node).map(|record| record.class.as_str()).or_else(|| {
            ops.iter().find_map(|op| match op {
                TxOp::New { id, class } if *id == node => Some(class.as_str()),
                _ => None,
            })
        })
    };
    ops.iter()
        .map(|op| match op {
            TxOp::Call {
                node,
                method,
                token,
                text,
                ..
            } if class_of(*node).map_or(!text.is_empty(), |class| {
                gnoblin_proto::tree_call_is_secret(class, method)
            }) =>
            {
                SentOp::Secret {
                    method: method.clone(),
                    token: *token,
                }
            }
            other => SentOp::Keep(other.clone()),
        })
        .collect()
}

/// Names one op for a refusal: its index, class, property and value kind, so a refused write is traceable
/// to the script call that made it.
fn describe_op(index: usize, op: &TxOp, mirror: &TreeMirror) -> String {
    let class_of = |node: NodeId| {
        mirror
            .node(node)
            .map_or_else(|| "unknown".to_owned(), |record| record.class.clone())
    };
    match op {
        TxOp::New { id, class } => format!("op {index}: new node {id} class {class}"),
        TxOp::Set { node, key, value, .. } => format!(
            "op {index}: set node {node} ({}) property {key} value kind {}",
            class_of(*node),
            value.kind_name()
        ),
        TxOp::Insert {
            node,
            parent,
            index: at,
        } => format!(
            "op {index}: insert node {node} ({}) into {parent} at {at}",
            class_of(*node)
        ),
        TxOp::Destroy { node } => format!("op {index}: destroy node {node} ({})", class_of(*node)),
        TxOp::Call { node, method, .. } => format!("op {index}: call {method} on node {node} ({})", class_of(*node)),
    }
}

/// Hands an answer to the module that asked: a toolkit measurement, an output `apply` or a lock password check.
pub(super) fn route_answer(state: &mut HostState, token: u64, result: &[u8], error: &str) {
    if !state.ui.accept_answer(token, result, error) && !state.outputs.accept_answer(token, error) {
        state.lock.accept_answer(token, result, error);
    }
}

/// Answers a call that is lost for good (refused, and not to be sent again) with the failure the compositor would have
/// given, so whoever waits for its answer is not left waiting. A token nobody asked under is ignored.
fn fail_call(state: &mut HostState, token: u64, code: &str) {
    let error = format!("failed: the transaction was refused {code} and the call was not sent again");
    route_answer(state, token, b"", &error);
}

impl JsShell {
    /// Matches a reply to its transaction and, for a stale refusal, keeps the ops to send again.
    pub(super) fn handle_reply(&mut self, reply: &TxReply, diagnostics: &mut Vec<String>) {
        let (sent_serial, ops) = self.sent.pop_front().unwrap_or((reply.serial, Vec::new()));
        if reply.applied {
            self.stale_refusals = 0;
        }
        // `stale-base` names the first op on a node a published batch changed, or no op (`failed_op` -1) when the base
        // itself is too old for the compositor to check. `unknown-node` names the op on a node the compositor
        // destroyed. A newer base cures both; any other code is a mistake of the script's and is reported below.
        let cured_by_a_newer_base = match reply.code.as_str() {
            "stale-base" => true,
            "unknown-node" => reply.failed_op >= 0,
            _ => false,
        };
        if !reply.applied && cured_by_a_newer_base {
            self.send_again(reply, ops, diagnostics);
            return;
        }
        if reply.applied {
            // The compositor sends no change message for a transaction that changed no node: one that only
            // calls (`focus`, `claim`) still advances its serial, and one that wrote the values already there
            // does not. Its change messages precede its reply, so every one up to `reply.serial` has been
            // read, and the mirror catches up to the serial it answers with.
            self.state.borrow_mut().mirror.advance_serial(reply.serial);
        }
        if self.in_flight == Some(sent_serial) && (!reply.applied || reply.serial < sent_serial) {
            // A refused transaction never advances the serial, and an applied one that changed nothing
            // published none: the gate opens now, as nothing is left to wait for.
            self.in_flight = None;
        }
        log::debug!(
            area = "shell",
            "transaction {} applied={} code={} failed_op={}",
            reply.serial,
            reply.applied,
            reply.code,
            reply.failed_op
        );
        if !reply.applied {
            let state = self.state.borrow();
            let op = usize::try_from(reply.failed_op)
                .ok()
                .and_then(|i| {
                    ops.get(i).map(|op| match op {
                        SentOp::Keep(op) => describe_op(i, op, &state.mirror),
                        SentOp::Secret { method, .. } => format!("op {i}: call {method} (secret text not kept)"),
                    })
                })
                .unwrap_or_else(|| "op not recorded".to_owned());
            diagnostics.push(format!(
                "transaction {} refused at op {}: {}; {}",
                reply.serial, reply.failed_op, reply.code, op
            ));
        }
    }

    /// Keeps the ops of a transaction refused for an out-of-date base, to be sent again on the next base ahead of the
    /// next frame's writes, so the script's newest write lands last. The gate stays shut until the mirror reaches the
    /// serial the compositor refused at: every batch up to it came ahead of the reply, so the mirror that has read them
    /// is the base to send again on. The serial the shell stamped is no guide, because a refusal never advances the
    /// serial and the batch that made the base stale may be one the shell already read.
    ///
    /// Which ops go again: a refused transaction applied nothing, so every op of it is intact and goes again, in
    /// order, whatever its kind. The one exception is a call whose text is a secret (a password check): its text is
    /// wiped once sent, so its caller is answered `failed` and the diagnostic names it. The op the compositor names is
    /// sent again for [`STALE_RETRIES`] refusals in a row, and then given up, because a node the compositor changes
    /// before every landing would refuse the whole transaction for ever; under `unknown-node` it is given up at
    /// once, its node being gone. A refusal that names no op blames the base: the whole frame goes again, and is given
    /// up whole only when the base stayed too old for every resend.
    ///
    /// An op that is given up is not silently gone: a call is answered `failed`, and a property write is handed to the
    /// layout driver (`lost_writes`) and forgotten by the builder, so the script reads what the compositor holds and
    /// the layout writes the place again.
    fn send_again(&mut self, reply: &TxReply, ops: Vec<SentOp>, diagnostics: &mut Vec<String>) {
        let named = usize::try_from(reply.failed_op).ok();
        let stale = reply.code == "stale-base";
        let spent = self.stale_refusals >= STALE_RETRIES;
        self.stale_refusals = self.stale_refusals.saturating_add(1);
        let mut state = self.state.borrow_mut();
        let mut kept: Vec<TxOp> = Vec::new();
        let mut given_up: Vec<TxOp> = Vec::new();
        for (index, op) in ops.into_iter().enumerate() {
            let give_up = match named {
                Some(failed) => failed == index && (!stale || spent),
                None => spent,
            };
            match op {
                SentOp::Keep(op) if give_up => given_up.push(op),
                SentOp::Keep(op) => kept.push(op),
                SentOp::Secret { method, token } => {
                    fail_call(&mut state, token, &reply.code);
                    diagnostics.push(format!(
                        "transaction {} {}: the {method} call carries a secret, was not sent again and failed",
                        reply.serial, reply.code
                    ));
                }
            }
        }
        if stale && !given_up.is_empty() {
            diagnostics.push(match named {
                Some(index) => format!(
                    "transaction {} stale at op {index}: dropped after {STALE_RETRIES} resends",
                    reply.serial
                ),
                None => format!(
                    "transaction {} stale with no op named: the frame was dropped after {STALE_RETRIES} resends",
                    reply.serial
                ),
            });
        }
        for op in &given_up {
            match op {
                TxOp::Set { node, key, .. } if stale => {
                    state.lost_writes.push(*node);
                    // A newer write of the same property still to go keeps the value the script reads.
                    let again = kept
                        .iter()
                        .chain(&self.retry)
                        .any(|op| matches!(op, TxOp::Set { node: n, key: k, .. } if n == node && k == key));
                    if !again {
                        state.tick.forget_write(*node, key);
                    }
                }
                TxOp::Call { token, .. } => fail_call(&mut state, *token, &reply.code),
                _ => {}
            }
        }
        log::debug!(
            area = "shell",
            "transaction {} refused {} at op {}; {} ops sent again, {} given up",
            reply.serial,
            reply.code,
            reply.failed_op,
            kept.len(),
            given_up.len()
        );
        kept.append(&mut self.retry);
        self.retry = kept;
        if self.in_flight.is_some() {
            self.in_flight = Some(reply.serial);
        }
    }

    /// Sends this frame's writes as one transaction, once the one in flight has landed.
    pub(super) fn send_transaction(&mut self) -> Result<(), JsShellError> {
        let tx = {
            let mut guard = self.state.borrow_mut();
            let state = &mut *guard;
            // The gate opens on the reply and on the mirror reaching the serial: a batch of the compositor's own may
            // take the serial this transaction was stamped for, and a second transaction sent then is built on a base
            // that lacks the first, which the compositor refuses stale against it.
            if !self.sent.is_empty() || !self.in_flight.is_none_or(|serial| state.mirror.serial() >= serial) {
                return Ok(());
            }
            let serial = state.mirror.serial();
            let tick = std::mem::take(&mut state.tick);
            // What the transaction wrote stays readable until the compositor's batch brings it into the
            // mirror, which may be frames away.
            state.tick = TxBuilder::seeded(tick.sent(&state.mirror));
            let mut tx = tick.into_transaction(&state.mirror, serial + 1);
            if !self.retry.is_empty() {
                // Ops refused stale-base go first, on this base. An op on a node the tree has lost since is
                // moot, and sending it would refuse the whole transaction again; a call among them is answered
                // `failed`, as its caller waits for its answer.
                let mirror = &state.mirror;
                let mut made: Vec<NodeId> = Vec::new();
                let mut ops: Vec<TxOp> = Vec::new();
                let mut lost_calls: Vec<u64> = Vec::new();
                for op in std::mem::take(&mut self.retry) {
                    let present = |id: &NodeId, made: &[NodeId]| made.contains(id) || mirror.node(*id).is_some();
                    let alive = match &op {
                        TxOp::New { id, .. } => {
                            made.push(*id);
                            true
                        }
                        TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => {
                            present(node, &made)
                        }
                        TxOp::Insert { node, parent, .. } => present(node, &made) && present(parent, &made),
                    };
                    match op {
                        TxOp::Call { token, .. } if !alive => lost_calls.push(token),
                        op if alive => ops.push(op),
                        _ => {}
                    }
                }
                for token in lost_calls {
                    fail_call(state, token, "unknown-node");
                }
                ops.append(&mut tx.ops);
                tx.ops = ops;
            }
            // One write per property, the last: a handler's write and the layout's are one intent.
            tx.ops = coalesce_sets(std::mem::take(&mut tx.ops));
            tx
        };
        if !tx.ops.is_empty() {
            let kept = sent_ops(&tx.ops, &self.state.borrow().mirror);
            self.sent.push_back((tx.serial, kept));
            self.in_flight = Some(tx.serial);
            self.gate.sent = self.gate.sent.saturating_add(1);
            self.link.transact(&tx).map_err(|e| JsShellError::Link(e.0))?;
        }
        Ok(())
    }
}
