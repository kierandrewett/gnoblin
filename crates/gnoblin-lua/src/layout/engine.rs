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

use gnoblin_types::{RectF, WorkspaceId};
use gnoblin_wm::layout::{Generation, Job, LayoutEngine, LayoutFailure, LayoutOutcome, Params, Reply, Ticket};

use super::LayoutReason;
use super::host::Host;
use super::lua::{LayoutCall, LayoutFault};
use super::snapshot::{Gaps, LayoutSnapshot, SizeHints, SnapshotWindow};
use super::worker::LayoutFinished;

/// One named Lua layout, driven through the wm's layout contract.
pub struct LuaEngine {
    host: Rc<RefCell<Host>>,
    layout: String,
    /// The workspace and gaps every run is made with. The output, area and windows
    /// come from each run's [`Params`].
    workspace: WorkspaceId,
    gaps: Gaps,
    /// Each submitted run: its host generation, or the fault that stopped it at submit.
    runs: BTreeMap<Generation, Result<u64, LayoutFault>>,
}

impl LuaEngine {
    /// Drives layout `layout` on `host` for `workspace`, with the gaps `gaps`.
    pub fn new(host: Rc<RefCell<Host>>, layout: impl Into<String>, workspace: WorkspaceId, gaps: Gaps) -> Self {
        Self {
            host,
            layout: layout.into(),
            workspace,
            gaps,
            runs: BTreeMap::new(),
        }
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
            focus: None,
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
    fn submit(&mut self, job: Job, generation: Generation) -> Ticket {
        self.forget_older(generation);
        let call = match &job {
            Job::Arrange(_) => LayoutCall::Arrange,
            Job::Request(_) => LayoutCall::Request,
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
            Ok(LayoutFinished::Loaded) => ready(generation, Err(LayoutFailure::Error)),
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
