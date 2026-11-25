//! The turn: what the library does once after the listeners of a dispatch turn have run.
//!
//! Anything that changes a workspace (a client joins, a gap changes, an output appears) marks it dirty and posts one turn to the
//! shell thread. The core runs posted work before the events of the next dispatch, so the turn sees the tree as the last batch
//! left it. It does what the TypeScript `layout_driver.ts` does after listeners: it tells the shell's listeners what changed, and
//! computes the workspaces that are dirty and shown, writes their placements as tree writes (one transaction at the end of the
//! dispatch cycle, coalesced per property by the core), and shows and hides windows. A workspace that is not shown stays dirty
//! until it is.
//!
//! Refusal and resend are the core's: a transaction the compositor refuses `stale-base` is sent again by the core's gate. What the
//! core hands back as a lost write is not part of its table, so the driver finds a lost placement itself: a window that still holds
//! what it held before the pass wrote its rectangle was never moved, and its workspace is laid out again (at most three times in a
//! row, so a compositor that keeps refusing is not asked for ever).
//!
//! A pass reads the focus, fullscreen and maximised flags of a workspace's windows only when one may have changed: the handler of each
//! of those properties marks the workspace (`scan_again`), and a workspace that is not marked, whose members, gaps, area and layout
//! revision are as the last pass saw them, and that is not due a compute, is left alone (`workspace_at_rest`). A turn that only a title
//! changed therefore costs the same at any number of windows. Lost writes are looked for in the windows whose placement has not been
//! seen held (`unverified`), and a pass that reads staged values (`settle`) looks at everything.
//!
//! A flag a pass reads may be a write the compositor has not answered: the tree shows a write that was sent until the answer comes (the
//! library's own answer to a window's request is one), and a write the compositor refuses, or drops after the gate's resends, is
//! forgotten with no event, so the flag reads as it was and nothing says so. A pass that finds the windows that cover their workspace
//! (the fullscreen and maximised ones) other than it had noted them therefore reads the workspace again once the compositor has
//! answered everything sent (`reread_when_answered`). `focused` is read-only for a shell, so no write of it is ever in flight.

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use gnoblin_core::{Gnoblin, LogLevel, Rect};
use gnoblin_std_abi as abi;

use crate::animation::easing::Easing;
use crate::animation::engine::Options;
use crate::animation::staging::{Curve, Prop, Staged};
use crate::floating::{grow, shrink};
use crate::layout::{LayoutContext, Placement};
use crate::nodes;
use crate::state::{self, Id, NodeId, Notice, State, Std};

/// How many passes in a row may find a placement missing before the driver stops asking.
const LOST_RETRIES: u8 = 3;

/// Posts a turn to the shell thread, once: a turn that is posted and has not run is enough for any number of changes.
pub(crate) fn schedule(std: &Rc<Std>) {
    if std.scheduled.replace(true) {
        return;
    }
    let (g, ctx) = {
        let st = std.st.borrow();
        (st.g.clone(), st.ctx)
    };
    let posted = g.poster().post(move |_g| {
        if let Some(std) = state::find(ctx) {
            turn(&std);
        }
    });
    if posted.is_err() {
        std.scheduled.set(false);
    }
}

/// One turn: the first one gives the windows that waited a workspace; every one tells the listeners what changed, runs the layout
/// pass, and tells them what the pass changed.
pub(crate) fn turn(std: &Rc<Std>) {
    std.scheduled.set(false);
    let g = std.g();
    {
        let mut st = std.st.borrow_mut();
        if st.desktop && !st.opened {
            st.opened = true;
            if let Err(fail) = st.ensure_default_workspace() {
                g.log_at(
                    LogLevel::Warn,
                    &format!("gnoblin-std: no default workspace: {}", fail.message),
                );
            }
            st.adopt_orphans();
        }
        st.flush_geometry();
        st.anim_flush();
        if st.desktop {
            st.reflow_displays();
        }
    }
    std.drain();
    layout_pass(std, &g);
    std.st.borrow_mut().apply_styles();
    std.st.borrow_mut().publish_active_workspace();
    crate::display_objects::frame(std);
    let wrote = std::mem::take(&mut std.st.borrow_mut().placements_written);
    if wrote {
        acknowledge_writes(std, &g);
    }
    let unconfirmed = std::mem::take(&mut std.st.borrow_mut().cover_unconfirmed);
    if !unconfirmed.is_empty() {
        reread_when_answered(std, &g, unconfirmed);
    }
    std.drain();
    // The views of workspaces follow what the pass did.
    let ctx = std.st.borrow().ctx;
    if let Some(world) = crate::scene::registry::find(ctx) {
        world.mirror_turn();
    }
    // The objects of the transitions that settled hear it from the next turn.
    let settled = std.st.borrow_mut().anim_take_settled();
    if !settled.is_empty() {
        crate::animation::objects::announce(ctx, settled);
    }
    // The layout pass and the drains told of closed windows, and what is queued holds what it names: their `Client`s go now.
    crate::object::client::turn_ended(ctx);
}

/// The passes of a turn that a staged transition runs against its staged state, so what they write is staged with the callback's:
/// the layout pass, then the style pass.
pub(crate) fn settle(std: &Rc<Std>) {
    let g = std.g();
    // What the pass reads of a window is what the transition staged, which the tree does not hold and says nothing of when it does
    // not take it (the callback failed, or the commit was refused). So `scan_all` is set twice, on purpose: before the pass, so that it
    // looks at every window against the staged state; and after it, because the pass has noted that staged state as read, and the
    // next turn has to look at every window again against the tree.
    std.st.borrow_mut().scan_all = true;
    layout_pass(std, &g);
    std.st.borrow_mut().scan_all = true;
    std.st.borrow_mut().apply_styles();
}

/// A slide the pass started: the windows have been moved to where it begins, and these are the writes that carry them home.
struct Play {
    moves: Vec<(NodeId, f64)>,
    duration_ms: f64,
    from: Id,
    resting: Vec<(NodeId, f64)>,
}

/// The layout pass: recomputes the workspaces that changed and applies their placements.
fn layout_pass(std: &Rc<Std>, g: &Gnoblin) {
    {
        let mut st = std.st.borrow_mut();
        let all = std::mem::take(&mut st.scan_all);
        if all {
            for ws in st.ws.values_mut() {
                ws.scan_stale = true;
            }
        }
        st.relay_lost_writes(all);
        st.fill_desks();
        st.counters.layout_passes += 1;
    }
    let workspaces = std.st.borrow().workspaces.clone();
    for workspace in workspaces {
        pass_workspace(std, g, workspace);
    }
    let plays = {
        let mut st = std.st.borrow_mut();
        let focus = nodes::focused_window(&st.g, &st.clients);
        st.last_focus = focus;
        let plays = st.start_slides();
        st.sync_visibility();
        plays
    };
    for play in plays {
        play_slide(std, g, play);
    }
}

/// Once the compositor has answered the transaction that carries this pass's placements, a window that does not hold what the pass
/// wrote is one whose write was lost. Before it has, the tree may simply not have heard of the change yet.
fn acknowledge_writes(std: &Rc<Std>, g: &Gnoblin) {
    let (ctx, pass) = {
        let st = std.st.borrow();
        (st.ctx, st.counters.layout_passes)
    };
    let _ = g.flush(move |_serial| {
        if let Some(std) = state::find(ctx) {
            for rec in std.st.borrow_mut().rec.values_mut() {
                if rec.write_pass <= pass {
                    rec.acked = true;
                }
            }
        }
    });
}

/// A pass read the windows that cover `workspaces` (the fullscreen and maximised ones) other than it had noted them, and what it read may
/// be a write the compositor has not answered. Once it has answered everything sent, the workspaces' flags are read again: a write that
/// was refused, or dropped after its resends, is forgotten with no event, and the pass that finds the flags changed asks again, so this
/// ends when they hold.
fn reread_when_answered(std: &Rc<Std>, g: &Gnoblin, workspaces: BTreeSet<Id>) {
    let ctx = std.st.borrow().ctx;
    let _ = g.flush(move |_answer| {
        if let Some(std) = state::find(ctx) {
            {
                let mut st = std.st.borrow_mut();
                for workspace in &workspaces {
                    if let Some(ws) = st.ws.get_mut(workspace) {
                        ws.scan_stale = true;
                    }
                }
            }
            schedule(&std);
        }
    });
}

/// Everything the pass needs to know about one workspace before it calls the layout.
struct Plan {
    cx: LayoutContext,
    swapped: Vec<(NodeId, NodeId)>,
    departed: Vec<NodeId>,
    arrived: Vec<NodeId>,
}

/// What a workspace that has to be computed again is computed from.
struct Compute {
    /// The area the layout last computed for, when it differs from the area now.
    moved_from: Option<Rect>,
    tiled: Vec<NodeId>,
}

fn pass_workspace(std: &Rc<Std>, g: &Gnoblin, workspace: Id) {
    // The layout and its revision first: asking for the revision calls into the shell, which must not happen inside a borrow.
    let Some(layout) = std.st.borrow().ws.get(&workspace).map(|ws| Rc::clone(&ws.layout)) else {
        return;
    };
    let revision = layout.revision();
    // A workspace whose windows, area, gaps and layout are as the last pass left them, and that is not due a compute, has nothing to
    // read and nothing to write: what it would read of every window is what `plan_workspace` read before (`workspace_at_rest`).
    if std.st.borrow().workspace_at_rest(workspace, revision) {
        return;
    }
    let Some(plan) = std.st.borrow_mut().plan_workspace(workspace, revision) else {
        return;
    };
    for (a, b) in &plan.swapped {
        layout.clients_swapped(*a, *b);
    }
    for client in &plan.departed {
        if let Some(next) = layout.client_removed(&plan.cx, *client) {
            let _ = g.window_handle(gnoblin_core::NodeId(next)).focus();
        }
    }
    let insets: HashMap<NodeId, crate::floating::Inset> = {
        let st = std.st.borrow();
        plan.cx
            .children
            .iter()
            .map(|child| (*child, st.inset_of(*child)))
            .collect()
    };
    // A window the shell gave floating bounds is where the user wants it: the floating layout leaves it alone.
    let floated: std::collections::HashSet<NodeId> = {
        let st = std.st.borrow();
        plan.arrived
            .iter()
            .copied()
            .filter(|client| st.has_floating_bounds(*client))
            .collect()
    };
    for client in &plan.arrived {
        layout.client_added(
            g,
            &plan.cx,
            *client,
            &crate::layout::Known {
                insets: &insets,
                floated: &floated,
            },
        );
    }
    {
        // Refit changed frames before placing their windows; new arrivals keep their current frame.
        let mut st = std.st.borrow_mut();
        let mut moved = false;
        for client in &plan.cx.children {
            if st.fit_pending(*client) && nodes::alive(&st.g, *client) && st.follow_frame(*client, &plan.cx.area) {
                moved = true;
            }
        }
        if moved && let Some(ws) = st.ws.get_mut(&workspace) {
            ws.dirty = true;
        }
    }
    let run = std.st.borrow_mut().begin_compute(workspace, &plan);
    if let Some(run) = run {
        if let Some(before) = &run.moved_from {
            layout.area_changed(before, &plan.cx.area);
        }
        let mut cx = plan.cx.clone();
        cx.children.clone_from(&run.tiled);
        if layout.wants_pointer() {
            cx.pointer = nodes::pointer(g);
        }
        std.st.borrow_mut().counters.computes += 1;
        match layout.compute(&cx) {
            Ok(placements) => std
                .st
                .borrow_mut()
                .apply_placements(&run.tiled, &placements, &plan.cx.area),
            Err(fail) => {
                std.st.borrow_mut().compute_failed = Some(fail.message.clone());
                g.log_at(
                    LogLevel::CoreError,
                    &format!(
                        "gnoblin-std: workspace {workspace}: the layout's compute failed: {}",
                        fail.message
                    ),
                );
            }
        }
    }
    // Hooks and compute may have changed the layout's own options: record them so they do not trigger another pass.
    let after = layout.revision();
    if let Some(ws) = std.st.borrow_mut().ws.get_mut(&workspace) {
        ws.revision = Some(after);
    }
}

impl State {
    /// The windows whose geometry changed since the listeners were last told: one event each. Nothing is looked at while no window's
    /// `geometry_pending` is set, which `geometry_waiting` says (`client_changed` sets both).
    pub(crate) fn flush_geometry(&mut self) {
        if !std::mem::take(&mut self.geometry_waiting) {
            return;
        }
        for client in self.clients.clone() {
            let pending = self
                .rec
                .get_mut(&client)
                .is_some_and(|rec| std::mem::take(&mut rec.geometry_pending));
            if !pending || !nodes::alive(&self.g, client) {
                continue;
            }
            let mut notice = Notice::new(
                abi::STD_EVENT_GEOMETRY_CHANGED,
                client,
                self.holders(abi::STD_COLLECTION_CLIENTS, client),
            );
            notice.rect = nodes::rect_of(&self.g, client);
            self.notices.push(notice);
        }
    }

    /// Whether the pass has nothing to do for `workspace`, which holds when everything `plan_workspace` and `begin_compute` look at is
    /// as the last pass saw it:
    ///
    /// - the layout's revision, the gaps, the area and whether a desk shows the workspace are read again here, as they are cheap;
    /// - the windows' `focused`, `fullscreen` and `maximised` flags are not: `scan_stale` says one changed (`scan_again`, called by
    ///   the handler of each of those properties), that a pass read staged values (`scan_all`), or that a pass read the windows that
    ///   cover the workspace other than it had noted them, which a write the compositor has not answered may have made
    ///   (`cover_unconfirmed`, read again once it has);
    /// - the members and their order are compared with what was scanned, and a window that arrived, left or swapped is the layout's to
    ///   be told of;
    /// - the workspace is shown and dirty, or shown on an area it did not compute for: `begin_compute` starts.
    ///
    /// A workspace with no area to lay out in is at rest when the revision and the gaps are as they were: `plan_workspace` returns
    /// before it looks at a window.
    pub(crate) fn workspace_at_rest(&self, workspace: Id, revision: u64) -> bool {
        let Some(ws) = self.ws.get(&workspace) else {
            return false;
        };
        if ws.revision != Some(revision) || (ws.gap_inner, ws.gap_outer) != ws.gaps_seen {
            return false;
        }
        let desk = self.desk_showing(workspace);
        let area = match desk {
            Some(desk) => self.area_of_desk(desk),
            None => self.screen_area(),
        };
        let Some(area) = area else {
            return true;
        };
        if ws.scan_stale
            || !ws.arrived.is_empty()
            || !ws.departed.is_empty()
            || !ws.swapped.is_empty()
            || ws.clients != ws.scanned
        {
            return false;
        }
        desk.is_none() || (!ws.dirty && ws.area == Some(area))
    }

    /// A flag of `client` that `plan_workspace` reads changed (`focused`, `fullscreen`, `maximised`): the next pass over its workspace
    /// reads the flags of every window again.
    pub(crate) fn scan_again(&mut self, client: NodeId) {
        if let Some(ws) = self
            .rec
            .get(&client)
            .and_then(|rec| rec.workspace)
            .and_then(|workspace| self.ws.get_mut(&workspace))
        {
            ws.scan_stale = true;
        }
    }

    /// Whether the room the client's frame takes is not the one it was last fitted for, so that `follow_frame` has something to do.
    /// It has nothing otherwise, whatever the window's flags are: it returns before it changes anything when the two are equal.
    fn fit_pending(&self, client: NodeId) -> bool {
        self.rec.get(&client).is_some_and(|rec| rec.fitted != Some(rec.inset))
    }

    /// Finds the windows whose placement the compositor never took: a window that still holds what it held before the last pass
    /// wrote its rectangle. Its workspace is laid out again, and what the pass last wrote to its visibility is forgotten.
    ///
    /// Only the windows in `unverified` are read: one whose rectangle was last seen to be the one written stays where it is until an
    /// event of its geometry says otherwise (`client_changed`), and a window whose placement has not been answered yet waits in the
    /// set. `all` reads every window that holds a written rectangle: a pass that reads staged values, and the turn after it.
    fn relay_lost_writes(&mut self, all: bool) {
        let watched: Vec<NodeId> = if all {
            self.clients.clone()
        } else {
            self.unverified.iter().copied().collect()
        };
        for client in watched {
            let Some(rec) = self.rec.get(&client) else {
                self.unverified.remove(&client);
                continue;
            };
            let Some((expected, before)) = rec.expected.zip(rec.before) else {
                self.unverified.remove(&client);
                continue;
            };
            if !rec.acked || !nodes::alive(&self.g, client) {
                continue;
            }
            let now = nodes::rect_of(&self.g, client);
            let Some(rec) = self.rec.get_mut(&client) else {
                continue;
            };
            if now == expected {
                rec.lost = 0;
                self.unverified.remove(&client);
            } else if now == before {
                rec.lost += 1;
                if rec.lost > LOST_RETRIES {
                    rec.expected = None;
                    rec.before = None;
                    self.unverified.remove(&client);
                    continue;
                }
                rec.written_visible = None;
                if let Some(workspace) = rec.workspace.and_then(|id| self.ws.get_mut(&id)) {
                    workspace.dirty = true;
                }
            } else {
                // Somebody else moved it since: not a lost write, and not this pass's to undo.
                rec.expected = None;
                rec.before = None;
                rec.lost = 0;
                self.unverified.remove(&client);
            }
        }
    }

    /// Decides what a workspace needs: marks it dirty when its layout's options, its gaps, its focus or its covering windows
    /// changed, and gathers what the layout is told. `None` when the workspace is gone or has no area to be laid out in.
    fn plan_workspace(&mut self, workspace: Id, revision: u64) -> Option<Plan> {
        let desk = self.desk_showing(workspace);
        let area = match desk {
            Some(desk) => self.area_of_desk(desk),
            None => self.screen_area(),
        };
        let children = self.ws.get(&workspace)?.clients.clone();
        let ws = self.ws.get_mut(&workspace)?;
        if ws.revision != Some(revision) {
            ws.dirty = true;
            ws.revision = Some(revision);
        }
        if (ws.gap_inner, ws.gap_outer) != ws.gaps_seen {
            ws.gaps_seen = (ws.gap_inner, ws.gap_outer);
            ws.dirty = true;
        }
        let area = area?;
        // The windows are read only for a workspace with an area to be laid out in, and what is read is what `workspace_at_rest` takes
        // as known until a flag changes or the members do.
        let focused = children
            .iter()
            .copied()
            .find(|client| nodes::flag(&self.g, *client, "focused"));
        let covering = children
            .iter()
            .filter(|client| {
                nodes::flag(&self.g, **client, "fullscreen") || nodes::flag(&self.g, **client, "maximised")
            })
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        ws.scan_stale = false;
        ws.scanned.clone_from(&children);
        if focused != ws.focus_seen {
            ws.focus_seen = focused;
            ws.dirty = true;
        }
        if covering != ws.cover_seen {
            ws.cover_seen = covering;
            ws.dirty = true;
            self.cover_unconfirmed.insert(workspace);
        }
        let cx = LayoutContext {
            workspace,
            children,
            area,
            gap_inner: ws.gap_inner,
            gap_outer: ws.gap_outer,
            focused,
            pointer: None,
        };
        let departed = std::mem::take(&mut ws.departed);
        let swapped = std::mem::take(&mut ws.swapped);
        let arrived: Vec<NodeId> = std::mem::take(&mut ws.arrived)
            .into_iter()
            .filter(|client| {
                nodes::alive(&self.g, *client) && self.rec.get(client).and_then(|rec| rec.workspace) == Some(workspace)
            })
            .collect();
        Some(Plan {
            cx,
            swapped,
            departed,
            arrived,
        })
    }

    /// Starts a compute when the workspace is shown and dirty or its area moved: records the area, refits the windows to a new
    /// one, and says which clients the layout places. Floating, fullscreen and maximised windows keep their own placement, and
    /// dialogs stay with their parent and are never tiled.
    fn begin_compute(&mut self, workspace: Id, plan: &Plan) -> Option<Compute> {
        self.desk_showing(workspace)?;
        let area = plan.cx.area;
        let ws = self.ws.get_mut(&workspace)?;
        let area_moved = ws.area != Some(area);
        if !(ws.dirty || area_moved) {
            return None;
        }
        let before = ws.area;
        ws.dirty = false;
        ws.area = Some(area);
        let moved_from = before.filter(|before| *before != area);
        if moved_from.is_some() {
            // When the work area changes, maximised windows fill it and floating windows move inside it.
            for client in plan.cx.children.clone() {
                if nodes::alive(&self.g, client) {
                    self.refit_client(client, &area);
                }
            }
        }
        let tiled = plan
            .cx
            .children
            .iter()
            .copied()
            .filter(|client| {
                !self.rec.get(client).is_some_and(|rec| rec.floating)
                    && !nodes::flag(&self.g, *client, "fullscreen")
                    && !nodes::flag(&self.g, *client, "maximised")
                    && nodes::node_ref(&self.g, *client, "parent_window").is_none()
            })
            .collect();
        Some(Compute { moved_from, tiled })
    }

    /// The rectangle a placement gives `client`: the content inside the frame the placement's bounds describe, moved by the offset of
    /// its transform's position, to whole pixels. A placement places the FRAME (the content with the room its frame takes on each
    /// side), so the content is what is left of the bounds. With no bounds the frame is the window's own size, frame included,
    /// centred in `area`.
    fn placement_bounds(&self, client: NodeId, placement: &Placement, area: &Rect) -> Option<Rect> {
        let inset = self.inset_of(client);
        let Some((dx, dy)) = placement.offset else {
            return placement.bounds.map(|bounds| shrink(&bounds, &inset));
        };
        let own = nodes::rect_of(&self.g, client);
        let room = grow(&Rect::new(0.0, 0.0, own.width, own.height), &inset);
        let base = placement.bounds.unwrap_or_else(|| {
            Rect::new(
                area.x + (area.width - room.width) / 2.0,
                area.y + (area.height - room.height) / 2.0,
                room.width,
                room.height,
            )
        });
        // JavaScript's `Math.round`: halves go up.
        let round = |value: f64| (value + 0.5).floor();
        Some(shrink(
            &Rect::new(round(base.x + dx), round(base.y + dy), base.width, base.height),
            &inset,
        ))
    }

    /// Writes what a layout's placements say: the rectangle, whether the window is tiled, hidden, and where it is in depth. A
    /// client the layout gave no placement is as if placed with the defaults, so a window that leaves a tiled layout for a floating
    /// one stops being tiled. Only what differs from the client's value is written.
    fn apply_placements(&mut self, tiled: &[NodeId], placements: &[Placement], area: &Rect) {
        let g = self.g.clone();
        let placed: HashMap<NodeId, &Placement> =
            placements.iter().map(|placement| (placement.node, placement)).collect();
        for client in tiled {
            let placement = placed.get(client).copied();
            let bounds = placement.and_then(|placement| self.placement_bounds(*client, placement, area));
            if let Some(bounds) = bounds {
                let before = nodes::rect_of(&g, *client);
                nodes::write_rect(&g, *client, &bounds);
                let pass = self.counters.layout_passes;
                if let Some(rec) = self.rec.get_mut(client) {
                    rec.before = (before != bounds).then_some(before);
                    rec.expected = (before != bounds).then_some(bounds);
                    rec.lost = 0;
                    rec.acked = false;
                    rec.write_pass = pass;
                }
                if before != bounds {
                    self.placements_written = true;
                    // Whether the compositor took it is told when it has answered (`relay_lost_writes`).
                    self.unverified.insert(*client);
                } else {
                    self.unverified.remove(client);
                }
            }
            nodes::write_flag(&g, *client, "tiled", placement.is_some_and(|placement| placement.tiled));
            let wanted = placement.is_none_or(|placement| placement.visible);
            if let Some(rec) = self.rec.get_mut(client) {
                rec.shown_by_layout = Some(wanted);
            }
            nodes::write_num(&g, *client, "z", placement.map_or(0.0, |placement| placement.z));
            nodes::write_triple(
                &g,
                *client,
                "rotation",
                placement.map_or([0.0; 3], |placement| placement.rotation),
                [0.0; 3],
            );
            nodes::write_triple(
                &g,
                *client,
                "scale",
                placement.map_or([1.0; 3], |placement| placement.scale),
                [1.0; 3],
            );
        }
    }

    /// Records the intended visibility and writes it only when the value changes.
    fn write_shown(&mut self, client: NodeId, shown: bool) {
        if let Some(rec) = self.rec.get_mut(&client) {
            rec.written_visible = Some(shown);
        }
        nodes::write_flag(&self.g, client, "visible", shown);
    }

    /// Shows the windows of the shown workspace (and of a workspace a slide is leaving) that their layout shows, and hides the
    /// rest. A window is written only when what it should be differs from what this library last wrote.
    fn sync_visibility(&mut self) {
        for workspace in self.workspaces.clone() {
            let shown = self.is_shown(workspace) || self.holding.contains_key(&workspace);
            let clients = self.ws.get(&workspace).map(|ws| ws.clients.clone()).unwrap_or_default();
            for client in clients {
                let Some(rec) = self.rec.get(&client) else {
                    continue;
                };
                let want = shown && rec.shown_by_layout.unwrap_or(true);
                if rec.written_visible.unwrap_or(true) != want && nodes::alive(&self.g, client) {
                    self.write_shown(client, want);
                }
            }
        }
    }

    /// Starts the slides `show` asked for, once the layout has put every window at rest: the windows of the workspace that comes in
    /// start a screen's width away on the side it comes from, and the windows of the one that leaves move a screen's width the other
    /// way. Returns the writes that carry them to where they rest.
    fn start_slides(&mut self) -> Vec<Play> {
        let mut plays = Vec::new();
        for slide in std::mem::take(&mut self.slides) {
            // The slide is as long as the output the incoming workspace is on is wide.
            let Some(area) = self.work_area_of_workspace(slide.to) else {
                continue;
            };
            let travel = slide.direction * area.width;
            let mut moves = Vec::new();
            for client in self.ws.get(&slide.to).map(|ws| ws.clients.clone()).unwrap_or_default() {
                let rest = nodes::number(&self.g, client, "x");
                nodes::write_num(&self.g, client, "x", rest + travel * (1.0 - slide.progress));
                moves.push((client, rest));
            }
            let mut resting = Vec::new();
            for client in self
                .ws
                .get(&slide.from)
                .map(|ws| ws.clients.clone())
                .unwrap_or_default()
            {
                let rest = nodes::number(&self.g, client, "x");
                resting.push((client, rest));
                if slide.progress > 0.0 {
                    nodes::write_num(&self.g, client, "x", rest - travel * slide.progress);
                }
                moves.push((client, rest - travel));
            }
            *self.holding.entry(slide.from).or_insert(0) += 1;
            plays.push(Play {
                moves,
                duration_ms: slide.duration_ms,
                from: slide.from,
                resting,
            });
        }
        plays
    }
}

/// Carries a slide's windows to where they rest, with the compositor's own tween, and hides the ones that left once it is over.
fn play_slide(std: &Rc<Std>, g: &Gnoblin, play: Play) {
    let ctx = std.st.borrow().ctx;
    let duration = play.duration_ms;
    let moves = play.moves;
    // The offsets were written this turn; the timed writes start from them once they have been sent: each window's `x` goes to where
    // it rests along the CSS `ease-out` curve, all of them as one transition.
    let started = g.flush(move |_serial| {
        if let Some(std) = state::find(ctx) {
            let samples = Easing::EaseOut.samples();
            {
                let mut st = std.st.borrow_mut();
                let entries: Vec<Staged> = moves
                    .iter()
                    .filter(|(client, _)| nodes::alive(&st.g, *client))
                    .map(|(client, target)| Staged {
                        node: *client,
                        key: "x".to_owned(),
                        value: Prop::Number(*target),
                        curve: Some(Curve {
                            duration_ms: duration,
                            samples: samples.clone(),
                        }),
                    })
                    .collect();
                let options = Options {
                    duration_ms: duration,
                    delay_ms: 0.0,
                    samples: samples.clone(),
                    autoplay: true,
                };
                let _ = st.transition_commit(entries, &options);
            }
            std.drain();
        }
    });
    if let Err(error) = started {
        g.log_at(
            LogLevel::Warn,
            &format!("gnoblin-std: a workspace slide could not start: {error}"),
        );
    }
    let (from, resting) = (play.from, play.resting);
    let ms = play.duration_ms.round().clamp(1.0, f64::from(u32::MAX)) as u32;
    let ended = g.after(ms, move || {
        let Some(std) = state::find(ctx) else {
            return;
        };
        {
            let mut st = std.st.borrow_mut();
            let left = st.holding.get(&from).copied().unwrap_or(1).saturating_sub(1);
            if left > 0 {
                st.holding.insert(from, left);
                return;
            }
            st.holding.remove(&from);
            for (client, rest) in &resting {
                let here = st.rec.get(client).and_then(|rec| rec.workspace) == Some(from);
                if nodes::alive(&st.g, *client) && here && !st.is_shown(from) {
                    st.write_shown(*client, false);
                    nodes::write_num(&st.g, *client, "x", *rest);
                }
            }
        }
        std.drain();
    });
    if let Err(error) = ended {
        g.log_at(
            LogLevel::Warn,
            &format!("gnoblin-std: a workspace slide cannot end: {error}"),
        );
    }
}
