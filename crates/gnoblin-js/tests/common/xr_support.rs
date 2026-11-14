//! What the XR tests ask of the fake compositor: nodes by class, the ops the shell sent, and nodes that come and go
//! the way the XR program's do (the program is an observer: its seat and controllers are unparented).
#![allow(dead_code)]

use gnoblin_tree_client::tree_mirror::{MirrorChange, MirrorChangeKind, MirrorValue, TxOp};

use super::{FakeCompositor, change, set_change};

impl FakeCompositor {
    /// The ids of every node of `class`, in id order.
    pub fn ids_of_class(&self, class: &str) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .iter()
            .filter(|(_, node)| node.class == class)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The node named `name`, if there is one.
    pub fn named(&self, name: &str) -> Option<u64> {
        self.0
            .borrow()
            .nodes
            .iter()
            .find(|(_, node)| matches!(node.props.get("name"), Some(MirrorValue::Text(text)) if text == name))
            .map(|(id, _)| *id)
    }

    pub fn parent_of(&self, node: u64) -> Option<u64> {
        self.0.borrow().nodes.get(&node).map(|entry| entry.parent)
    }

    pub fn exists(&self, node: u64) -> bool {
        self.0.borrow().nodes.contains_key(&node)
    }

    /// Every transaction the shell has sent, oldest first.
    pub fn transactions(&self) -> Vec<Vec<TxOp>> {
        self.0.borrow().sent.clone()
    }

    /// Every op the shell has sent, oldest first.
    pub fn ops(&self) -> Vec<TxOp> {
        self.0.borrow().sent.iter().flatten().cloned().collect()
    }

    /// Forgets what the shell has sent, so the next assertion sees only what comes after.
    pub fn clear_sent(&self) {
        self.0.borrow_mut().sent.clear();
    }

    /// How many `Set` ops of `node` the shell has sent, per key.
    pub fn sets_of(&self, node: u64) -> Vec<String> {
        self.ops()
            .into_iter()
            .filter_map(|op| match op {
                TxOp::Set { node: target, key, .. } if target == node => Some(key),
                _ => None,
            })
            .collect()
    }

    /// A node the XR program makes while the shell runs: created, optionally parented, with its properties.
    pub fn appear(&self, id: u64, parent: u64, class: &str, props: Vec<(&str, MirrorValue)>) {
        self.add_node(id, parent, class, props.clone());
        let mut changes: Vec<MirrorChange> = vec![MirrorChange {
            class: class.to_owned(),
            ..change(MirrorChangeKind::Created, id)
        }];
        if parent != 0 {
            changes.push(MirrorChange {
                parent,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            });
        }
        changes.extend(props.into_iter().map(|(key, value)| set_change(id, key, value)));
        self.publish(changes);
    }

    /// Several properties of one node written in one batch, as the program writes a controller's frame.
    pub fn external_sets(&self, node: u64, writes: Vec<(&str, MirrorValue)>) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            for (key, value) in &writes {
                entry.props.insert((*key).to_owned(), value.clone());
            }
        }
        self.publish(
            writes
                .into_iter()
                .map(|(key, value)| set_change(node, key, value))
                .collect(),
        );
    }
}
