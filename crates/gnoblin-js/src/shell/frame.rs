//! One frame of the shell: the compositor's batch is applied to the mirror, the script's listeners are told
//! what changed and run under their budgets, the layouts and the toolkit run, and what the script wrote is
//! sent as one transaction.
//!
//! The frame is [`STAGES`], run in that order, and nothing else decides the order. Several stages are in the order they
//! are in because of what an earlier one leaves for a later one, and the table says so beside each; `tests/frame_order.rs`
//! pins the order by recording the calls a frame makes into the entry.

use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::{MirrorChangeKind, NodeId, TreeIncoming};
use rquickjs::{Array, Function, Object, Value};

use super::gate::Backlog;
use super::log;
use super::txgate::route_answer;
use super::{JsFrame, JsShell, JsShellError, MAX_JOBS_PER_PUMP};
use crate::error::{JsError, JsErrorKind};
use crate::host_api::JsTrackEnd;
use crate::input::{JsInputMessage, messages_to_js};

fn location_text(at: &crate::error::JsLocation) -> String {
    format!("{}:{}:{}", at.file.display(), at.line, at.column)
}

/// What the stages of one frame hand each other.
struct FrameWork {
    /// What the compositor sent, for the first stage to apply.
    incoming: Vec<TreeIncoming>,
    /// What the links beside the tree had, each part taken by the stage that serves its link.
    backlog: Backlog,
    /// The nodes the frame's batches made, changed and destroyed, as the first two stages leave them for `plan`.
    created: Vec<NodeId>,
    touched: Vec<NodeId>,
    destroyed: Vec<NodeId>,
    /// When the frame's work in the script began: its budget counts from here.
    started: Instant,
    /// The listener calls the entry queued and the `listeners` stage has still to run.
    pending: usize,
    diagnostics: Vec<String>,
}

/// One stage of a frame: its name, which a log line uses, and what it does.
struct FrameStage {
    name: &'static str,
    run: fn(&mut JsShell, &mut FrameWork) -> Result<(), JsShellError>,
}

const fn stage(name: &'static str, run: fn(&mut JsShell, &mut FrameWork) -> Result<(), JsShellError>) -> FrameStage {
    FrameStage { name, run }
}

/// The stages of a frame, in order, the order of the table being the order of the frame. The comment on each says what
/// it needs from the ones before it and what the ones after it need from it.
const STAGES: &[FrameStage] = &[
    // Destroyed nodes go to the graveyard before their batch is applied, so `plan` can read their last values; the
    // answers, chords and events are sorted into the inboxes the stages after it read.
    stage("drain", JsShell::stage_drain),
    // The builder's copies of nodes must show what the compositor wrote natively before any script reads them: after the
    // batches, before every call into the script.
    stage("settle", JsShell::stage_settle),
    // The windows of the boot snapshot are announced once, before the plan, so a window that was there at boot is ready
    // before the frame's own changes reach it.
    stage("ready", JsShell::stage_ready),
    // Departures before arrivals: clients are released, adopted and told of their changes before the listeners that
    // follow can see them.
    stage("plan", JsShell::stage_plan),
    // After the plan, so a client made in this frame has its listeners; before the input matcher, so a press that focuses
    // a window is heard first.
    stage("pointer", JsShell::stage_pointer),
    // After the plan, so a client that died in this frame has already cancelled its drags; every frame, because a drag
    // samples the pointer once per frame.
    stage("input", JsShell::stage_input),
    // The listener calls the stages before it queued run in the order they were queued, each under its own budget, and
    // what they wrote is laid out in this very frame by the stages after.
    stage("listeners", JsShell::stage_listeners),
    // Nothing after the listeners may read a destroyed node's last values: from here a destroyed node reads its frozen
    // copy only.
    stage("graveyard", JsShell::stage_graveyard),
    // Process ends, operation results, signals, timers and the ends of animations reach the script after the listeners
    // and before the layout, so a timer's writes are laid out in the frame they fire.
    stage("api", JsShell::stage_api),
    // The configuration document is committed once the script has finished writing device settings, if its revision
    // moved.
    stage("document", JsShell::stage_document),
    // Bindings are flushed before the layouts read them, and every workspace is laid out once, after every listener has
    // run, so it sees the frame's final state; its last focus feeds the next frame's `release`.
    stage("layout", JsShell::stage_layout),
    // After the layout, so a titlebar reads the size the layout gave its window.
    stage("ui", JsShell::stage_ui),
    // The compositor's dialogs are offered to the listeners, and the verdicts go back, after the frame's own work.
    stage("dialogs", JsShell::stage_dialogs),
    // The idle connection is opened, served and closed after the listeners that may have asked for holds.
    stage("links", JsShell::stage_links),
    // The failures the stages before it recorded are reported once each, and the script's console is flushed, after
    // everything that can fail has run.
    stage("reports", JsShell::stage_reports),
    // After every stage that runs script: what the script dropped is let go of now, and what this stage does is no script.
    stage("natives", JsShell::stage_natives),
    // Everything the frame wrote goes out as one transaction, last, once the gate allows one in flight.
    stage("send", JsShell::stage_send),
];

/// The stages of a frame of a generation on the native library. The core's dispatch announces what happened and calls the script
/// back (the runtime's turn runs it beside the frame), so what is left to the engine is the tree link, the promise jobs and the
/// console.
const NATIVE_STAGES: &[FrameStage] = &[
    stage("drain", JsShell::stage_drain),
    stage("settle", JsShell::stage_settle),
    // Nothing announces `ready` to the script: the core does, for the windows it holds.
    stage("boot", JsShell::stage_boot),
    stage("graveyard", JsShell::stage_graveyard),
    stage("jobs", JsShell::stage_jobs),
    stage("document", JsShell::stage_document),
    stage("console", JsShell::stage_console),
    // After every stage that runs script: the objects the script dropped give their references back to the libraries.
    stage("natives", JsShell::stage_natives),
    stage("send", JsShell::stage_send),
];

impl JsShell {
    /// Runs one frame: applies what the compositor sent, tells the script's listeners, runs them, and
    /// sends what the script wrote as one transaction.
    pub fn frame(&mut self) -> Result<JsFrame, JsShellError> {
        let mut incoming = std::mem::take(&mut self.incoming);
        incoming.extend(self.link.poll().map_err(|e| JsShellError::Link(e.0))?);
        if !self.legacy() {
            // On the native library the core hears the session for the script. The engine's own session connection is read here
            // only for the request to reload the configuration (`gnoblinctl config reload`, `runtime.reloadConfig()`), which the
            // host answers by reloading; nothing on this library listens to its other events, so they are let go.
            drop(self.state.borrow_mut().api.poll());
        }
        // A turn on which nothing arrived and nothing is due does no work: see `gate`.
        if incoming.is_empty() && !self.frame_due() {
            self.note_quiet_turn();
            return Ok(JsFrame::default());
        }
        let mut backlog = self.take_backlog();
        let diagnostics = std::mem::take(&mut backlog.diagnostics);
        let worked = self.take_script_ran()
            || !incoming.is_empty()
            || !backlog.api.is_empty()
            || !backlog.dialogs.is_empty()
            || backlog.idle.reading.is_some()
            || backlog.idle.lost;
        let (dispatches_before, sent_before) = (self.dispatches, self.gate.sent);
        let mut work = FrameWork {
            incoming,
            backlog,
            created: Vec::new(),
            touched: Vec::new(),
            destroyed: Vec::new(),
            started: Instant::now(),
            pending: 0,
            diagnostics,
        };
        let steps = if self.legacy() { STAGES } else { NATIVE_STAGES };
        for step in steps {
            log::trace!(area = "shell", "frame stage {}", step.name);
            if let Err(error) = (step.run)(self, &mut work) {
                // A stage that fails ends the shell. The graveyard is emptied all the same: a destroyed node's last
                // values do not outlive the frame.
                self.state.borrow_mut().graveyard.clear();
                return Err(error);
            }
        }
        // What this frame did decides whether the next one looks again: a frame that ran a listener, sent a
        // transaction or had something to handle may have left something for the pass that comes after it.
        let worked = worked || self.dispatches != dispatches_before || self.gate.sent != sent_before;
        self.plan_next_frame(worked)?;
        Ok(JsFrame {
            diagnostics: work.diagnostics,
        })
    }

    /// Applies the compositor's batches to the mirror and sorts the rest of what arrived to the inboxes of the stages that
    /// serve it.
    fn stage_drain(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        let legacy = self.legacy();
        for message in std::mem::take(&mut work.incoming) {
            // A generation on the native library has no use for what the engine's own object model sorts: the core serves the
            // chords, the events and the answers to the libraries, so the engine keeps the tree and its own transactions only.
            if !legacy && !matches!(&message, TreeIncoming::Changes { .. } | TreeIncoming::Reply(_)) {
                continue;
            }
            match message {
                TreeIncoming::Changes { serial, changes } => {
                    let mut state = self.state.borrow_mut();
                    // A destroyed node's last values stay readable for the frame's plan, deepest node
                    // first as a node's own destruction follows its subtree's.
                    for change in changes.iter().filter(|c| c.kind == MirrorChangeKind::Destroyed) {
                        let subtree = change.removed.iter().copied().filter(|id| *id != change.node);
                        for id in subtree.rev().chain(std::iter::once(change.node)) {
                            if let Some(record) = state.mirror.node(id).cloned() {
                                state.graveyard.insert(id, record);
                            }
                            work.destroyed.push(id);
                        }
                    }
                    state.ui.forget(&work.destroyed);
                    state.xr.forget(&work.destroyed);
                    state
                        .mirror
                        .apply_changes(serial, &changes)
                        .map_err(|e| JsShellError::Mirror(format!("{e:?}")))?;
                    work.touched.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Set)
                            .map(|c| c.node),
                    );
                    work.created.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Created)
                            .map(|c| c.node),
                    );
                }
                TreeIncoming::Reply(reply) => self.handle_reply(&reply, &mut work.diagnostics),
                // A claimed chord, gesture or pointer drag is parked for the input module, which the frame hands it
                // to once the listeners are planned. A pointer or key event addressed to a canvas of the UI toolkit and
                // the answer to one of its measurements are parked for the toolkit's frame, the answer to a password
                // check for the lock screen's, and a pointer edge on a window for its client's listeners. Every other
                // event and answer has no JavaScript API: it is consumed so it does not pile up.
                TreeIncoming::Chord(chord) => self.state.borrow_mut().input.inbox.push(JsInputMessage::Chord(chord)),
                TreeIncoming::Gesture(gesture) => self
                    .state
                    .borrow_mut()
                    .input
                    .inbox
                    .push(JsInputMessage::Gesture(gesture)),
                TreeIncoming::Event(event) => {
                    let state = &mut *self.state.borrow_mut();
                    if !state.ui.accept_event(&event, &state.handles) {
                        state.input.accept_window_event(&state.mirror, &state.handles, &event);
                    }
                }
                TreeIncoming::Answer { token, result, error } => {
                    route_answer(&mut self.state.borrow_mut(), token, &result, &error);
                }
                TreeIncoming::TrackEnded {
                    node,
                    property,
                    completed,
                    tween,
                } => {
                    // The compositor's clock says an animation is over: the transition module settles on it.
                    let state = &mut *self.state.borrow_mut();
                    let end = JsTrackEnd {
                        node: state.handles.to_script(node),
                        property,
                        completed,
                        tween: tween.map_or(0.0, |tween| state.handles.to_script(tween)),
                    };
                    state.api.track_ends.push(end);
                }
                TreeIncoming::PointerChord(drag) => {
                    self.state.borrow_mut().input.inbox.push(JsInputMessage::Pointer(drag));
                }
                TreeIncoming::PointerButton(button) => {
                    self.state.borrow_mut().input.inbox.push(JsInputMessage::Button(button));
                }
                TreeIncoming::Attached(_) | TreeIncoming::Snapshot { .. } => {
                    return Err(JsShellError::Link("attach or snapshot after boot".to_owned()));
                }
            }
        }
        Ok(())
    }

    /// Lets the builder's copies show what the compositor wrote natively, and drops from the frame's lists the nodes that
    /// are gone or were made in it.
    fn stage_settle(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        let settled = self.tx.is_settled();
        let state = &mut *self.state.borrow_mut();
        if settled {
            state.tick.settle();
        }
        // A node the builder still holds a copy of (pending writes that a shut gate kept from the last frame, or
        // a transaction in flight) must show what the compositor has written natively since the copy was made: a
        // read of its `decoration` is the compositor's word, not the copy's.
        state.tick.refresh(&state.mirror);
        // A node a later batch of this frame destroyed has nothing to wrap or to touch, and a node made
        // in this frame is born with its final values, so only older nodes raise change events.
        work.created.retain(|node| state.mirror.node(*node).is_some());
        work.touched
            .retain(|node| state.mirror.node(*node).is_some() && !work.created.contains(node));
        // The frame's budget counts from here: the script has not run yet.
        work.started = Instant::now();
        Ok(())
    }

    /// Announces `ready` for the snapshot's nodes, once after boot or a swap.
    fn stage_ready(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        if let Some(nodes) = self.boot_ready.take() {
            let nodes = self.script_ids(&nodes);
            let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
                entry.get::<_, Function<'_>>("ready")?.call((nodes,))
            })?;
        }
        Ok(())
    }

    /// Hands the frame's changes to the module's `plan`.
    fn stage_plan(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        let (created, touched, destroyed) = (
            self.script_ids(&work.created),
            self.script_ids(&work.touched),
            self.script_ids(&work.destroyed),
        );
        let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
            entry
                .get::<_, Function<'_>>("plan")?
                .call((created, touched, destroyed))
        })?;
        Ok(())
    }

    /// Hands the pointer's edges on windows to the clients' listeners.
    fn stage_pointer(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        let _queued: usize = self.call_entry(self.frame_budget, |entry| {
            let events = self.state.borrow_mut().input.take_window_events(entry.ctx())?;
            entry.get::<_, Function<'_>>("pointer")?.call((events,))
        })?;
        Ok(())
    }

    /// Hands the frame's chord, gesture and pointer messages to the input matcher, which queues the calls they cause.
    fn stage_input(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        let messages = std::mem::take(&mut self.state.borrow_mut().input.inbox);
        work.pending = self.call_entry(self.frame_budget, move |entry| {
            let list = messages_to_js(entry.ctx(), &messages)?;
            entry.get::<_, Function<'_>>("input")?.call((list,))
        })?;
        Ok(())
    }

    /// Runs the queued listener calls one by one, each under its own budget, until they are all run or the frame's.
    fn stage_listeners(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_pending(work.pending, work.started, &mut work.diagnostics)
    }

    /// Forgets the destroyed nodes' last values.
    fn stage_graveyard(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        self.state.borrow_mut().graveyard.clear();
        Ok(())
    }

    /// Takes the boot snapshot's mark: on the native library the core announces the windows it holds, and the engine has
    /// nothing to announce.
    fn stage_boot(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        self.boot_ready = None;
        Ok(())
    }

    /// Runs the promise jobs a callback of the core left, which no message announces.
    fn stage_jobs(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.pump_jobs(&mut work.diagnostics)
    }

    /// Flushes the script's console.
    fn stage_console(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        self.generation.flush_console();
        Ok(())
    }

    /// Lets the libraries have back the references of the objects the script dropped (`ScriptIds::on_safe_point`). It runs no script,
    /// so it is not work: it marks nothing and wakes no frame.
    fn stage_natives(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        self.generation.run_safe_points();
        Ok(())
    }

    /// The host API's frame: the batch of what happened outside the script goes to the module.
    fn stage_api(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_host_api(std::mem::take(&mut work.backlog.api), &mut work.diagnostics)
    }

    /// Commits the configuration document if its revision moved.
    fn stage_document(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.commit_document_changes(&mut work.diagnostics)
    }

    /// Lays out the workspaces that changed.
    fn stage_layout(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_layouts(&mut work.diagnostics)
    }

    /// Runs the UI toolkit's frame.
    fn stage_ui(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_ui(&mut work.diagnostics)
    }

    /// Serves the compositor's dialogs.
    fn stage_dialogs(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_dialogs(std::mem::take(&mut work.backlog.dialogs), &mut work.diagnostics)
    }

    /// Serves the idle connection.
    fn stage_links(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.run_compositor_links(std::mem::take(&mut work.backlog.idle), &mut work.diagnostics)
    }

    /// Reports the failures the frame recorded and flushes the console.
    fn stage_reports(&mut self, work: &mut FrameWork) -> Result<(), JsShellError> {
        self.collect_reports(&mut work.diagnostics)?;
        self.generation.flush_console();
        Ok(())
    }

    /// Sends what the frame wrote as one transaction.
    fn stage_send(&mut self, _work: &mut FrameWork) -> Result<(), JsShellError> {
        self.send_transaction()
    }

    /// Runs the queued calls one by one, each under its own budget, until they are all run or the frame's
    /// budget is used up. Calls that do not fit wait for the next frame.
    pub(super) fn run_pending(
        &mut self,
        mut pending: usize,
        started: Instant,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        while pending > 0 {
            if started.elapsed() >= self.frame_budget {
                diagnostics.push(format!(
                    "the frame's {} ms budget is used up: {pending} listener calls wait for the next frame",
                    self.frame_budget.as_millis()
                ));
                break;
            }
            self.dispatches = self.dispatches.saturating_add(1);
            match self.call_entry(self.entry_budget, |entry| {
                entry.get::<_, Function<'_>>("next")?.call(())
            }) {
                Ok(left) => pending = left,
                Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                    // The call that ran out was taken off the queue before it ran, so the next one is
                    // next. Its failure is a diagnostic here, and the listeners after it still run.
                    self.count_strike(&error);
                    diagnostics.push(format!("a listener call was stopped: {error}"));
                    pending = self.call_entry(self.entry_budget, |entry| {
                        entry.get::<_, Function<'_>>("pending")?.call(())
                    })?;
                }
                Err(other) => return Err(other),
            }
        }
        self.pump_jobs(diagnostics)
    }

    /// Runs the promise jobs the queued calls left: the reaction of a `.then` on a promise a call settled, such as
    /// `Transition.finished`. A script that chains jobs without end is cut at [`MAX_JOBS_PER_PUMP`] and the rest wait for
    /// the next frame, and one that runs out its budget is stopped and counted like any call.
    fn pump_jobs(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        match self
            .generation
            .call_budgeted(self.entry_budget, |ctx| {
                for _ in 0..MAX_JOBS_PER_PUMP {
                    if !ctx.execute_pending_job() {
                        break;
                    }
                }
                Ok(())
            })
            .map_err(JsShellError::from)
        {
            Ok(()) => Ok(()),
            Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                self.count_strike(&error);
                diagnostics.push(format!("a promise job was stopped: {error}"));
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Lays out the workspaces that changed this frame (membership, shown workspace, area or a layout's own
    /// fields), once each, after every listener has run so the layouts see the frame's final state. A layout
    /// that throws is a diagnostic from the module; one that runs out its budget stops the pass, counts a
    /// strike, and leaves the workspaces after it for the next frame.
    fn run_layouts(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        match self.call_entry(self.frame_budget, |entry| {
            entry.get::<_, Function<'_>>("layout")?.call::<_, usize>(())
        }) {
            Ok(_laid_out) => Ok(()),
            Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                self.count_strike(&error);
                diagnostics.push(format!("the layout pass was stopped: {error}"));
                self.gate.deferred = true;
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Runs the UI toolkit's frame: measurement answers and pointer events in, every canvas arranged and painted.
    /// It runs after the layouts so a titlebar reads the size the layout gave its window. A canvas that throws is a
    /// diagnostic from the module; running out the budget stops the pass, counts a strike and leaves the rest for the
    /// next frame.
    fn run_ui(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        match self.call_entry(self.frame_budget, |entry| {
            entry.get::<_, Function<'_>>("ui")?.call::<_, ()>(())
        }) {
            Ok(()) => Ok(()),
            Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                self.count_strike(&error);
                diagnostics.push(format!("the UI pass was stopped: {error}"));
                self.gate.deferred = true;
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// The numbers a script holds for a list of node ids (see `NodeHandles`).
    fn script_ids(&self, nodes: &[NodeId]) -> Vec<f64> {
        let state = self.state.borrow();
        nodes.iter().map(|id| state.handles.to_script(*id)).collect()
    }

    /// Runs `f` on the module's entry object, under `budget`.
    pub(super) fn call_entry<R>(
        &self,
        budget: Duration,
        f: impl for<'js> FnOnce(Object<'js>) -> rquickjs::Result<R>,
    ) -> Result<R, JsShellError> {
        self.generation
            .call_budgeted(budget, |ctx| {
                let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
                f(host.get("entry")?)
            })
            .map_err(JsShellError::from)
    }

    /// Takes the failures the listeners recorded and reports each site once.
    pub(super) fn collect_reports(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        if !self.legacy() {
            // A callback of the core that throws is the core's to report: the failure comes back to it from the call.
            return Ok(());
        }
        let generation = &self.generation;
        let failures: Vec<(String, JsError)> = generation.call_budgeted(self.entry_budget, |ctx| {
            let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
            let entry: Object<'_> = host.get("entry")?;
            let list: Array<'_> = entry.get::<_, Function<'_>>("takeErrors")?.call(())?;
            let mut failures = Vec::new();
            for item in list.iter::<Object<'_>>() {
                let item = item?;
                let site: String = item.get("site")?;
                let thrown: Value<'_> = item.get("error")?;
                failures.push((site, generation.describe(ctx, &thrown)));
            }
            Ok(failures)
        })?;
        for (site, error) in failures {
            // One site is one listener method failing at one place; a loop that fails every frame is
            // logged once, not once per frame.
            let key = format!(
                "{site}@{}",
                error.location.as_ref().map_or_else(String::new, location_text)
            );
            let seen = self.reported.entry(key).or_insert(0);
            *seen += 1;
            if *seen == 1 {
                let text = format!("{site} threw: {error}");
                log::warn!(area = "listener", "{text}");
                diagnostics.push(text);
            }
        }
        // What the module could not do and says once per generation (a style property no renderer draws).
        let notices: Vec<String> = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("takeNotices")?.call(())
        })?;
        for notice in notices {
            log::info!(area = "shell", "{notice}");
            diagnostics.push(notice);
        }
        Ok(())
    }

    pub(super) fn count_strike(&mut self, error: &JsError) {
        if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) {
            self.strikes = self.strikes.saturating_add(1);
        }
    }
}
