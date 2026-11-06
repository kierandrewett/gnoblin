//! A registered Lua layout as a `gnoblin_wm` [`LayoutEngine`].
//!
//! The engine never runs Lua on the caller's thread. [`LayoutEngine::submit`] starts a
//! run on the host's worker and returns at once. [`LayoutEngine::collect`] polls that
//! run until the caller's deadline and reports `Pending` past it. Only the newest run
//! is kept: submitting a run forgets every older one, so a late answer never reaches
//! the caller.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use gnoblin_types::{RectF, WindowId, WorkspaceId};
use gnoblin_wm::layout::{
    Generation, Job, LayoutEngine, LayoutExtent, LayoutFailure, LayoutKind, LayoutOutcome, Params, Reply, Ticket,
};

use super::LayoutReason;
use super::host::Host;
use super::lua::{LayoutCall, LayoutFault};
use super::snapshot::{Gaps, LayoutSnapshot, SizeHints, SnapshotWindow};
use super::worker::LayoutFinished;

/// One named Lua layout, driven through the wm's layout contract.
pub struct LuaEngine {
    host: Rc<RefCell<Host>>,
    layout: String,
    /// The workspace, focus and gaps every run is made with. The output, area and
    /// windows come from each run's [`Params`].
    workspace: WorkspaceId,
    focus: Option<WindowId>,
    gaps: Gaps,
    /// Each submitted run: its host generation, or the fault that stopped it at submit.
    runs: BTreeMap<Generation, Result<u64, LayoutFault>>,
}

impl LuaEngine {
    /// Drives layout `layout` on `host` for `workspace`, with the gaps `gaps`. No window
    /// is focused until [`LuaEngine::with_focus`] names one.
    pub fn new(host: Rc<RefCell<Host>>, layout: impl Into<String>, workspace: WorkspaceId, gaps: Gaps) -> Self {
        Self {
            host,
            layout: layout.into(),
            workspace,
            focus: None,
            gaps,
            runs: BTreeMap::new(),
        }
    }

    /// Sets the focused window every run is made with. A focus that names no window in a
    /// run's [`Params`] is dropped from that run.
    #[must_use]
    pub fn with_focus(mut self, focus: Option<WindowId>) -> Self {
        self.focus = focus;
        self
    }

    fn snapshot(&self, params: &Params) -> LayoutSnapshot {
        let windows: Vec<SnapshotWindow> = params
            .windows()
            .iter()
            .map(|window| SnapshotWindow {
                id: window.id,
                app_id: String::new(),
                title: String::new(),
                width: window.width,
                height: window.height,
                size_hints: SizeHints::default(),
            })
            .collect();
        let focus = self
            .focus
            .filter(|focus| windows.iter().any(|window| window.id == *focus));
        LayoutSnapshot {
            workspace: self.workspace,
            output: params.output().clone(),
            area: RectF {
                x: 0.0,
                y: 0.0,
                width: params.output_width(),
                height: params.output_height(),
            },
            windows,
            focus,
            gaps: self.gaps,
        }
    }

    /// Forgets every run except `generation`'s, releasing their host state.
    fn forget_older(&mut self, generation: Generation) {
        let stale: Vec<Generation> = self.runs.keys().copied().filter(|g| *g != generation).collect();
        for old in stale {
            if let Some(Ok(run)) = self.runs.remove(&old) {
                self.host.borrow_mut().forget(run);
            }
        }
    }
}

impl LayoutEngine for LuaEngine {
    fn kind(&self) -> LayoutKind {
        // A layout that is not registered refuses every run, so it places nothing. Floating
        // leaves its windows to their own geometry, which is what that means.
        self.host
            .borrow()
            .manifest(&self.layout)
            .map_or(LayoutKind::Floating, |manifest| manifest.kind)
    }

    fn extent(&self) -> LayoutExtent {
        self.host
            .borrow()
            .manifest(&self.layout)
            .map_or(LayoutExtent::Output, |manifest| manifest.extent)
    }

    fn declares(&self, command: &str) -> bool {
        self.host
            .borrow()
            .manifest(&self.layout)
            .is_some_and(|manifest| manifest.commands.contains(command))
    }

    fn submit(&mut self, job: Job, generation: Generation) -> Ticket {
        self.forget_older(generation);
        let call = match &job {
            Job::Arrange(_) => LayoutCall::Arrange,
            Job::Request(_) => LayoutCall::Request,
            Job::Command(_, command) => LayoutCall::Command {
                name: command.name.clone(),
                args: command.args.clone(),
            },
        };
        let snapshot = self.snapshot(job.params());
        let started = self.host.borrow_mut().start(&self.layout, call, &snapshot);
        self.runs.insert(generation, started);
        Ticket::new(generation)
    }

    fn collect(&mut self, generation: Generation, deadline: Instant) -> LayoutOutcome {
        let run = match self.runs.get(&generation) {
            None => return ready(generation, Err(LayoutFailure::Error)),
            Some(Err(fault)) => {
                let failure = failure(fault);
                self.runs.remove(&generation);
                return ready(generation, Err(failure));
            }
            Some(Ok(run)) => *run,
        };
        let finished = self.host.borrow_mut().poll(run, deadline);
        let Some(finished) = finished else {
            return LayoutOutcome::Pending;
        };
        self.runs.remove(&generation);
        match finished {
            Ok(LayoutFinished::Ran(ran)) => ready(generation, Ok(ran.reply)),
            Ok(LayoutFinished::Loaded(_)) => ready(generation, Err(LayoutFailure::Error)),
            Err(fault) => ready(generation, Err(failure(&fault))),
        }
    }
}

fn ready(generation: Generation, result: Result<Reply, LayoutFailure>) -> LayoutOutcome {
    LayoutOutcome::Ready { generation, result }
}

/// The wm's failure for a host fault.
fn failure(fault: &LayoutFault) -> LayoutFailure {
    match fault.reason {
        LayoutReason::Deadline | LayoutReason::Stalled => LayoutFailure::Timeout,
        LayoutReason::VmWedged => LayoutFailure::VmWedged,
        LayoutReason::MissingModule => LayoutFailure::Disabled,
        LayoutReason::Memory
        | LayoutReason::Error
        | LayoutReason::RefusedReply
        | LayoutReason::VmDown
        | LayoutReason::InvalidLayout => LayoutFailure::Error,
    }
}
