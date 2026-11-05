//! The worker thread that owns one layout VM. A Lua state is not `Send`, so the VM is
//! built on the thread and never leaves it; only jobs and results cross the channels.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gnoblin_lua_sandbox::Limits;

use super::data::Data;
use super::lua::{LayoutCall, LayoutFault, Ran, Vm};
use super::require::message;
use super::snapshot::LayoutSnapshot;
use super::{LayoutReason, WedgeHook};

/// Work the host sends to a worker. Generation 0 marks a load the host replays after a
/// respawn; its result is never reported to a caller.
pub(crate) enum Job {
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

impl Job {
    fn generation(&self) -> u64 {
        match self {
            Job::Load { generation, .. } | Job::Run { generation, .. } => *generation,
        }
    }
}

/// What a successful job produced.
pub(crate) enum LayoutFinished {
    /// A layout module was loaded and registered on the worker.
    Loaded,
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
    pub(crate) module_dirs: Vec<PathBuf>,
    pub(crate) wedge: Option<WedgeHook>,
}

/// A running worker: the sending end of its jobs and the receiving end of its answers.
pub(crate) struct Worker {
    jobs: Sender<Job>,
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
    pub(crate) fn send(&self, job: Job) -> bool {
        self.jobs.send(job).is_ok()
    }

    pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<Done, RecvTimeoutError> {
        self.done.recv_timeout(timeout)
    }

    pub(crate) fn try_recv(&self) -> Result<Done, TryRecvError> {
        self.done.try_recv()
    }
}

fn serve(jobs: Receiver<Job>, done: Sender<Done>, spawn: Spawn) {
    let mut vm = Vm::new(spawn.limits, spawn.module_dirs).map_err(message);
    for job in jobs {
        let generation = job.generation();
        let result = match (&mut vm, job) {
            (Err(text), _) => Err(LayoutFault::new(LayoutReason::VmDown, text.clone())),
            (
                Ok(vm),
                Job::Load {
                    name, module, options, ..
                },
            ) => vm.load(&name, &module, &options).map(|_| LayoutFinished::Loaded),
            (
                Ok(vm),
                Job::Run {
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
