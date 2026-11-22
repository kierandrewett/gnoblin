//! Desks: which workspace each output shows, and the policies that decide it.
//!
//! A *desk* is what an output shows: one workspace at a time. Every output that can show a workspace (it is on and mirrors nothing)
//! has a desk; a workspace is on one desk and may span several outputs. A desk whose output went waits *parked* for one to return; with
//! no outputs at all one unparked desk keeps a workspace shown. This is `host/desks.ts`, `show.ts` and `lifecycle.ts` of the
//! TypeScript library, with the display manager's mirror and span modes as an [`Policy`] of the library.

use gnoblin_core::{Rect, Value};
use gnoblin_std_abi as abi;

use crate::error::{Fail, Res};
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode};
use crate::layout::LayoutObj;
use crate::nodes;
use crate::state::{Id, NodeId, Notice, State};
use crate::workspace_objects::{NOTICE_HIDDEN, NOTICE_SHOWN};

/// The workspace shown by an output, and how its windows are handled when outputs come and go.
#[derive(Clone, Debug)]
pub(crate) struct Desk {
    pub(crate) id: u64,
    /// The output using this desk, or `None` when it has none.
    pub(crate) output: Option<NodeId>,
    /// The workspace currently shown, or `None` when empty.
    pub(crate) shown: Option<Id>,
    /// The desk waits hidden for an output to return.
    pub(crate) parked: bool,
    /// The outputs this desk's one workspace is stretched over (its own first), or `None` for a desk of one output.
    pub(crate) spans: Option<Vec<NodeId>>,
    /// The desk whose workspace covers this desk's output too: such a desk shows no workspace of its own.
    pub(crate) spanned_by: Option<u64>,
}

/// A slide between workspaces that the next layout pass starts.
#[derive(Clone, Debug)]
pub(crate) struct Slide {
    pub(crate) from: Id,
    pub(crate) to: Id,
    /// 1 when `to` comes after `from`, so the windows move left.
    pub(crate) direction: f64,
    pub(crate) duration_ms: f64,
    pub(crate) progress: f64,
}

/// How the outputs share workspaces and where workspaces come from.
#[derive(Clone, Debug)]
pub(crate) struct Policy {
    pub(crate) outputs: u32,
    pub(crate) lead: NodeId,
    pub(crate) workspaces: u32,
    pub(crate) prefix: String,
    pub(crate) gaps: (f64, f64),
    pub(crate) placement: PlaceMode,
    pub(crate) step: f64,
    /// The outputs the library asked the compositor to mirror, so a change of policy can ask it to stop.
    pub(crate) mirrored: Vec<NodeId>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            outputs: abi::STD_OUTPUTS_SEPARATE,
            lead: 0,
            workspaces: abi::STD_WORKSPACES_MANUAL,
            prefix: "workspace-".to_owned(),
            gaps: (0.0, 0.0),
            placement: PlaceMode::FreeSpace,
            step: DEFAULT_CASCADE_STEP,
            mirrored: Vec::new(),
        }
    }
}

/// An output's rectangle in the compositor's coordinates, `None` for one with no size yet.
pub(crate) fn output_rect(g: &gnoblin_core::Gnoblin, output: NodeId) -> Option<Rect> {
    let pick = |own: &str, screen: &str| {
        let value = nodes::number(g, output, own);
        if value != 0.0 {
            value
        } else {
            nodes::number(g, output, screen)
        }
    };
    let width = pick("width", "screen_width");
    let height = pick("height", "screen_height");
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    Some(Rect::new(pick("x", "screen_x"), pick("y", "screen_y"), width, height))
}

impl State {
    fn desk_index(&self, id: u64) -> Option<usize> {
        self.desks.iter().position(|desk| desk.id == id)
    }

    /// The desk with this id.
    pub(crate) fn desk(&self, id: u64) -> Option<&Desk> {
        self.desks.iter().find(|desk| desk.id == id)
    }

    /// The desk that decides what `desk` shows: itself, or the desk spanning over it.
    fn effective_desk(&self, desk: u64) -> u64 {
        self.desk(desk).and_then(|desk| desk.spanned_by).unwrap_or(desk)
    }

    /// The desk of `output`, if it has one.
    pub(crate) fn desk_of_output(&self, output: NodeId) -> Option<u64> {
        self.desks
            .iter()
            .find(|desk| desk.output == Some(output))
            .map(|desk| desk.id)
    }

    /// The desk that shows `workspace`.
    pub(crate) fn desk_showing(&self, workspace: Id) -> Option<u64> {
        self.desks
            .iter()
            .find(|desk| desk.shown == Some(workspace))
            .map(|desk| desk.id)
    }

    /// Whether an output (or the tree's one output-less desk) shows `workspace`. A parked desk shows nothing.
    pub(crate) fn is_shown(&self, workspace: Id) -> bool {
        self.desk_showing(workspace)
            .and_then(|id| self.desk(id))
            .is_some_and(|desk| !desk.parked)
    }

    /// The output that shows `workspace`, or `None`.
    pub(crate) fn output_showing(&self, workspace: Id) -> Option<NodeId> {
        self.desk_showing(workspace)
            .and_then(|id| self.desk(id))
            .filter(|desk| !desk.parked)
            .and_then(|desk| desk.output)
    }

    /// The outputs that have a desk, in the order the compositor made them.
    pub(crate) fn desk_outputs(&self) -> Vec<NodeId> {
        self.outputs
            .iter()
            .copied()
            .filter(|output| self.desk_of_output(*output).is_some())
            .collect()
    }

    /// The output flagged primary, or the first one.
    pub(crate) fn primary_output(&self) -> Option<NodeId> {
        let list = self.desk_outputs();
        list.iter()
            .copied()
            .find(|output| nodes::flag(&self.g, *output, "primary"))
            .or_else(|| list.first().copied())
    }

    /// The desk of the primary output; with no output, the desk a tree without one keeps.
    fn primary_desk(&self) -> Option<u64> {
        match self.primary_output() {
            Some(output) => self.desk_of_output(output),
            None => self.desks.iter().find(|desk| desk.output.is_none()).map(|desk| desk.id),
        }
    }

    /// The output the pointer is over, among those with a desk.
    fn pointer_screen(&self) -> Option<NodeId> {
        let seat = nodes::seat(&self.g)?;
        // The seat names the screen the pointer is on; its coordinates are the fallback for a tree whose seat does not.
        if let Ok(Some(screen)) = seat.get_node("screen") {
            return self.outputs.contains(&screen.id().0).then(|| screen.id().0);
        }
        let (x, y) = (seat.get_f64("pointer_x").ok()?, seat.get_f64("pointer_y").ok()?);
        self.outputs.iter().copied().find(|output| {
            output_rect(&self.g, *output)
                .is_some_and(|rect| x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height)
        })
    }

    /// The desk a new window, a key and a plain `show` act on: the one under the pointer, else the primary one.
    pub(crate) fn current_desk(&self) -> Option<u64> {
        let under = self.pointer_screen().and_then(|output| self.desk_of_output(output));
        under
            .or_else(|| self.primary_desk())
            .map(|desk| self.effective_desk(desk))
    }

    /// A desk for a tree that has no output, so the first workspace is on show whatever the compositor has.
    pub(crate) fn ensure_desk(&mut self) {
        if self.desks.is_empty() {
            self.push_desk(None);
        }
    }

    fn push_desk(&mut self, output: Option<NodeId>) -> u64 {
        let id = crate::state::next_id();
        self.desks.push(Desk {
            id,
            output,
            shown: None,
            parked: false,
            spans: None,
            spanned_by: None,
        });
        id
    }

    /// Gives each desk that shows nothing the first workspace no desk shows.
    pub(crate) fn fill_desks(&mut self) {
        let ids: Vec<u64> = self.desks.iter().map(|desk| desk.id).collect();
        for id in ids {
            let Some(desk) = self.desk(id) else {
                continue;
            };
            if desk.shown.is_some() || desk.spanned_by.is_some() {
                continue;
            }
            let free = self
                .workspaces
                .iter()
                .copied()
                .find(|workspace| self.desk_showing(*workspace).is_none());
            if let Some(free) = free {
                self.set_shown(id, Some(free));
            }
        }
    }

    /// Makes `next` the workspace `desk` shows.
    pub(crate) fn set_shown(&mut self, desk: u64, next: Option<Id>) {
        let Some(at) = self.desk_index(desk) else {
            return;
        };
        let before = self.desks[at].shown;
        if before == next {
            return;
        }
        self.desks[at].shown = next;
        // The objects of the object protocol hear which workspace stopped being shown and which started, on which output.
        let output = self.desks[at].output.unwrap_or(0);
        for (kind, workspace) in [(NOTICE_HIDDEN, before), (NOTICE_SHOWN, next)] {
            if let Some(workspace) = workspace {
                let mut notice = Notice::new(kind, workspace, Vec::new());
                notice.output = output;
                self.notices.push(notice);
            }
        }
        if let Some(workspace) = next.and_then(|id| self.ws.get_mut(&id)) {
            workspace.dirty = true;
        }
        for changed in [before, next].into_iter().flatten() {
            if self.ws.get(&changed).is_some_and(|ws| ws.attached) {
                self.recompute(abi::STD_COLLECTION_WORKSPACES, changed);
            }
        }
    }

    /// The id of the workspace the primary output shows, which is the one the desktop shows now.
    fn primary_workspace_id(&self) -> Option<String> {
        let desk = self.primary_desk()?;
        let shown = self.desk(self.effective_desk(desk))?.shown?;
        self.ws.get(&shown).map(|ws| ws.id.clone())
    }

    /// Writes the id of the workspace the primary output shows to the seat's `active_workspace`, which the compositor serves to a client of
    /// `ext-workspace-v1` as that workspace's `active` state. It is written when it changes, and once
    /// a shell uses the workspace classes; a seat without the property, or none, is left alone.
    pub(crate) fn publish_active_workspace(&mut self) {
        if !self.desktop || !self.publish_active {
            return;
        }
        let Some(id) = self.primary_workspace_id() else {
            return;
        };
        if self.active_written.as_deref() == Some(id.as_str()) {
            return;
        }
        if let Some(seat) = nodes::seat(&self.g)
            && seat.set("active_workspace", id.clone()).is_ok()
        {
            self.active_written = Some(id);
        }
    }

    // ---- work areas -----------------------------------------------------------------------------------------------------

    /// What a layout tiles in on `output`: its rectangle less the room the shell's own bars take from the top and the bottom
    /// (`output_set_insets`; the panels themselves are the shell's UI, not this library's).
    pub(crate) fn area_of(&self, output: NodeId) -> Option<Rect> {
        let rect = output_rect(&self.g, output)?;
        let (top, bottom) = self.insets.get(&output).copied().unwrap_or((0.0, 0.0));
        Some(Rect::new(
            rect.x,
            rect.y + top,
            rect.width,
            (rect.height - top - bottom).max(1.0),
        ))
    }

    /// The one area a spanning desk's workspace is laid out in: the bounding box of the work areas of the outputs it covers.
    fn span_area(&self, outputs: &[NodeId]) -> Option<Rect> {
        let areas: Vec<Rect> = outputs.iter().filter_map(|output| self.area_of(*output)).collect();
        if areas.is_empty() {
            return None;
        }
        let left = areas.iter().map(|a| a.x).fold(f64::INFINITY, f64::min);
        let top = areas.iter().map(|a| a.y).fold(f64::INFINITY, f64::min);
        let right = areas.iter().map(|a| a.x + a.width).fold(f64::NEG_INFINITY, f64::max);
        let bottom = areas.iter().map(|a| a.y + a.height).fold(f64::NEG_INFINITY, f64::max);
        Some(Rect::new(left, top, right - left, bottom - top))
    }

    /// The area of the output a desk shows on: `None` for a desk with no output, a parked one, or an output with no size.
    pub(crate) fn area_of_desk(&self, desk: u64) -> Option<Rect> {
        let desk = self.desk(desk)?;
        let output = desk.output?;
        if desk.parked {
            return None;
        }
        match &desk.spans {
            None => self.area_of(output),
            Some(spans) => self.span_area(spans),
        }
    }

    /// The area windows are laid out in on the primary output.
    pub(crate) fn screen_area(&self) -> Option<Rect> {
        self.primary_output().and_then(|output| self.area_of(output))
    }

    /// The work area of the output that shows `workspace` (all of a span), or the primary output's when none does.
    pub(crate) fn work_area_of_workspace(&self, workspace: Id) -> Option<Rect> {
        match self.desk_showing(workspace).and_then(|id| self.desk(id)) {
            Some(desk) if desk.output.is_some() && !desk.parked => self.area_of_desk(desk.id),
            _ => self.screen_area(),
        }
    }

    // ---- outputs come and go -------------------------------------------------------------------------------------------

    /// Whether an output can show a workspace: it is on and mirrors nothing.
    pub(crate) fn desk_capable(&self, output: NodeId) -> bool {
        let g = &self.g;
        nodes::flag_or(g, output, "enabled", true) && nodes::text(g, output, "mirror_of").is_empty()
    }

    /// Gives an output a desk, reusing one without an output or creating one. Returns whether the reused desk was parked.
    pub(crate) fn give_desk(&mut self, output: NodeId) -> bool {
        let desk = match self.desks.iter().find(|desk| desk.output.is_none()).map(|desk| desk.id) {
            Some(desk) => desk,
            None => self.push_desk(None),
        };
        let Some(at) = self.desk_index(desk) else {
            return false;
        };
        let was_parked = self.desks[at].parked;
        self.desks[at].output = Some(output);
        self.desks[at].parked = false;
        self.fill_desks();
        if let Some(workspace) = self.desks[at].shown.and_then(|id| self.ws.get_mut(&id)) {
            workspace.dirty = true;
        }
        was_parked
    }

    /// Takes an output's desk away: its windows move to another output's workspace, or the desk parks.
    pub(crate) fn take_desk(&mut self, output: NodeId) {
        let Some(id) = self.desk_of_output(output) else {
            return;
        };
        let Some(at) = self.desk_index(id) else {
            return;
        };
        self.desks[at].output = None;
        let survivor = self
            .desks
            .iter()
            .find(|candidate| candidate.id != id && candidate.output.is_some())
            .map(|candidate| candidate.id);
        let shown = self.desks[at].shown;
        if let Some(survivor) = survivor {
            self.desks.retain(|desk| desk.id != id);
            let target = self
                .desk(survivor)
                .and_then(|desk| desk.shown)
                .or_else(|| self.workspaces.first().copied());
            if let (Some(shown), Some(target)) = (shown, target)
                && shown != target
            {
                let area = self.area_of_desk(survivor);
                for client in self.ws.get(&shown).map(|ws| ws.clients.clone()).unwrap_or_default() {
                    if nodes::node_ref(&self.g, client, "parent_window").is_none() {
                        self.move_client(client, Some(target), true);
                        if let Some(area) = &area {
                            self.refit_client(client, area);
                        }
                    }
                }
            }
            if let Some(workspace) = shown.and_then(|id| self.ws.get_mut(&id)) {
                workspace.dirty = true;
            }
        } else {
            self.desks[at].parked = true;
            let holder = nodes::seat(&self.g).and_then(|_| nodes::focused_window(&self.g, &self.clients));
            let held_here = holder
                .is_some_and(|holder| shown.is_some() && self.rec.get(&holder).and_then(|rec| rec.workspace) == shown);
            if held_here && let Some(seat) = nodes::seat(&self.g) {
                // No output can hold the focus any more.
                let _ = seat.set("focus", gnoblin_core::NodeId::NIL);
            }
        }
        for desk in &mut self.desks {
            if desk.spanned_by == Some(id) {
                desk.spanned_by = None;
            }
        }
    }

    /// An output appeared (or became one that can show a workspace).
    pub(crate) fn output_added(&mut self, output: NodeId) -> bool {
        let mut returned = false;
        if self.desk_capable(output) && self.desk_of_output(output).is_none() {
            returned = self.give_desk(output);
        }
        self.apply_policies();
        returned
    }

    /// An output that came back to a parked desk was announced: restore the windows that waited hidden by focusing one, if a seat
    /// exists and none has the keyboard.
    pub(crate) fn focus_returned(&mut self, output: NodeId) {
        let Some(seat) = nodes::seat(&self.g) else {
            return;
        };
        if nodes::focused_window(&self.g, &self.clients).is_some() {
            return;
        }
        let first = self
            .desk_of_output(output)
            .and_then(|desk| self.desk(desk))
            .and_then(|desk| desk.shown)
            .and_then(|workspace| self.ws.get(&workspace))
            .and_then(|ws| {
                ws.clients
                    .iter()
                    .copied()
                    .find(|client| nodes::alive(&self.g, *client) && !nodes::flag(&self.g, *client, "minimised"))
            });
        if let Some(first) = first {
            let _ = seat.set("focus", gnoblin_core::NodeId(first));
        }
    }

    /// An output is gone, or can no longer show a workspace.
    pub(crate) fn output_left(&mut self, output: NodeId) {
        self.take_desk(output);
        self.policy.mirrored.retain(|one| *one != output);
        self.apply_policies();
        self.prune_policy_workspaces();
    }

    /// An output's `enabled` or `mirror_of` changed: it gains or loses its desk.
    pub(crate) fn output_changed(&mut self, output: NodeId) {
        let capable = self.desk_capable(output);
        let has = self.desk_of_output(output).is_some();
        if capable && !has {
            self.give_desk(output);
            self.apply_policies();
        } else if !capable && has {
            self.take_desk(output);
            self.prune_policy_workspaces();
        }
    }

    // ---- showing ---------------------------------------------------------------------------------------------------------

    /// Shows a workspace on an output, swapping it with the one that output showed; the windows slide when `duration_ms` asks.
    pub(crate) fn show_workspace(
        &mut self,
        workspace: Id,
        output: Option<NodeId>,
        duration_ms: f64,
        progress: f64,
    ) -> Res<()> {
        if !self.workspace(workspace)?.attached {
            return Err(Fail::refused("add the workspace to the list first"));
        }
        if !duration_ms.is_finite() || duration_ms < 0.0 {
            return Err(Fail::type_error(
                "duration must be a finite number of milliseconds of at least 0",
            ));
        }
        if !(0.0..=1.0).contains(&progress) {
            return Err(Fail::type_error("progress must be a number from 0 to 1"));
        }
        if let Some(output) = output
            && !nodes::alive(&self.g, output)
        {
            return Err(Fail::refused(format!("output {output} is gone")));
        }
        self.ensure_desk();
        let own = match output {
            None => self.current_desk(),
            Some(output) => self.desk_of_output(output),
        };
        let Some(desk) = own.map(|desk| self.effective_desk(desk)) else {
            return Err(Fail::refused("there is no output to show a workspace on"));
        };
        let from = self.desk(desk).and_then(|desk| desk.shown);
        // A workspace can appear on only one desk, so swap the two desks' shown workspaces.
        let other = self
            .desks
            .iter()
            .find(|candidate| candidate.id != desk && candidate.shown == Some(workspace))
            .map(|candidate| candidate.id);
        self.set_shown(desk, Some(workspace));
        if let Some(other) = other {
            self.set_shown(other, from);
        }
        if let Some(from) = from
            && from != workspace
            && duration_ms > 0.0
        {
            let order = |id: Id| self.workspaces.iter().position(|one| *one == id);
            self.slides.push(Slide {
                from,
                to: workspace,
                direction: if order(workspace) > order(from) { 1.0 } else { -1.0 },
                duration_ms,
                progress,
            });
        }
        Ok(())
    }

    /// Moves the keyboard `step` windows along the workspace's windows: nothing moves past either end.
    pub(crate) fn walk(&mut self, workspace: Id, step: i64) -> Res<()> {
        let list = self.workspace(workspace)?.clients.clone();
        let focused = nodes::focused_window(&self.g, &self.clients);
        let Some(at) = list.iter().position(|client| Some(*client) == focused) else {
            return Ok(());
        };
        let Some(next) = i64::try_from(at)
            .ok()
            .and_then(|at| usize::try_from(at + step).ok())
            .and_then(|i| list.get(i))
        else {
            return Ok(());
        };
        if nodes::alive(&self.g, *next)
            && let Some(seat) = nodes::seat(&self.g)
        {
            let _ = seat.set("focus", gnoblin_core::NodeId(*next));
        }
        Ok(())
    }

    // ---- policies -------------------------------------------------------------------------------------------------------

    /// Makes the desk of `lead` show one workspace over `members` (the desks of the other members show nothing of their own and
    /// cover the same workspace), or ends the span with `None`. Asking for the span that holds changes nothing.
    pub(crate) fn span_desks(&mut self, lead: NodeId, members: Option<&[NodeId]>) {
        let Some(desk) = self.desk_of_output(lead) else {
            return;
        };
        let covered: Vec<NodeId> = members
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|output| self.desk_of_output(*output).is_some())
            .collect();
        let next = (covered.len() > 1).then_some(covered);
        let Some(at) = self.desk_index(desk) else {
            return;
        };
        if self.desks[at].spans == next {
            return;
        }
        for other in &mut self.desks {
            if other.spanned_by == Some(desk) {
                other.spanned_by = None;
            }
        }
        self.desks[at].spans.clone_from(&next);
        for output in next.unwrap_or_default() {
            if let Some(under) = self.desk_of_output(output)
                && under != desk
                && let Some(index) = self.desk_index(under)
            {
                self.desks[index].spanned_by = Some(desk);
                self.set_shown(under, None);
            }
        }
        if let Some(workspace) = self.desks[at].shown.and_then(|id| self.ws.get_mut(&id)) {
            workspace.dirty = true;
        }
        self.fill_desks();
    }

    /// Asks the compositor to make `output` a copy of the output called `connector` (an empty name ends the copy).
    fn request_mirror(&self, output: NodeId, connector: &str) {
        let target = self.g.node(gnoblin_core::NodeId(output));
        let _ = target.call("apply", &[Value::Text(format!("mirror_of={connector}"))]);
    }

    /// The output the other outputs copy or the span starts from: the policy's lead if it still has a desk, else the primary one.
    fn policy_lead(&self) -> Option<NodeId> {
        if self.policy.lead != 0 && self.desk_of_output(self.policy.lead).is_some() {
            return Some(self.policy.lead);
        }
        self.primary_output()
    }

    /// Brings the outputs and the desks to what the output and workspace policies say.
    pub(crate) fn apply_policies(&mut self) {
        match self.policy.outputs {
            abi::STD_OUTPUTS_DUPLICATE => {
                self.span_desks_end();
                if let Some(lead) = self.policy_lead() {
                    let connector = nodes::text(&self.g, lead, "connector");
                    for output in self.outputs.clone() {
                        if output == lead {
                            continue;
                        }
                        let copies = nodes::text(&self.g, output, "mirror_of");
                        if copies != connector && !connector.is_empty() {
                            self.request_mirror(output, &connector);
                            if !self.policy.mirrored.contains(&output) {
                                self.policy.mirrored.push(output);
                            }
                        }
                    }
                }
            }
            abi::STD_OUTPUTS_EXTEND => {
                self.release_mirrors();
                if let Some(lead) = self.policy_lead() {
                    let members = self.desk_outputs();
                    // The lead's desk spans the rest: put it first in its own span.
                    let mut ordered = vec![lead];
                    ordered.extend(members.into_iter().filter(|output| *output != lead));
                    self.span_desks(lead, Some(&ordered));
                }
            }
            _ => {
                self.release_mirrors();
                self.span_desks_end();
            }
        }
        if self.policy.workspaces == abi::STD_WORKSPACES_PER_OUTPUT {
            self.ensure_policy_workspaces();
        }
        self.fill_desks();
    }

    /// Ends every span.
    fn span_desks_end(&mut self) {
        let leads: Vec<NodeId> = self
            .desks
            .iter()
            .filter(|desk| desk.spans.is_some())
            .filter_map(|desk| desk.output)
            .collect();
        for lead in leads {
            self.span_desks(lead, None);
        }
    }

    /// Asks the compositor to stop the mirrors this library asked for.
    fn release_mirrors(&mut self) {
        for output in std::mem::take(&mut self.policy.mirrored) {
            if self.outputs.contains(&output) && !nodes::text(&self.g, output, "mirror_of").is_empty() {
                self.request_mirror(output, "");
            }
        }
    }

    /// One workspace for every output that can show one: adds the ones that are missing.
    pub(crate) fn ensure_policy_workspaces(&mut self) {
        let wanted = self
            .desks
            .iter()
            .filter(|desk| desk.output.is_some() && desk.spanned_by.is_none())
            .count();
        let mut counter = 1;
        while self.workspaces.len() < wanted {
            let id = loop {
                let candidate = format!("{}{counter}", self.policy.prefix);
                counter += 1;
                if !self.ws.values().any(|ws| ws.attached && ws.id == candidate) {
                    break candidate;
                }
            };
            let layout = LayoutObj::floating(self.policy.placement, self.policy.step);
            let workspace = self.new_workspace(id, String::new(), self.policy.gaps, layout, true);
            let _ = self.add_workspace(workspace);
        }
    }

    /// An output went: removes the workspaces the policy added that no output needs any more, their windows moving to a workspace
    /// that stays.
    pub(crate) fn prune_policy_workspaces(&mut self) {
        if self.policy.workspaces != abi::STD_WORKSPACES_PER_OUTPUT {
            return;
        }
        let wanted = self
            .desks
            .iter()
            .filter(|desk| desk.output.is_some() && desk.spanned_by.is_none())
            .count()
            .max(1);
        while self.workspaces.len() > wanted {
            let spare = self
                .workspaces
                .iter()
                .rev()
                .copied()
                .find(|workspace| self.ws.get(workspace).is_some_and(|ws| ws.auto) && !self.is_shown(*workspace));
            let Some(spare) = spare else {
                break;
            };
            self.remove_workspace(spare);
        }
    }
}
