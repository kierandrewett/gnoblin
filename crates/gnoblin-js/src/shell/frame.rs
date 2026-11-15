//! One frame of the shell: the compositor's batch is applied to the mirror, the script's listeners are told
//! what changed and run under their budgets, the layouts and the toolkit run, and what the script wrote is
//! sent as one transaction.

use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::{MirrorChangeKind, NodeId, TreeIncoming};
use rquickjs::{Array, Function, Object, Value};

use super::log;
use super::txgate::route_answer;
use super::{JsFrame, JsShell, JsShellError, MAX_JOBS_PER_PUMP};
use crate::error::{JsError, JsErrorKind};
use crate::host_api::JsTrackEnd;
use crate::input::{JsInputMessage, messages_to_js};

fn location_text(at: &crate::error::JsLocation) -> String {
    format!("{}:{}:{}", at.file.display(), at.line, at.column)
}

impl JsShell {
    /// Runs one frame: applies what the compositor sent, tells the script's listeners, runs them, and
    /// sends what the script wrote as one transaction.
    pub fn frame(&mut self) -> Result<JsFrame, JsShellError> {
        let mut diagnostics = Vec::new();
        let mut touched: Vec<NodeId> = Vec::new();
        let mut created: Vec<NodeId> = Vec::new();
        let mut destroyed: Vec<NodeId> = Vec::new();

        let mut incoming = std::mem::take(&mut self.incoming);
        incoming.extend(self.link.poll().map_err(|e| JsShellError::Link(e.0))?);
        for message in incoming {
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
                            destroyed.push(id);
                        }
                    }
                    state.ui.forget(&destroyed);
                    state.xr.forget(&destroyed);
                    state
                        .mirror
                        .apply_changes(serial, &changes)
                        .map_err(|e| JsShellError::Mirror(format!("{e:?}")))?;
                    touched.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Set)
                            .map(|c| c.node),
                    );
                    created.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Created)
                            .map(|c| c.node),
                    );
                }
                TreeIncoming::Reply(reply) => self.handle_reply(&reply, &mut diagnostics),
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
                TreeIncoming::Attached(_) | TreeIncoming::Snapshot { .. } => {
                    return Err(JsShellError::Link("attach or snapshot after boot".to_owned()));
                }
            }
        }

        let settled = self.sent.is_empty() && self.retry.is_empty();
        {
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
            created.retain(|node| state.mirror.node(*node).is_some());
            touched.retain(|node| state.mirror.node(*node).is_some() && !created.contains(node));
        }

        let started = Instant::now();
        let outcome = self.dispatch(&created, &touched, &destroyed, started, &mut diagnostics);
        self.state.borrow_mut().graveyard.clear();
        outcome?;
        self.run_host_api(&mut diagnostics)?;
        self.commit_document_changes(&mut diagnostics)?;
        self.run_layouts(&mut diagnostics)?;
        self.run_ui(&mut diagnostics)?;
        self.run_dialogs(&mut diagnostics)?;
        self.run_compositor_links(&mut diagnostics)?;
        self.collect_reports(&mut diagnostics)?;
        self.generation.flush_console();

        self.send_transaction()?;
        Ok(JsFrame { diagnostics })
    }

    /// Hands the frame's changes to the module's `plan`, then runs the queued listener calls one by one,
    /// each under its own budget, until they are all run or the frame's budget is.
    fn dispatch(
        &mut self,
        created: &[NodeId],
        touched: &[NodeId],
        destroyed: &[NodeId],
        started: Instant,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        if let Some(nodes) = self.boot_ready.take() {
            let nodes = self.script_ids(&nodes);
            let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
                entry.get::<_, Function<'_>>("ready")?.call((nodes,))
            })?;
        }
        let (created, touched, destroyed) = (
            self.script_ids(created),
            self.script_ids(touched),
            self.script_ids(destroyed),
        );
        let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
            entry
                .get::<_, Function<'_>>("plan")?
                .call((created, touched, destroyed))
        })?;
        // The pointer's edges on windows go to the clients' listeners after the plan, so a client made in this frame
        // is one the listeners know, and before the input matcher, so a press that focuses a window is heard first.
        let _queued: usize = self.call_entry(self.frame_budget, |entry| {
            let events = self.state.borrow_mut().input.take_window_events(entry.ctx())?;
            entry.get::<_, Function<'_>>("pointer")?.call((events,))
        })?;
        // The input matcher runs after the plan, so a client that died in this frame has already cancelled
        // its drags when the frame's chord messages are read. It runs every frame, with or without messages:
        // a drag samples the pointer once per frame.
        let messages = std::mem::take(&mut self.state.borrow_mut().input.inbox);
        let pending: usize = self.call_entry(self.frame_budget, move |entry| {
            let list = messages_to_js(entry.ctx(), &messages)?;
            entry.get::<_, Function<'_>>("input")?.call((list,))
        })?;
        self.run_pending(pending, started, diagnostics)
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
