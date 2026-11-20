//! Workspaces and the membership of clients in them.
//!
//! A workspace is a place windows are laid out in. The compositor does not know it: the library makes it, keeps its clients as a
//! collection the shell may change, and calls its layout when it changes. Removing a workspace, or a window from one, never closes
//! the window. This is `host/workspace.ts`, `workspaces.ts` and `membership.ts` of the TypeScript library.

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_std_abi as abi;

use crate::error::{Fail, Res};
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode};
use crate::layout::LayoutObj;
use crate::nodes;
use crate::state::{Id, NodeId, Notice, State};

/// A workspace.
pub(crate) struct Workspace {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) layout: Rc<LayoutObj>,
    pub(crate) gap_inner: f64,
    pub(crate) gap_outer: f64,
    /// The workspace is in the added list.
    pub(crate) attached: bool,
    /// The clients on the workspace, in order.
    pub(crate) clients: Vec<NodeId>,
    /// The collection of `clients`.
    pub(crate) coll: Id,
    /// The clients that joined since the layout last heard of them.
    pub(crate) arrived: Vec<NodeId>,
    /// The clients that left since the layout last heard of them.
    pub(crate) departed: Vec<NodeId>,
    /// The pairs of clients that traded places since the layout last heard of it: it hears of them in its next pass, never inside the
    /// call that traded them.
    pub(crate) swapped: Vec<(NodeId, NodeId)>,
    /// The layout has to compute again the next time the workspace is shown.
    pub(crate) dirty: bool,
    /// The focused client the layout last computed for: a change of focus is a reason to compute again.
    pub(crate) focus_seen: Option<NodeId>,
    /// The fullscreen and maximised clients the layout last computed for.
    pub(crate) cover_seen: String,
    /// The layout's revision when the workspace was last looked at.
    pub(crate) revision: Option<u64>,
    /// The gaps the layout last saw.
    pub(crate) gaps_seen: (f64, f64),
    /// The area the layout last computed for.
    pub(crate) area: Option<Rect>,
    /// The library added this workspace for an output (the per-output policy), so it removes it when the output goes.
    pub(crate) auto: bool,
}

/// What a workspace had when a transition began staging: its layout, its gaps and its name.
pub(crate) type WorkspaceSnapshot = Vec<(Id, Rc<LayoutObj>, (f64, f64), String)>;

/// A gap that is a finite number of at least zero.
pub(crate) fn check_gap(value: f64, what: &str) -> Res<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        Err(Fail::type_error(format!(
            "{what} must be a finite number of at least 0, got {value}"
        )))
    }
}

impl State {
    /// The layout, the gaps and the name of every workspace, for a transition that may have to put them back.
    pub(crate) fn snapshot_workspaces(&self) -> WorkspaceSnapshot {
        self.ws
            .iter()
            .map(|(id, ws)| {
                (
                    *id,
                    Rc::clone(&ws.layout),
                    (ws.gap_inner, ws.gap_outer),
                    ws.name.clone(),
                )
            })
            .collect()
    }

    /// Puts back what [`State::snapshot_workspaces`] kept, for the workspaces that are still there: a layout a callback gave goes to
    /// the graveyard, and a workspace whose layout, gaps or name changed is laid out again. A workspace made or removed since is not
    /// touched.
    pub(crate) fn restore_workspaces(&mut self, snapshot: WorkspaceSnapshot) {
        for (id, layout, gaps, name) in snapshot {
            let Some(ws) = self.ws.get_mut(&id) else { continue };
            let mut changed = false;
            if !Rc::ptr_eq(&ws.layout, &layout) {
                let given = std::mem::replace(&mut ws.layout, layout);
                ws.revision = None;
                self.graveyard.push(given);
                changed = true;
            }
            if (ws.gap_inner, ws.gap_outer) != gaps {
                (ws.gap_inner, ws.gap_outer) = gaps;
                changed = true;
            }
            if ws.name != name {
                ws.name = name;
            }
            if changed {
                ws.dirty = true;
            }
        }
    }

    /// A floating layout with the default options.
    pub(crate) fn default_layout() -> Rc<LayoutObj> {
        LayoutObj::floating(PlaceMode::FreeSpace, DEFAULT_CASCADE_STEP)
    }

    /// Makes a workspace that is not in the added list yet.
    pub(crate) fn new_workspace(
        &mut self,
        id: String,
        name: String,
        gaps: (f64, f64),
        layout: Rc<LayoutObj>,
        auto: bool,
    ) -> Id {
        let handle = crate::state::next_id();
        let coll = self.new_root(
            abi::STD_COLLECTION_CLIENTS,
            crate::collection::Source::WorkspaceClients(handle),
        );
        self.ws.insert(
            handle,
            Workspace {
                name: if name.is_empty() { id.clone() } else { name },
                id,
                layout,
                gap_inner: gaps.0,
                gap_outer: gaps.1,
                attached: false,
                clients: Vec::new(),
                coll,
                arrived: Vec::new(),
                departed: Vec::new(),
                swapped: Vec::new(),
                dirty: true,
                focus_seen: None,
                cover_seen: String::new(),
                revision: None,
                gaps_seen: gaps,
                area: None,
                auto,
            },
        );
        handle
    }

    pub(crate) fn workspace(&self, id: Id) -> Res<&Workspace> {
        self.ws
            .get(&id)
            .ok_or_else(|| Fail::stale(format!("workspace {id} does not exist")))
    }

    pub(crate) fn workspace_mut(&mut self, id: Id) -> Res<&mut Workspace> {
        self.ws
            .get_mut(&id)
            .ok_or_else(|| Fail::stale(format!("workspace {id} does not exist")))
    }

    /// Puts `client`, which is on no workspace, on `workspace`.
    fn link(&mut self, workspace: Id, client: NodeId, arrived: bool) {
        let Some(ws) = self.ws.get_mut(&workspace) else {
            return;
        };
        ws.clients.push(client);
        ws.dirty = true;
        let coll = ws.coll;
        if arrived {
            ws.arrived.push(client);
        }
        if let Some(rec) = self.rec.get_mut(&client) {
            rec.workspace = Some(workspace);
            if arrived {
                rec.offered = true;
            }
        }
        self.notify_one(abi::STD_EVENT_ADDED, client, coll);
        self.recompute(abi::STD_COLLECTION_CLIENTS, client);
    }

    /// Takes `client` out of `workspace`.
    fn unlink(&mut self, workspace: Id, client: NodeId) {
        let Some(ws) = self.ws.get_mut(&workspace) else {
            return;
        };
        let coll = ws.coll;
        ws.clients.retain(|one| *one != client);
        ws.dirty = true;
        if let Some(rec) = self.rec.get_mut(&client) {
            rec.workspace = None;
        }
        self.forget_arrival(workspace, client);
        // The listeners of the workspace's clients hear it before the selections do, as the TypeScript library queues it.
        self.notices
            .push(Notice::new(abi::STD_EVENT_REMOVED, client, vec![coll]));
        self.recompute(abi::STD_COLLECTION_CLIENTS, client);
    }

    /// A client that leaves before its layout heard of it never was there for the layout.
    pub(crate) fn forget_arrival(&mut self, workspace: Id, client: NodeId) {
        let seen = self.rec.get(&client).is_some_and(|rec| rec.offered || rec.preexisting);
        let Some(ws) = self.ws.get_mut(&workspace) else {
            return;
        };
        if let Some(at) = ws.arrived.iter().position(|one| *one == client) {
            ws.arrived.remove(at);
        } else if seen {
            ws.departed.push(client);
        }
    }

    /// Moves `client` to `target` (or out of every workspace when it is `None`). `arrived` says a layout may place it.
    pub(crate) fn move_client(&mut self, client: NodeId, target: Option<Id>, arrived: bool) {
        let from = self.rec.get(&client).and_then(|rec| rec.workspace);
        if from == target {
            return;
        }
        if let Some(from) = from {
            self.unlink(from, client);
        }
        if let Some(target) = target {
            self.link(target, client, arrived);
        }
        // A dialog stays with its parent: it goes wherever the parent goes.
        for dialog in self.dialogs_of(client) {
            self.move_client(dialog, target, false);
        }
    }

    /// The live clients that are dialogs of `parent`.
    pub(crate) fn dialogs_of(&self, parent: NodeId) -> Vec<NodeId> {
        self.clients
            .iter()
            .copied()
            .filter(|client| {
                *client != parent
                    && nodes::alive(&self.g, *client)
                    && nodes::node_ref(&self.g, *client, "parent_window") == Some(parent)
            })
            .collect()
    }

    /// Puts a dialog where its parent is, and centres it in the parent the first time. Returns whether it did: a dialog whose parent
    /// is on no workspace is placed as any other window.
    pub(crate) fn place_dialog(&mut self, client: NodeId) -> bool {
        let Some(parent) = nodes::node_ref(&self.g, client, "parent_window") else {
            return false;
        };
        let Some(workspace) = self.rec.get(&parent).and_then(|rec| rec.workspace) else {
            return false;
        };
        if self.rec.get(&client).and_then(|rec| rec.workspace) != Some(workspace) {
            self.move_client(client, Some(workspace), false);
        }
        let centred = self.rec.get(&client).is_some_and(|rec| rec.centred);
        if !centred {
            if let Some(rec) = self.rec.get_mut(&client) {
                rec.centred = true;
            }
            let (own, around) = (nodes::rect_of(&self.g, client), nodes::rect_of(&self.g, parent));
            nodes::write_num(
                &self.g,
                client,
                "x",
                around.x + ((around.width - own.width) / 2.0).floor(),
            );
            nodes::write_num(
                &self.g,
                client,
                "y",
                around.y + ((around.height - own.height) / 2.0).floor(),
            );
        }
        true
    }

    /// The workspace a window that has none joins: the one the output under the pointer shows, else the first that is shown, else the
    /// first.
    pub(crate) fn new_window_workspace(&self) -> Option<Id> {
        self.current_desk()
            .and_then(|desk| self.desks.iter().find(|candidate| candidate.id == desk))
            .and_then(|desk| desk.shown)
            .or_else(|| self.initial_workspace())
    }

    /// The active workspace, or the first when none is.
    fn initial_workspace(&self) -> Option<Id> {
        self.workspaces
            .iter()
            .copied()
            .find(|workspace| self.is_shown(*workspace))
            .or_else(|| self.workspaces.first().copied())
    }

    /// Gives each window that has no workspace the one the initial-workspace rule names.
    pub(crate) fn adopt_orphans(&mut self) {
        if !self.opened {
            return;
        }
        for client in self.clients.clone() {
            if self.rec.get(&client).is_none_or(|rec| rec.workspace.is_some()) {
                continue;
            }
            if self.place_dialog(client) {
                continue;
            }
            let Some(target) = self.new_window_workspace() else {
                return;
            };
            let arrived = self.rec.get(&client).is_some_and(|rec| !rec.preexisting);
            self.move_client(client, Some(target), arrived);
        }
    }

    /// A shell that declared no workspace gets one floating `desktop`, so there is always somewhere for a window to go.
    pub(crate) fn ensure_default_workspace(&mut self) -> Res<()> {
        if !self.workspaces.is_empty() {
            return Ok(());
        }
        let workspace = self.new_workspace(
            "desktop".to_owned(),
            String::new(),
            (0.0, 0.0),
            Self::default_layout(),
            false,
        );
        self.add_workspace(workspace)
    }

    /// Adds a workspace to the list and gives it the windows that wait for one.
    pub(crate) fn add_workspace(&mut self, workspace: Id) -> Res<()> {
        let id = self.workspace(workspace)?.id.clone();
        if self.workspace(workspace)?.attached {
            return Ok(());
        }
        if self
            .workspaces
            .iter()
            .any(|other| self.ws.get(other).is_some_and(|ws| ws.id == id))
        {
            return Err(Fail::busy(format!("there is already a workspace with id \"{id}\"")));
        }
        let ws = self.workspace_mut(workspace)?;
        ws.attached = true;
        ws.dirty = true;
        self.workspaces.push(workspace);
        let root = self.roots[(abi::STD_COLLECTION_WORKSPACES - 1) as usize];
        self.notify_one(abi::STD_EVENT_ADDED, workspace, root);
        self.recompute(abi::STD_COLLECTION_WORKSPACES, workspace);
        self.ensure_desk();
        self.fill_desks();
        self.adopt_orphans();
        Ok(())
    }

    /// Takes the workspace out of the list. Its windows move to the neighbour before it (the one after it, for the first), and that
    /// neighbour is shown where the removed one was; none is closed.
    pub(crate) fn remove_workspace(&mut self, workspace: Id) {
        let Some(ws) = self.ws.get_mut(&workspace) else {
            return;
        };
        if !ws.attached {
            return;
        }
        ws.attached = false;
        let at = self.workspaces.iter().position(|one| *one == workspace).unwrap_or(0);
        let neighbour = at
            .checked_sub(1)
            .and_then(|before| self.workspaces.get(before))
            .or_else(|| self.workspaces.get(at + 1))
            .copied();
        self.workspaces.retain(|one| *one != workspace);
        let root = self.roots[(abi::STD_COLLECTION_WORKSPACES - 1) as usize];
        self.notify_one(abi::STD_EVENT_REMOVED, workspace, root);
        self.recompute(abi::STD_COLLECTION_WORKSPACES, workspace);
        for desk in 0..self.desks.len() {
            if self.desks[desk].shown != Some(workspace) {
                continue;
            }
            self.desks[desk].shown = None;
            // The neighbour takes its place, unless another output shows it already: then the first workspace no output shows.
            let free = match neighbour {
                Some(next) if self.desk_showing(next).is_none() => Some(next),
                _ => self
                    .workspaces
                    .iter()
                    .copied()
                    .find(|candidate| self.desk_showing(*candidate).is_none()),
            };
            let id = self.desks[desk].id;
            self.set_shown(id, free);
        }
        let moving = self
            .workspace(workspace)
            .map(|ws| ws.clients.clone())
            .unwrap_or_default();
        for client in moving {
            self.move_client(client, neighbour, true);
        }
    }

    /// Takes the workspace out of the list unless it is the last one: a shell with nowhere to put a window is unusable. Whether it was
    /// taken: a workspace that is not in the list is not.
    pub(crate) fn remove_workspace_checked(&mut self, workspace: Id) -> Res<bool> {
        let attached = self.workspace(workspace)?.attached;
        if !attached || self.workspaces.len() < 2 {
            return Ok(false);
        }
        self.remove_workspace(workspace);
        Ok(true)
    }

    /// Takes every workspace out of the list, closing no window. The last one goes too, which a shell that clears the list is about to
    /// fill again.
    pub(crate) fn clear_workspaces(&mut self) {
        for workspace in self.workspaces.clone() {
            self.remove_workspace(workspace);
        }
    }

    /// `client.moveToWorkspace(workspace)`: 1 when it moved, 0 when it is refused.
    pub(crate) fn move_client_to(&mut self, client: NodeId, workspace: Id) -> Res<bool> {
        let attached = self.workspace(workspace)?.attached;
        let from = self.rec.get(&client).and_then(|rec| rec.workspace);
        if !self.rec.contains_key(&client)
            || !nodes::alive(&self.g, client)
            || !attached
            || nodes::flag(&self.g, client, "fullscreen")
            || nodes::node_ref(&self.g, client, "parent_window").is_some()
            || from == Some(workspace)
        {
            return Ok(false);
        }
        let was_focused = nodes::focused_window(&self.g, &self.clients) == Some(client);
        self.move_client(client, Some(workspace), true);
        // The window left the screen, so the focus passes to a window that stays.
        if was_focused && !self.is_shown(workspace) {
            self.pass_focus_from(from, client);
        }
        if let Some(area) = self.work_area_of_workspace(workspace) {
            self.refit_client(client, &area);
        }
        Ok(true)
    }

    /// A reload gives `client` back to `workspace`: it moves, with its dialogs, but is not refitted. `arrived` says a layout may place
    /// it. 1 when it moved, 0 when it is refused.
    pub(crate) fn restore_client(&mut self, workspace: Id, client: NodeId, arrived: bool) -> Res<bool> {
        let attached = self.workspace(workspace)?.attached;
        let from = self.rec.get(&client).and_then(|rec| rec.workspace);
        if !attached || !self.rec.contains_key(&client) || !nodes::alive(&self.g, client) || from == Some(workspace) {
            return Ok(false);
        }
        self.move_client(client, Some(workspace), arrived);
        Ok(true)
    }

    /// `workspace.clients.add(client)`: the client leaves its workspace and joins this one.
    pub(crate) fn transfer_client(&mut self, workspace: Id, client: NodeId) -> Res<()> {
        if !self.workspace(workspace)?.attached {
            return Err(Fail::refused("add the workspace to the list first"));
        }
        if !self.rec.contains_key(&client) || !nodes::alive(&self.g, client) {
            return Err(Fail::no_such("the client is gone or is not a client of the compositor"));
        }
        self.move_client(client, Some(workspace), true);
        Ok(())
    }

    /// `workspace.clients.remove(client)`: the client has no workspace until it is given one. 1 when it was there.
    pub(crate) fn detach_client(&mut self, workspace: Id, client: NodeId) -> bool {
        if self.rec.get(&client).and_then(|rec| rec.workspace) == Some(workspace) {
            self.move_client(client, None, false);
            true
        } else {
            false
        }
    }

    /// Removes every client from a workspace without closing the windows.
    pub(crate) fn clear_clients(&mut self, workspace: Id) {
        for client in self
            .workspace(workspace)
            .map(|ws| ws.clients.clone())
            .unwrap_or_default()
        {
            self.detach_client(workspace, client);
        }
    }

    /// Trades the places of two clients in a workspace's order; its layout hears of it.
    pub(crate) fn swap_clients(&mut self, workspace: Id, a: NodeId, b: NodeId) -> Res<()> {
        let ws = self.workspace_mut(workspace)?;
        let (Some(first), Some(second)) = (
            ws.clients.iter().position(|one| *one == a),
            ws.clients.iter().position(|one| *one == b),
        ) else {
            return Err(Fail::err("both members must be in the collection"));
        };
        if first == second {
            return Ok(());
        }
        ws.clients.swap(first, second);
        ws.dirty = true;
        ws.swapped.push((a, b));
        Ok(())
    }

    /// Hands the keyboard to the first window still showing on `workspace`, or to nobody. A tree with no seat has none.
    pub(crate) fn pass_focus_from(&mut self, workspace: Option<Id>, leaving: NodeId) {
        let Some(seat) = nodes::seat(&self.g) else {
            return;
        };
        let next = workspace.and_then(|workspace| self.ws.get(&workspace)).and_then(|ws| {
            ws.clients
                .iter()
                .copied()
                .find(|one| *one != leaving && nodes::alive(&self.g, *one) && !nodes::flag(&self.g, *one, "minimised"))
        });
        match next {
            Some(next) => {
                let _ = seat.set("focus", gnoblin_core::NodeId(next));
            }
            None => {
                let _ = seat.set("focus", gnoblin_core::NodeId::NIL);
            }
        }
    }

    /// A window closed: hands the keyboard to the neighbour of a focused window, as every layout does. The window after it in the
    /// workspace, else the one before; a dialog hands it back to its parent. Nothing moves while the workspace is not shown, a
    /// minimised window is skipped, and a window the compositor has already focused is left alone.
    pub(crate) fn hand_off_focus(&mut self, workspace: Id, index: usize, parent: Option<NodeId>) {
        let Some(seat) = nodes::seat(&self.g) else {
            return;
        };
        if !self.is_shown(workspace) || nodes::focused_window(&self.g, &self.clients).is_some() {
            return;
        }
        let Some(ws) = self.ws.get(&workspace) else {
            return;
        };
        let usable = |client: Option<NodeId>| {
            client.filter(|c| nodes::alive(&self.g, *c) && !nodes::flag(&self.g, *c, "minimised"))
        };
        let next = [
            parent,
            ws.clients.get(index).copied(),
            index.checked_sub(1).and_then(|i| ws.clients.get(i).copied()),
        ]
        .into_iter()
        .find_map(usable);
        if let Some(next) = next {
            let _ = seat.set("focus", gnoblin_core::NodeId(next));
        }
    }
}
