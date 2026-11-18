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

use std::collections::HashMap;
use std::rc::Rc;

use gnoblin_core::{Gnoblin, LogLevel, Rect};
use gnoblin_std_abi as abi;

use crate::animation::easing::Easing;
use crate::animation::engine::Options;
use crate::animation::staging::{Curve, Prop, Staged};
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
    }
    std.drain();
    layout_pass(std, &g);
    std.st.borrow_mut().apply_styles();
    let wrote = std::mem::take(&mut std.st.borrow_mut().placements_written);
    if wrote {
        acknowledge_writes(std, &g);
    }
    std.drain();
}

/// The passes of a turn that a staged transition runs against its staged state, so what they write is staged with the callback's:
/// the layout pass, then the style pass.
pub(crate) fn settle(std: &Rc<Std>) {
    let g = std.g();
    layout_pass(std, &g);
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
        st.relay_lost_writes();
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

/// Everything the pass needs to know about one workspace before it calls the layout.
struct Plan {
    cx: LayoutContext,
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
    let Some(plan) = std.st.borrow_mut().plan_workspace(workspace, revision) else {
        return;
    };
    for client in &plan.departed {
        if let Some(next) = layout.client_removed(&plan.cx, *client) {
            let _ = g.window_handle(gnoblin_core::NodeId(next)).focus();
        }
    }
    for client in &plan.arrived {
        layout.client_added(g, &plan.cx, *client);
    }
    {
        // Refit changed frames before placing their windows; new arrivals keep their current frame.
        let mut st = std.st.borrow_mut();
        let mut moved = false;
        for client in &plan.cx.children {
            if nodes::alive(&st.g, *client) && st.follow_frame(*client, &plan.cx.area) {
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
            Err(fail) => g.log_at(
                LogLevel::CoreError,
                &format!(
                    "gnoblin-std: workspace {workspace}: the layout's compute failed: {}",
                    fail.message
                ),
            ),
        }
    }
    // Hooks and compute may have changed the layout's own options: record them so they do not trigger another pass.
    let after = layout.revision();
    if let Some(ws) = std.st.borrow_mut().ws.get_mut(&workspace) {
        ws.revision = Some(after);
    }
}

impl State {
    /// The windows whose geometry changed since the listeners were last told: one event each.
    pub(crate) fn flush_geometry(&mut self) {
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

    /// Finds the windows whose placement the compositor never took: a window that still holds what it held before the last pass
    /// wrote its rectangle. Its workspace is laid out again, and what the pass last wrote to its visibility is forgotten.
    fn relay_lost_writes(&mut self) {
        for client in self.clients.clone() {
            let Some((expected, before)) = self
                .rec
                .get(&client)
                .filter(|rec| rec.acked)
                .and_then(|rec| rec.expected.zip(rec.before))
                .filter(|_| nodes::alive(&self.g, client))
            else {
                continue;
            };
            let now = nodes::rect_of(&self.g, client);
            let Some(rec) = self.rec.get_mut(&client) else {
                continue;
            };
            if now == expected {
                rec.lost = 0;
            } else if now == before {
                rec.lost += 1;
                if rec.lost > LOST_RETRIES {
                    rec.expected = None;
                    rec.before = None;
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
        if focused != ws.focus_seen {
            ws.focus_seen = focused;
            ws.dirty = true;
        }
        if covering != ws.cover_seen {
            ws.cover_seen = covering;
            ws.dirty = true;
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
        let arrived: Vec<NodeId> = std::mem::take(&mut ws.arrived)
            .into_iter()
            .filter(|client| {
                nodes::alive(&self.g, *client) && self.rec.get(client).and_then(|rec| rec.workspace) == Some(workspace)
            })
            .collect();
        Some(Plan { cx, departed, arrived })
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

    /// The rectangle a placement gives `client`: its bounds moved by the offset of its transform's position, to whole pixels.
    /// With no bounds the rectangle is the window's own size centred in `area`.
    fn placement_bounds(&self, client: NodeId, placement: &Placement, area: &Rect) -> Option<Rect> {
        let Some((dx, dy)) = placement.offset else {
            return placement.bounds;
        };
        let own = nodes::rect_of(&self.g, client);
        let base = placement.bounds.unwrap_or_else(|| {
            Rect::new(
                area.x + (area.width - own.width) / 2.0,
                area.y + (area.height - own.height) / 2.0,
                own.width,
                own.height,
            )
        });
        // JavaScript's `Math.round`: halves go up.
        let round = |value: f64| (value + 0.5).floor();
        Some(Rect::new(
            round(base.x + dx),
            round(base.y + dy),
            base.width,
            base.height,
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
                if nodes::alive(&self.g, client) && rec.written_visible.unwrap_or(true) != want {
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
