//! A fake owner that keeps the tree an observer builds, for the controller tests.
//!
//! It answers the attach with a snapshot, applies every `new`, `set` and `destroy` of an owner transaction to a tree of
//! its own, publishes the matching change batch before it answers (as the compositor does, so the client's mirror shows
//! the write by the time the transaction returns), and keeps every transaction it saw. A test reads the tree the
//! program built, makes the next transaction fail with a code, and writes a `claimed` list the way the shell does.
//! Nothing here has met a real compositor: the compositor's own rules for observers are tested in `gnoblin-compositor`.

use std::collections::{BTreeMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use gnoblin_proto::tree::change;
use gnoblin_proto::tree::op;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, read_frame, write_frame};

/// The request id the client uses for its attach request.
const ATTACH_REQUEST_ID: u64 = 1;

/// One node the observer made: its class and the last `set` of each of its properties.
#[derive(Debug, Clone, Default)]
pub(crate) struct Node {
    pub class: String,
    props: BTreeMap<String, op::Properties>,
}

impl Node {
    pub fn text(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(|op| op.value_text.as_str())
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        self.props.get(key).map(|op| op.value_double)
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        self.props.get(key).map(|op| op.value_bool)
    }

    pub fn list(&self, key: &str) -> Option<&[f64]> {
        self.props.get(key).map(|op| op.value_list.as_slice())
    }
}

#[derive(Default)]
struct State {
    transactions: Vec<Vec<op::Properties>>,
    nodes: BTreeMap<u64, Node>,
    /// The codes the next transactions are refused with, one each.
    refusals: VecDeque<&'static str>,
}

/// The fake owner's handle: what it saw, what it holds, and what the shell would write.
#[derive(Clone)]
pub(crate) struct Owner {
    state: Arc<Mutex<State>>,
    out: Arc<Mutex<UnixStream>>,
    serial: Arc<AtomicU64>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Copies the value `op` carries into a change of the same property.
fn change_of(op: &op::Properties) -> change::Properties {
    change::Properties {
        kind: "set".to_owned(),
        node: op.node,
        key: op.key.clone(),
        value_kind: op.value_kind.clone(),
        value_bool: op.value_bool,
        value_int: op.value_int,
        value_uint: op.value_uint,
        value_double: op.value_double,
        value_text: op.value_text.clone(),
        value_node: op.value_node,
        value_list: op.value_list.clone(),
        value_texts: op.value_texts.clone(),
        ..Default::default()
    }
}

/// Starts a fake owner whose snapshot holds `nodes`, and returns the client's end of the socket and the owner's handle.
pub(crate) fn serve(nodes: Vec<snapshot_node::Properties>) -> (UnixStream, Owner) {
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    let owner = Owner {
        state: Arc::default(),
        out: Arc::new(Mutex::new(server.try_clone().expect("a second handle"))),
        serial: Arc::new(AtomicU64::new(1)),
    };
    let handle = owner.clone();
    thread::spawn(move || {
        let attach = read_frame(&mut server).expect("read attach").expect("attach frame");
        assert_eq!(attach.request_id, ATTACH_REQUEST_ID);
        let attached = Envelope::reply_message(
            "tree",
            "attach",
            ATTACH_REQUEST_ID,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 1,
            },
        );
        let snapshot = Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 1,
                chunk: 0,
                last: true,
                nodes,
            },
        );
        {
            let mut out = locked(&handle.out);
            for event in [&attached, &snapshot] {
                if write_frame(&mut *out, event).is_err() {
                    return;
                }
            }
        }
        while let Ok(Some(request)) = read_frame(&mut server) {
            let args = request.payload_as::<TransactArgs>().expect("transact args");
            let refused = {
                let mut state = locked(&handle.state);
                state.transactions.push(args.ops.clone());
                state.refusals.pop_front()
            };
            let echo = if refused.is_some() {
                Vec::new()
            } else {
                handle.apply(&args.ops)
            };
            let mut out = locked(&handle.out);
            if !echo.is_empty() {
                let serial = handle.serial.fetch_add(1, Ordering::SeqCst) + 1;
                let event = Envelope::event_message("tree", "changes", &ChangesPayload { serial, changes: echo });
                if write_frame(&mut *out, &event).is_err() {
                    return;
                }
            }
            let reply = Envelope::reply_message(
                "tree",
                "transact",
                request.request_id,
                &ReplyPayload {
                    serial: handle.serial.load(Ordering::SeqCst),
                    applied: refused.is_none(),
                    failed_op: if refused.is_some() { 0 } else { -1 },
                    code: refused.unwrap_or_default().to_owned(),
                    removed: Vec::new(),
                },
            );
            if write_frame(&mut *out, &reply).is_err() {
                return;
            }
        }
    });
    (client, owner)
}

impl Owner {
    /// Applies `ops` to the tree and returns the changes that publish them.
    fn apply(&self, ops: &[op::Properties]) -> Vec<change::Properties> {
        let mut state = locked(&self.state);
        let mut changes = Vec::new();
        for op in ops {
            match op.kind.as_str() {
                "new" => {
                    state.nodes.insert(
                        op.id,
                        Node {
                            class: op.class.clone(),
                            ..Node::default()
                        },
                    );
                    changes.push(change::Properties {
                        kind: "created".to_owned(),
                        node: op.id,
                        class: op.class.clone(),
                        ..Default::default()
                    });
                }
                "set" => {
                    if let Some(node) = state.nodes.get_mut(&op.node) {
                        node.props.insert(op.key.clone(), op.clone());
                    }
                    changes.push(change_of(op));
                }
                "destroy" => {
                    state.nodes.remove(&op.node);
                    changes.push(change::Properties {
                        kind: "destroyed".to_owned(),
                        node: op.node,
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }
        changes
    }

    /// Every transaction the client sent, oldest first.
    pub fn transactions(&self) -> Vec<Vec<op::Properties>> {
        locked(&self.state).transactions.clone()
    }

    /// How many transactions the client has sent.
    pub fn count(&self) -> usize {
        locked(&self.state).transactions.len()
    }

    /// Closes the socket, as an owner that went away does.
    pub fn hang_up(&self) {
        let _ = locked(&self.out).shutdown(std::net::Shutdown::Both);
    }

    /// The nodes of class `class` the owner holds, by id.
    pub fn nodes_of(&self, class: &str) -> Vec<(u64, Node)> {
        locked(&self.state)
            .nodes
            .iter()
            .filter(|(_, node)| node.class == class)
            .map(|(id, node)| (*id, node.clone()))
            .collect()
    }

    /// The controller whose `hand` is `hand`, with its id.
    pub fn controller(&self, hand: &str) -> Option<(u64, Node)> {
        self.nodes_of("controller")
            .into_iter()
            .find(|(_, node)| node.text("hand") == Some(hand))
    }

    /// Refuses the next transaction with `code`, and applies nothing of it.
    pub fn refuse_next(&self, code: &'static str) {
        locked(&self.state).refusals.push_back(code);
    }

    /// The shell writes `claimed` on `node`: the owner publishes the change as it would for the holder's write.
    pub fn claim(&self, node: u64, names: &[&str]) {
        let serial = self.serial.fetch_add(1, Ordering::SeqCst) + 1;
        let change = change::Properties {
            kind: "set".to_owned(),
            node,
            key: "claimed".to_owned(),
            value_kind: "list_text".to_owned(),
            value_texts: names.iter().map(|name| (*name).to_owned()).collect(),
            ..Default::default()
        };
        let event = Envelope::event_message(
            "tree",
            "changes",
            &ChangesPayload {
                serial,
                changes: vec![change],
            },
        );
        write_frame(&mut *locked(&self.out), &event).expect("the claim reaches the client");
    }
}
