//! A shell's connection to the tree, for the tests that need a node the compositor was never given a window for.
//!
//! The shell is an owner: it attaches, takes the snapshot, and writes transactions the way the stock shell does.

use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, Kind};

use super::{Session, Wire};

/// The side of the Views these tests make, in pixels.
pub const SIDE: u32 = 64;

/// A shell's connection to the tree, and the View it made.
pub struct Shell {
    wire: Wire,
    /// The View's node id, with the shell's epoch in its high half.
    pub view: u64,
    /// The serial the next transaction is based on.
    serial: u64,
}

/// The next reply to `member`, skipping the change batches and the snapshot that come between.
fn reply_to<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    loop {
        let envelope = wire.receive().expect("a frame");
        if matches!(envelope.kind, Some(Kind::Reply(_))) {
            assert_eq!(envelope.member, member);
            return envelope.payload_as::<M>().expect("the payload decodes");
        }
    }
}

fn tree_op(kind: &str) -> op::Properties {
    op::Properties {
        kind: kind.to_owned(),
        ..Default::default()
    }
}

fn set_size(node: u64, key: &str, value: f64) -> op::Properties {
    op::Properties {
        node,
        key: key.to_owned(),
        value_kind: "double".to_owned(),
        value_double: value,
        ..tree_op("set")
    }
}

impl Shell {
    /// Attaches to the tree and makes one View of `SIDE` by `SIDE` under the root.
    pub fn with_view(session: &Session) -> Shell {
        let mut wire = Wire::connect(session.socket());
        wire.send(&Envelope::request_message(
            "tree",
            "attach",
            1,
            &AttachArgs {
                version: 1,
                mode: String::new(),
            },
        ));
        let attached: AttachedPayload = reply_to(&mut wire, "attach");
        loop {
            let envelope = wire.receive().expect("a frame");
            if envelope.member == "snapshot" && envelope.payload_as::<SnapshotPayload>().is_some_and(|chunk| chunk.last)
            {
                break;
            }
        }
        let view = (attached.epoch << 32) | 0x102;
        let mut shell = Shell {
            wire,
            view,
            serial: attached.serial,
        };
        shell.transact(vec![
            op::Properties {
                class: "view".to_owned(),
                id: view,
                ..tree_op("new")
            },
            op::Properties {
                node: view,
                parent: attached.root,
                index: 0,
                ..tree_op("insert")
            },
            set_size(view, "width", f64::from(SIDE)),
            set_size(view, "height", f64::from(SIDE)),
        ]);
        shell
    }

    /// Applies `ops` and checks they applied.
    pub fn transact(&mut self, ops: Vec<op::Properties>) {
        self.wire.send(&Envelope::request_message(
            "tree",
            "transact",
            2,
            &TransactArgs {
                base_serial: self.serial,
                serial: self.serial + 1,
                ops,
            },
        ));
        let applied: ReplyPayload = reply_to(&mut self.wire, "transact");
        assert!(applied.applied, "the shell's transaction applied: {applied:?}");
        self.serial += 1;
    }

    /// Destroys the View.
    pub fn destroy_view(&mut self) {
        let view = self.view;
        self.transact(vec![op::Properties {
            node: view,
            ..tree_op("destroy")
        }]);
    }
}
