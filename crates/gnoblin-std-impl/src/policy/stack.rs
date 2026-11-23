//! What the policy reads and writes of the windows as a stack: which are there, which are on the same workspace, who holds the keyboard,
//! and the moves that put a window on top of the others or under them.

use std::collections::HashSet;

use gnoblin_core::Rect;
use gnoblin_object::{Res, Val};

use crate::nodes;
use crate::object::record::{look, number_field, record};
use crate::ui::Ui;

/// Whether `client` is a window that is still there.
pub(super) fn alive(ui: &Ui, client: u64) -> bool {
    let node = ui.node(client);
    node.is_alive() && node.class_name().is_ok_and(|class| class == "window")
}

/// The windows back to front: those under the root in tree order, then the ones the compositor made without a parent, by node number (the
/// order they were mapped in).
pub(super) fn stack(ui: &Ui) -> Vec<u64> {
    let mut order: Vec<u64> =
        ui.g.root()
            .and_then(|root| root.children())
            .unwrap_or_default()
            .into_iter()
            .filter(|node| node.class_name().is_ok_and(|class| class == "window"))
            .map(|node| node.id().0)
            .collect();
    let seen: HashSet<u64> = order.iter().copied().collect();
    let mut orphans: Vec<u64> =
        ui.g.windows()
            .unwrap_or_default()
            .into_iter()
            .map(|window| window.id().0)
            .filter(|id| !seen.contains(id))
            .collect();
    orphans.sort_unstable();
    order.extend(orphans);
    order
}

/// The workspace `client` is on, if the library holds it on one.
fn workspace_of(ui: &Ui, client: u64) -> Option<u64> {
    look(ui.ctx, |st| Ok(st.rec.get(&client).and_then(|rec| rec.workspace)))
        .ok()
        .flatten()
}

/// The windows on the workspace of `client` (the windows with no workspace, for one that has none), back to front.
pub(super) fn siblings(ui: &Ui, client: u64) -> Vec<u64> {
    let own = workspace_of(ui, client);
    stack(ui)
        .into_iter()
        .filter(|other| workspace_of(ui, *other) == own)
        .collect()
}

/// Whether the workspace of `client` is the one on show. A window on no workspace is not.
pub(super) fn on_active_workspace(ui: &Ui, client: u64) -> bool {
    workspace_of(ui, client)
        .and_then(|workspace| look(ui.ctx, |st| Ok(st.is_shown(workspace))).ok())
        .unwrap_or(false)
}

/// Gives every window the compositor made without a parent the root as its parent, in the order the tree stacks them: a parentless window
/// stacks after everything under the root, whatever the root holds.
fn adopt(ui: &Ui, order: &[u64], skip: u64) {
    let Ok(root) = ui.g.root() else {
        return;
    };
    for id in order.iter().copied().filter(|id| *id != skip) {
        let node = ui.node(id);
        if node.parent().ok().flatten().is_none() {
            let _ = node.insert_into(&root, None);
        }
    }
}

/// Puts `client` on top of every other window, and puts each fullscreen window of its workspace back over it. A window that is on top
/// already is left alone.
pub(super) fn raise(ui: &Ui, client: u64) {
    let order = stack(ui);
    if order.last() != Some(&client) {
        adopt(ui, &order, client);
        if let Ok(root) = ui.g.root() {
            let _ = ui.node(client).insert_into(&root, None);
        }
    }
    if nodes::flag(&ui.g, client, "fullscreen") {
        return;
    }
    let own = workspace_of(ui, client);
    for other in stack(ui) {
        if other != client && nodes::flag(&ui.g, other, "fullscreen") && workspace_of(ui, other) == own {
            let order = stack(ui);
            if order.last() != Some(&other) {
                adopt(ui, &order, other);
                if let Ok(root) = ui.g.root() {
                    let _ = ui.node(other).insert_into(&root, None);
                }
            }
        }
    }
}

/// Puts `client` under every other window. A window that is at the bottom already is left alone.
pub(super) fn lower(ui: &Ui, client: u64) {
    let order = stack(ui);
    if order.first() == Some(&client) {
        return;
    }
    adopt(ui, &order, client);
    if let Ok(root) = ui.g.root() {
        let _ = ui.node(client).insert_into(&root, Some(0));
    }
}

/// The window that has the keyboard, or none.
pub(super) fn focused(ui: &Ui) -> Option<u64> {
    let windows = stack(ui);
    nodes::focused_window(&ui.g, &windows)
}

/// Gives the keyboard to `client`, or takes it from every window. A tree with no seat has nobody to give it to.
pub(super) fn focus(ui: &Ui, client: Option<u64>) {
    if nodes::seat(&ui.g).is_none() {
        return;
    }
    let _ = ui.set_focus(client.unwrap_or(0));
}

/// A `layout_rect` as a value.
pub(super) fn rect_val(rect: &Rect) -> Val {
    record(vec![
        Val::Double(rect.x),
        Val::Double(rect.y),
        Val::Double(rect.width),
        Val::Double(rect.height),
    ])
}

/// The rectangle a `layout_rect` value holds; a field left out is 0.
pub(super) fn rect_in(value: &Val) -> Res<Rect> {
    let fields = value.record()?;
    Ok(Rect::new(
        number_field(fields, 0, 0.0)?,
        number_field(fields, 1, 0.0)?,
        number_field(fields, 2, 0.0)?,
        number_field(fields, 3, 0.0)?,
    ))
}
