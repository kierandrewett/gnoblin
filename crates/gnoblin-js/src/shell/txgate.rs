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

/// Splits a transaction's ops into those to send now and those to send behind them. When the transaction holds a call
/// whose text is secret beside other ops, the secret calls are sent alone; every op is sent now otherwise, and when a
/// secret call is on a node this transaction makes (so that it cannot go without the `new`) too.
fn split_secret(ops: Vec<TxOp>, mirror: &TreeMirror) -> (Vec<TxOp>, Vec<TxOp>) {
    let kept = sent_ops(&ops, mirror);
    let secret = |index: usize| matches!(kept[index], SentOp::Secret { .. });
    let on_a_node_that_is_there = |op: &TxOp| matches!(op, TxOp::Call { node, .. } if mirror.node(*node).is_some());
    let alone = ops
        .iter()
        .enumerate()
        .all(|(index, op)| !secret(index) || on_a_node_that_is_there(op));
    let any_secret = (0..ops.len()).any(secret);
    let all_secret = (0..ops.len()).all(secret);
    if !alone || !any_secret || all_secret {
        return (ops, Vec::new());
    }
    let (mut apart, mut rest) = (Vec::new(), Vec::new());
    for (index, op) in ops.into_iter().enumerate() {
        if secret(index) {
            apart.push(op);
        } else {
            rest.push(op);
        }
    }
    (apart, rest)
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

/// What a refusal no newer base cures does to the ops it refused.
#[derive(Debug, Default, PartialEq)]
struct RefusedOps {
    /// The ops that go again, in order: the ones that were not at fault.
    kept: Vec<TxOp>,
    /// The ops given up: the one the compositor named (every op when it named none), and every op on a node that a
    /// given-up `new` would have made, which has nothing to land on.
    dropped: Vec<TxOp>,
    /// The calls whose text is secret, as `(method, token)`: they cannot go again.
    secret: Vec<(String, u64)>,
}

/// Splits the ops of a transaction the compositor refused for a reason of its own. `failed_op` is the op it names, or
/// `-1` when it refused the batch as a whole and so named none.
fn partition_refused(failed_op: i64, ops: Vec<SentOp>) -> RefusedOps {
    let named = usize::try_from(failed_op).ok();
    let mut out = RefusedOps::default();
    let mut unmade: Vec<NodeId> = Vec::new();
    for (index, op) in ops.into_iter().enumerate() {
        let op = match op {
            SentOp::Keep(op) => op,
            SentOp::Secret { method, token } => {
                out.secret.push((method, token));
                continue;
            }
        };
        let on_unmade = match &op {
            TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => unmade.contains(node),
            TxOp::Insert { node, parent, .. } => unmade.contains(node) || unmade.contains(parent),
            TxOp::New { .. } => false,
        };
        if named.is_none_or(|failed| failed == index) || on_unmade {
            if let TxOp::New { id, .. } = &op {
                unmade.push(*id);
            }
            out.dropped.push(op);
        } else {
            out.kept.push(op);
        }
    }
    out
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
            let op = {
                let state = self.state.borrow();
                usize::try_from(reply.failed_op)
                    .ok()
                    .and_then(|i| {
                        ops.get(i).map(|op| match op {
                            SentOp::Keep(op) => describe_op(i, op, &state.mirror),
                            SentOp::Secret { method, .. } => format!("op {i}: call {method} (secret text not kept)"),
                        })
                    })
                    .unwrap_or_else(|| "op not recorded".to_owned())
            };
            diagnostics.push(format!(
                "transaction {} refused at op {}: {}; {}",
                reply.serial, reply.failed_op, reply.code, op
            ));
            self.give_up_refused(reply, ops, diagnostics);
        }
    }

    /// Settles a refusal no newer base cures: the op it names is wrong whatever the base, so sending it again would be
    /// refused again. The compositor applies a transaction whole or not at all, so the ops beside the named one were
    /// refused with it and are not at fault: they go again, in order, on the next base, and a refusal costs the frame
    /// its one bad op and not the rest of its writes (see [`partition_refused`]).
    ///
    /// What is given up is told to whoever waits for it: a call is answered `failed`, so its caller is not left
    /// waiting, and a write is forgotten, so a read shows what the compositor holds and not the value it refused (the
    /// layout is not asked to write it again: the same value would be refused again). A call whose text is secret
    /// cannot go again whatever the cause and is answered `failed`.
    fn give_up_refused(&mut self, reply: &TxReply, ops: Vec<SentOp>, diagnostics: &mut Vec<String>) {
        let RefusedOps { kept, dropped, secret } = partition_refused(reply.failed_op, ops);
        let mut state = self.state.borrow_mut();
        for op in &dropped {
            match op {
                TxOp::Set { node, key, .. } => state.tick.forget_write(*node, key),
                TxOp::Call { token, .. } => fail_call(&mut state, *token, &reply.code),
                TxOp::New { .. } | TxOp::Insert { .. } | TxOp::Destroy { .. } => {}
            }
        }
        for (method, token) in secret {
            fail_call(&mut state, token, &reply.code);
            diagnostics.push(format!(
                "transaction {} {}: the {method} call carries a secret, was not sent again and failed",
                reply.serial, reply.code
            ));
        }
        log::debug!(
            area = "shell",
            "transaction {} refused {} at op {}; {} ops sent again, {} given up",
            reply.serial,
            reply.code,
            reply.failed_op,
            kept.len(),
            dropped.len()
        );
        let mut kept = kept;
        kept.append(&mut self.retry);
        self.retry = kept;
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
            // A call that carries a secret goes in a transaction of its own, so that a refusal the frame's other writes
            // cause does not cost it (its text cannot be sent again) and a refusal of it does not cost them. The rest
            // goes behind it, first on the next base, as an op refused stale-base does.
            let (now, later) = split_secret(std::mem::take(&mut tx.ops), &state.mirror);
            tx.ops = now;
            self.retry = later;
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

#[cfg(test)]
mod tests {
    use gnoblin_secret::SecretText;
    use gnoblin_tree_client::tree_mirror::MirrorValue;

    use super::*;

    fn set(node: NodeId, key: &str) -> SentOp {
        SentOp::Keep(TxOp::Set {
            node,
            key: key.to_owned(),
            value: MirrorValue::Double(1.0),
            seconds: None,
            delay: None,
            tween: None,
            samples: None,
        })
    }

    fn made(id: NodeId) -> SentOp {
        SentOp::Keep(TxOp::New {
            id,
            class: "canvas".to_owned(),
        })
    }

    fn insert(node: NodeId, parent: NodeId) -> SentOp {
        SentOp::Keep(TxOp::Insert {
            node,
            parent,
            index: -1,
        })
    }

    fn call(node: NodeId, token: u64) -> SentOp {
        SentOp::Keep(TxOp::Call {
            node,
            method: "apply".to_owned(),
            args: Vec::new(),
            token,
            text: SecretText::new(String::new()),
        })
    }

    fn ops(parts: Vec<SentOp>) -> Vec<TxOp> {
        parts
            .into_iter()
            .map(|op| match op {
                SentOp::Keep(op) => op,
                SentOp::Secret { .. } => panic!("a secret has no op to compare"),
            })
            .collect()
    }

    #[test]
    fn the_named_op_is_given_up_and_the_ops_beside_it_go_again_in_order() {
        let refused = partition_refused(1, vec![set(3, "x"), set(4, "y"), call(5, 7), set(6, "x")]);
        assert_eq!(refused.dropped, ops(vec![set(4, "y")]));
        assert_eq!(refused.kept, ops(vec![set(3, "x"), call(5, 7), set(6, "x")]));
        assert!(refused.secret.is_empty());
    }

    #[test]
    fn a_refusal_that_names_no_op_gives_every_op_up() {
        let refused = partition_refused(-1, vec![set(3, "x"), call(5, 7)]);
        assert!(refused.kept.is_empty(), "no op can be told from the others");
        assert_eq!(refused.dropped, ops(vec![set(3, "x"), call(5, 7)]));
    }

    #[test]
    fn the_ops_on_a_node_a_given_up_new_would_have_made_go_with_it() {
        // The `new` of node 9 is refused: its insert, its writes and the insert of a node under it have nothing to
        // land on. The window's write beside them, and a `new` of a node that is not under it, are not at fault.
        let refused = partition_refused(
            1,
            vec![
                set(3, "x"),
                made(9),
                insert(9, 1),
                set(9, "width"),
                made(10),
                insert(10, 9),
                set(10, "width"),
            ],
        );
        assert_eq!(
            refused.dropped,
            ops(vec![made(9), insert(9, 1), set(9, "width"), insert(10, 9)])
        );
        assert_eq!(refused.kept, ops(vec![set(3, "x"), made(10), set(10, "width")]));
    }

    #[test]
    fn a_secret_call_is_never_kept_whoever_was_at_fault() {
        let refused = partition_refused(
            0,
            vec![
                set(3, "x"),
                SentOp::Secret {
                    method: "authenticate".to_owned(),
                    token: 4,
                },
            ],
        );
        assert_eq!(refused.dropped, ops(vec![set(3, "x")]));
        assert!(refused.kept.is_empty());
        assert_eq!(refused.secret, [("authenticate".to_owned(), 4)]);
    }

    fn mirror() -> TreeMirror {
        let node = |id, parent, class: &str| gnoblin_tree_client::tree_mirror::SnapshotNode {
            id,
            parent,
            index: 0,
            class: class.to_owned(),
            title: String::new(),
            app_id: String::new(),
            props: Vec::new(),
        };
        TreeMirror::from_snapshot(1, 1, 1, &[node(1, 0, "node"), node(3, 1, "lock"), node(4, 1, "window")])
            .expect("a mirror")
    }

    fn check(node: NodeId) -> TxOp {
        TxOp::Call {
            node,
            method: "authenticate".to_owned(),
            args: Vec::new(),
            token: 9,
            text: SecretText::new("hunter2".to_owned()),
        }
    }

    fn plain(op: SentOp) -> TxOp {
        match op {
            SentOp::Keep(op) => op,
            SentOp::Secret { .. } => panic!("not a plain op"),
        }
    }

    #[test]
    fn a_password_check_goes_alone_and_the_ops_beside_it_go_behind_it_in_order() {
        let (x, y) = (plain(set(4, "x")), plain(set(4, "y")));
        let (now, later) = split_secret(vec![x.clone(), check(3), y.clone()], &mirror());
        assert_eq!(now, [check(3)]);
        assert_eq!(later, [x, y]);
    }

    #[test]
    fn a_transaction_with_no_secret_or_only_secrets_is_sent_as_it_is() {
        let x = plain(set(4, "x"));
        assert_eq!(split_secret(vec![x.clone()], &mirror()), (vec![x], Vec::new()));
        assert_eq!(split_secret(vec![check(3)], &mirror()), (vec![check(3)], Vec::new()));
    }

    #[test]
    fn a_secret_call_on_a_node_the_transaction_makes_cannot_go_without_it() {
        let ops = vec![
            TxOp::New {
                id: 9,
                class: "lock".to_owned(),
            },
            check(9),
            plain(set(4, "x")),
        ];
        // The call is secret by the schema, and its node is in no mirror yet: it cannot be separated from the `new` it
        // depends on, so the transaction is sent as it is.
        assert_eq!(split_secret(ops.clone(), &mirror()), (ops, Vec::new()));
    }
}
