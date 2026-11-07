//! The worker thread that owns one layout VM. A Lua state is not `Send`, so the VM is
//! built on the thread and never leaves it; only jobs and results cross the channels.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gnoblin_lua_sandbox::Limits;

use super::LayoutReason;
use super::data::Data;
use super::host::WedgeHook;
use super::lua::{LayoutCall, LayoutFault, LayoutManifest, Ran, Vm};
use super::require::{ModuleRoots, message};
use super::snapshot::LayoutSnapshot;

/// Work the host sends to a worker. Generation 0 marks a load the host replays after a
/// respawn; its result is never reported to a caller.
pub(crate) enum WorkerJob {
    Load {
        generation: u64,
        name: String,
        module: String,
        options: Data,
    },
    Run {
        generation: u64,
        name: String,
        call: LayoutCall,
        snapshot: LayoutSnapshot,
        state: Option<(u64, Data)>,
    },
}

impl WorkerJob {
    fn generation(&self) -> u64 {
        match self {
            WorkerJob::Load { generation, .. } | WorkerJob::Run { generation, .. } => *generation,
        }
    }
}

/// What a successful job produced.
pub(crate) enum LayoutFinished {
    /// A layout module was loaded and registered on the worker.
    Loaded(LayoutManifest),
    /// A layout call's reply, its module hash and its persisted state.
    Ran(Box<Ran>),
}

/// A worker's answer to one job.
pub(crate) struct Done {
    pub(crate) generation: u64,
    pub(crate) result: Result<LayoutFinished, LayoutFault>,
}

/// What a worker thread needs to build its VM.
pub(crate) struct Spawn {
    pub(crate) limits: Limits,
    pub(crate) roots: ModuleRoots,
    pub(crate) wedge: Option<WedgeHook>,
}

/// A running worker: the sending end of its jobs and the receiving end of its answers.
pub(crate) struct Worker {
    jobs: Sender<WorkerJob>,
    pub(crate) done: Receiver<Done>,
    /// Dropped without joining: a wedged worker is abandoned, never waited for.
    _thread: JoinHandle<()>,
}

impl Worker {
    pub(crate) fn spawn(spawn: Spawn) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("gnoblin-layout".into())
            .spawn(move || serve(job_rx, done_tx, spawn))
            .expect("spawning the layout worker thread");
        Self {
            jobs,
            done,
            _thread: thread,
        }
    }

    /// Queues `job`. Returns false when the worker thread has exited.
    pub(crate) fn send(&self, job: WorkerJob) -> bool {
        self.jobs.send(job).is_ok()
    }

    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<Done, RecvTimeoutError> {
        self.done.recv_timeout(timeout)
    }
}

fn serve(jobs: Receiver<WorkerJob>, done: Sender<Done>, spawn: Spawn) {
    let mut vm = Vm::new(spawn.limits, spawn.roots).map_err(message);
    for job in jobs {
        let generation = job.generation();
        let result = match (&mut vm, job) {
            (Err(text), _) => Err(LayoutFault::new(LayoutReason::VmDown, text.clone())),
            (
                Ok(vm),
                WorkerJob::Load {
                    name, module, options, ..
                },
            ) => vm.load(&name, &module, &options).map(LayoutFinished::Loaded),
            (
                Ok(vm),
                WorkerJob::Run {
                    name,
                    call,
                    snapshot,
                    state,
                    ..
                },
            ) => {
                if spawn.wedge.as_ref().is_some_and(|wedge| wedge(&name)) {
                    // A stuck layout that never answers. The host abandons this thread.
                    loop {
                        thread::park();
                    }
                }
                vm.run(&name, &call, &snapshot, state)
                    .map(|ran| LayoutFinished::Ran(Box::new(ran)))
            }
        };
        if done.send(Done { generation, result }).is_err() {
            return;
        }
    }
}
