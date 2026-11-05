//! The layout host: the caller's side of the worker. It never blocks on a layout for
//! longer than the frame deadline, discards stale answers, and abandons a worker that
//! stops answering. Rust cannot kill a thread, so an abandoned worker is leaked and a
//! fresh one is built from the last good persisted state.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use gnoblin_lua_sandbox::Limits;
use gnoblin_wm::layout::{Layout, LayoutFailure, LayoutWindow, Params, fallback, settle};

use super::LayoutReason;
use super::data::Data;
use super::lua::{LayoutCall, LayoutFault};
use super::snapshot::{LayoutRequest, LayoutSnapshot};
use super::worker::{LayoutFinished, Spawn, Worker, WorkerJob};

/// Decides whether a layout's worker must stall, for tests of the wedge path. Returns
/// true to make the worker stop answering the named layout's next call.
pub type WedgeHook = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The host's settings.
#[derive(Clone)]
pub struct HostConfig {
    /// The budgets each entry into Lua runs under.
    pub limits: Limits,
    /// The only directories layout modules are loaded from.
    pub module_dirs: Vec<PathBuf>,
    /// The longest a caller waits for a layout's answer.
    pub frame_deadline: Duration,
    /// How long a worker may stay silent with work outstanding before it is abandoned.
    pub hard_limit: Duration,
    /// How many abandoned workers are tolerated before layouts are disabled.
    pub leak_cap: usize,
    /// Test seam: makes the worker stall on a named layout. `None` in production.
    pub wedge: Option<WedgeHook>,
}

impl HostConfig {
    /// The production settings for layouts loaded from `module_dirs`.
    pub fn new(module_dirs: Vec<PathBuf>) -> Self {
        Self {
            limits: Limits::default(),
            module_dirs,
            frame_deadline: Duration::from_millis(16),
            hard_limit: Duration::from_millis(100),
            leak_cap: 3,
            wedge: None,
        }
    }
}

/// A failed registration: why the layout cannot be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFailure {
    /// Why the layout cannot be loaded.
    pub reason: LayoutReason,
    /// The Lua message, or the host's description.
    pub message: String,
}

/// What a layout call produced for the caller.
#[derive(Clone, Debug)]
pub enum HostOutcome {
    /// The layout's placement. A declined request yields the built-in floating
    /// placement here, with `origin` set to the fallback.
    Placed(Layout),
    /// The layout failed. The caller applies `fallback`, the built-in floating
    /// placement, when present; it is absent only when the snapshot itself is invalid.
    Failed {
        /// The layout the call was for.
        layout: String,
        /// Why the layout produced no placement.
        reason: LayoutReason,
        /// The Lua message, or the host's description.
        message: String,
        /// The built-in floating placement for this snapshot.
        fallback: Option<Layout>,
    },
}

struct LayoutRegistered {
    module: String,
    options: Data,
}

/// The layout host. One per compositor; it owns the worker and the layout registry.
pub struct Host {
    config: HostConfig,
    worker: Option<Worker>,
    registered: BTreeMap<String, LayoutRegistered>,
    /// The last persisted `ctx.state` per layout, with the module hash it belongs to.
    states: BTreeMap<String, (u64, Data)>,
    generation: u64,
    /// Jobs sent whose answer has not been received.
    outstanding: usize,
    /// When the oldest outstanding job was sent.
    busy_since: Option<Instant>,
    leaked: usize,
    disabled: bool,
}

impl Host {
    /// Starts the host and its first worker.
    pub fn new(config: HostConfig) -> Self {
        let worker = spawn_worker(&config);
        Self {
            config,
            worker: Some(worker),
            registered: BTreeMap::new(),
            states: BTreeMap::new(),
            generation: 0,
            outstanding: 0,
            busy_since: None,
            leaked: 0,
            disabled: false,
        }
    }

    /// The number of workers abandoned so far.
    pub fn leaked(&self) -> usize {
        self.leaked
    }

    /// Whether layouts are disabled for the rest of the session.
    pub fn disabled(&self) -> bool {
        self.disabled
    }

    /// Loads `module` and registers it as layout `name` with the user's `options`. The
    /// module is loaded on the worker; the wait is one hard limit, not a frame.
    pub fn register(&mut self, name: &str, module: &str, options: Data) -> Result<(), HostFailure> {
        if self.disabled {
            return Err(disabled_failure());
        }
        self.reap();
        self.abandon_if_wedged();
        if self.disabled {
            return Err(disabled_failure());
        }
        self.ensure_worker();
        let previous = self.registered.insert(
            name.to_owned(),
            LayoutRegistered {
                module: module.to_owned(),
                options: options.clone(),
            },
        );
        let generation = self.next_generation();
        let job = WorkerJob::Load {
            generation,
            name: name.to_owned(),
            module: module.to_owned(),
            options,
        };
        let outcome = self.submit(job, generation, self.config.hard_limit);
        let failure = match outcome {
            Some(Ok(LayoutFinished::Loaded)) => return Ok(()),
            Some(Ok(LayoutFinished::Ran(_))) => HostFailure {
                reason: LayoutReason::InvalidLayout,
                message: "the worker answered a load with a layout call".into(),
            },
            Some(Err(fault)) => HostFailure {
                reason: fault.reason,
                message: fault.message,
            },
            None => HostFailure {
                reason: LayoutReason::Deadline,
                message: format!("loading `{module}` did not finish in time"),
            },
        };
        match previous {
            Some(old) => self.registered.insert(name.to_owned(), old),
            None => self.registered.remove(name),
        };
        Err(failure)
    }

    /// The arrange pass for layout `name` over `snapshot`.
    pub fn arrange(&mut self, name: &str, snapshot: &LayoutSnapshot) -> HostOutcome {
        self.dispatch(name, LayoutCall::Arrange, snapshot)
    }

    /// A message to layout `name`: a window was added or removed, a request arrived, or
    /// a command ran.
    pub fn message(&mut self, name: &str, request: LayoutRequest, snapshot: &LayoutSnapshot) -> HostOutcome {
        self.dispatch(name, LayoutCall::Message(request), snapshot)
    }

    fn dispatch(&mut self, name: &str, call: LayoutCall, snapshot: &LayoutSnapshot) -> HostOutcome {
        let params = match params_of(snapshot) {
            Ok(params) => params,
            Err(message) => {
                return HostOutcome::Failed {
                    layout: name.to_owned(),
                    reason: LayoutReason::Error,
                    message,
                    fallback: None,
                };
            }
        };
        let failed = |reason: LayoutReason, message: String| HostOutcome::Failed {
            layout: name.to_owned(),
            reason,
            message,
            fallback: Some(fallback(&params)),
        };
        if self.disabled {
            return failed(
                LayoutReason::VmWedged,
                "layouts are disabled after repeated wedged workers".into(),
            );
        }
        self.reap();
        self.abandon_if_wedged();
        if self.disabled {
            return failed(
                LayoutReason::VmWedged,
                "layouts are disabled after repeated wedged workers".into(),
            );
        }
        if !self.registered.contains_key(name) {
            return failed(
                LayoutReason::MissingModule,
                format!("layout `{name}` is not registered"),
            );
        }
        self.ensure_worker();
        let generation = self.next_generation();
        let state = self.states.get(name).cloned();
        let job = WorkerJob::Run {
            generation,
            name: name.to_owned(),
            call,
            snapshot: snapshot.clone(),
            state,
        };
        match self.submit(job, generation, self.config.frame_deadline) {
            None => failed(
                LayoutReason::Deadline,
                format!("layout `{name}` did not answer within the frame"),
            ),
            Some(Err(fault)) => failed(fault.reason, fault.message),
            Some(Ok(LayoutFinished::Loaded)) => failed(
                LayoutReason::Error,
                "the worker answered a layout call with a load".into(),
            ),
            Some(Ok(LayoutFinished::Ran(ran))) => match settle(&params, ran.reply) {
                Ok(layout) => {
                    self.states.insert(name.to_owned(), (ran.hash, ran.state));
                    HostOutcome::Placed(layout)
                }
                Err(failure) => failed(
                    match failure {
                        LayoutFailure::Error => LayoutReason::Error,
                        LayoutFailure::Timeout => LayoutReason::Deadline,
                        LayoutFailure::Disabled => LayoutReason::MissingModule,
                        LayoutFailure::VmWedged => LayoutReason::VmWedged,
                        LayoutFailure::InvalidReply(_) => LayoutReason::RefusedReply,
                    },
                    format!("layout `{name}` returned an invalid reply"),
                ),
            },
        }
    }

    /// Queues `job` and waits up to `timeout` for the answer with `generation`. Answers
    /// with other generations are stale and are discarded. `None` means no answer.
    fn submit(
        &mut self,
        job: WorkerJob,
        generation: u64,
        timeout: Duration,
    ) -> Option<Result<LayoutFinished, LayoutFault>> {
        let sent = match self.worker.as_ref() {
            Some(worker) => worker.send(job),
            None => false,
        };
        if !sent {
            self.lost_worker();
            return Some(Err(LayoutFault::new(
                LayoutReason::VmDown,
                "the layout worker is not running",
            )));
        }
        self.note_sent();
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let received = match self.worker.as_ref() {
                Some(worker) => worker.recv_timeout(remaining),
                None => {
                    return Some(Err(LayoutFault::new(
                        LayoutReason::VmDown,
                        "the layout worker is not running",
                    )));
                }
            };
            match received {
                Ok(done) => {
                    self.note_settled();
                    if done.generation == generation {
                        return Some(done.result);
                    }
                }
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.lost_worker();
                    return Some(Err(LayoutFault::new(LayoutReason::VmDown, "the layout worker exited")));
                }
            }
        }
    }

    /// Consumes every answer that has already arrived. All of them are stale: a caller
    /// only waits for the answer to its own job, inside `submit`.
    fn reap(&mut self) {
        loop {
            let received = match self.worker.as_ref() {
                Some(worker) => worker.try_recv(),
                None => return,
            };
            match received {
                Ok(_) => self.note_settled(),
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.lost_worker();
                    return;
                }
            }
        }
    }

    /// Abandons the worker if work has been outstanding for a whole hard limit. After
    /// `leak_cap` abandoned workers, layouts are disabled.
    fn abandon_if_wedged(&mut self) {
        let wedged = match self.busy_since {
            Some(since) => self.outstanding > 0 && since.elapsed() >= self.config.hard_limit,
            None => false,
        };
        if !wedged {
            return;
        }
        // Dropping the worker detaches its thread: the old thread is leaked, not joined.
        self.worker = None;
        self.leaked += 1;
        self.outstanding = 0;
        self.busy_since = None;
        if self.leaked >= self.config.leak_cap {
            self.disabled = true;
        }
    }

    /// Starts a worker if there is none, and replays every registration onto it.
    fn ensure_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        let worker = spawn_worker(&self.config);
        self.worker = Some(worker);
        self.outstanding = 0;
        self.busy_since = None;
        let replay: Vec<(String, String, Data)> = self
            .registered
            .iter()
            .map(|(name, layout)| (name.clone(), layout.module.clone(), layout.options.clone()))
            .collect();
        for (name, module, options) in replay {
            let job = WorkerJob::Load {
                generation: 0,
                name,
                module,
                options,
            };
            if let Some(worker) = self.worker.as_ref() {
                worker.send(job);
            }
            self.note_sent();
        }
    }

    /// The worker thread is gone. The next call starts a fresh one.
    fn lost_worker(&mut self) {
        self.worker = None;
        self.outstanding = 0;
        self.busy_since = None;
    }

    fn note_sent(&mut self) {
        if self.outstanding == 0 {
            self.busy_since = Some(Instant::now());
        }
        self.outstanding += 1;
    }

    fn note_settled(&mut self) {
        self.outstanding = self.outstanding.saturating_sub(1);
        if self.outstanding == 0 {
            self.busy_since = None;
        }
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}

fn spawn_worker(config: &HostConfig) -> Worker {
    Worker::spawn(Spawn {
        limits: config.limits,
        module_dirs: config.module_dirs.clone(),
        wedge: config.wedge.clone(),
    })
}

fn disabled_failure() -> HostFailure {
    HostFailure {
        reason: LayoutReason::VmWedged,
        message: "layouts are disabled after repeated wedged workers".into(),
    }
}

fn params_of(snapshot: &LayoutSnapshot) -> Result<Params, String> {
    let windows = snapshot
        .windows
        .iter()
        .map(|window| LayoutWindow {
            id: window.id,
            width: window.width,
            height: window.height,
        })
        .collect();
    Params::new(snapshot.area.width, snapshot.area.height, windows).map_err(|error| format!("{error:?}"))
}
