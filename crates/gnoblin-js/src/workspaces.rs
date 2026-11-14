//! The Rust side of workspaces: what a reload carries from one shell to the next, and how a frame's
//! property writes are coalesced.
//!
//! A workspace is a script object (`Workspace` in `js/gnoblin.ts`), so the compositor does not know which
//! workspace a window is on. A new generation starts without that knowledge. [`JsWorkspaceSnapshot`] is
//! what the old shell hands over ([`JsShell::workspace_snapshot`](crate::JsShell::workspace_snapshot)) and
//! the new shell reconciles by workspace id ([`JsShell::reconcile`](crate::JsShell::reconcile)).

use std::collections::HashMap;

use gnoblin_tree_client::tree_mirror::{NodeId, TxOp};
use rquickjs::{Array, Ctx, Object};

/// One workspace of a snapshot: its id and the windows it held, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsWorkspaceEntry {
    pub id: String,
    pub clients: Vec<NodeId>,
}

/// What one output showed: the output's name and the id of its workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsOutputShown {
    pub output: String,
    pub workspace: String,
}

/// The workspaces of a shell, in order, the id of the one the primary output showed, and the one each output showed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsWorkspaceSnapshot {
    pub active: Option<String>,
    pub workspaces: Vec<JsWorkspaceEntry>,
    pub outputs: Vec<JsOutputShown>,
}

impl JsWorkspaceSnapshot {
    /// Reads the object `entry.snapshot()` returns.
    pub(crate) fn from_js<'js>(value: &Object<'js>) -> rquickjs::Result<Self> {
        let active: Option<String> = value.get("active")?;
        let list: Array<'js> = value.get("workspaces")?;
        let mut workspaces = Vec::with_capacity(list.len());
        for item in list.iter::<Object<'js>>() {
            let item = item?;
            let clients: Vec<f64> = item.get("clients")?;
            workspaces.push(JsWorkspaceEntry {
                id: item.get("id")?,
                // The ids are node ids the shell turned into numbers, so each is exact.
                clients: clients.into_iter().map(|id| id as NodeId).collect(),
            });
        }
        let shown: Array<'js> = value.get("outputs")?;
        let mut outputs = Vec::with_capacity(shown.len());
        for item in shown.iter::<Object<'js>>() {
            let item = item?;
            outputs.push(JsOutputShown {
                output: item.get("output")?,
                workspace: item.get("workspace")?,
            });
        }
        Ok(Self {
            active,
            workspaces,
            outputs,
        })
    }

    /// The object `entry.reconcile(previous)` takes.
    pub(crate) fn to_js<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
        let list = Array::new(ctx.clone())?;
        for (index, entry) in self.workspaces.iter().enumerate() {
            let item = Object::new(ctx.clone())?;
            item.set("id", entry.id.as_str())?;
            item.set(
                "clients",
                entry.clients.iter().map(|id| *id as f64).collect::<Vec<f64>>(),
            )?;
            list.set(index, item)?;
        }
        let shown = Array::new(ctx.clone())?;
        for (index, entry) in self.outputs.iter().enumerate() {
            let item = Object::new(ctx.clone())?;
            item.set("output", entry.output.as_str())?;
            item.set("workspace", entry.workspace.as_str())?;
            shown.set(index, item)?;
        }
        let object = Object::new(ctx.clone())?;
        object.set("active", self.active.as_deref())?;
        object.set("workspaces", list)?;
        object.set("outputs", shown)?;
        Ok(object)
    }
}

/// Keeps one instant write of each property per transaction, the last one. A script may write `x` twice in a
/// frame (a handler, then the layout), and only the final value is its intent. The kept write stays where it
/// was, so a write to a node the transaction makes still comes after the `New`. A write that animates
/// (`seconds` or `samples`) is not merged: its curve is its own.
pub(crate) fn coalesce_sets(ops: Vec<TxOp>) -> Vec<TxOp> {
    let keep: Vec<bool> = {
        let mut last: HashMap<(NodeId, &str), usize> = HashMap::new();
        for (index, op) in ops.iter().enumerate() {
            if let TxOp::Set {
                node,
                key,
                seconds: None,
                samples: None,
                ..
            } = op
            {
                last.insert((*node, key.as_str()), index);
            }
        }
        ops.iter()
            .enumerate()
            .map(|(index, op)| match op {
                TxOp::Set {
                    node,
                    key,
                    seconds: None,
                    samples: None,
                    ..
                } => last.get(&(*node, key.as_str())) == Some(&index),
                _ => true,
            })
            .collect()
    };
    ops.into_iter()
        .zip(keep)
        .filter_map(|(op, kept)| kept.then_some(op))
        .collect()
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::MirrorValue;

    use super::*;

    fn set(node: NodeId, key: &str, value: f64) -> TxOp {
        TxOp::Set {
            node,
            key: key.to_owned(),
            value: MirrorValue::Double(value),
            seconds: None,
            samples: None,
        }
    }

    #[test]
    fn the_last_instant_write_of_a_property_is_the_one_kept() {
        let ops = vec![
            set(3, "x", 1.0),
            set(3, "y", 5.0),
            set(4, "x", 7.0),
            set(3, "x", 2.0),
            set(3, "x", 3.0),
        ];
        assert_eq!(
            coalesce_sets(ops),
            vec![set(3, "y", 5.0), set(4, "x", 7.0), set(3, "x", 3.0)]
        );
    }

    #[test]
    fn an_animated_write_is_not_merged_and_other_ops_stay() {
        let animated = TxOp::Set {
            node: 3,
            key: "x".to_owned(),
            value: MirrorValue::Double(9.0),
            seconds: Some(0.2),
            samples: None,
        };
        let destroy = TxOp::Destroy { node: 5 };
        let ops = vec![set(3, "x", 1.0), animated.clone(), destroy.clone(), set(3, "x", 2.0)];
        assert_eq!(coalesce_sets(ops), vec![animated, destroy, set(3, "x", 2.0)]);
    }
}
