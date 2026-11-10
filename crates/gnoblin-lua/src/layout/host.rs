//! The layout host: the caller's side of the worker. A run is started without waiting
//! and polled against a caller's deadline. Every answer is filed under its generation,
//! and an answer nobody is waiting for is dropped when its run is forgotten. A worker
//! that stays silent for a whole hard limit is abandoned. Rust cannot kill a thread, so
//! an abandoned worker is leaked and a fresh one is built from the registrations.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use gnoblin_lua_sandbox::Limits;

use super::LayoutReason;
use super::data::Data;
use super::lua::{LayoutCall, LayoutFault, LayoutManifest};
use super::require::{ModuleRoots, Root, fnv1a, read_source, resolve};
use super::snapshot::LayoutSnapshot;
use super::worker::{Done, LayoutFinished, Spawn, Worker, WorkerJob};

/// Decides whether a layout's worker must stall, for tests of the wedge path. Returns
/// true to make the worker stop answering the named layout's next call.
pub type WedgeHook = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The host's settings.
#[derive(Clone)]
pub struct HostConfig {
    /// The budgets each entry into Lua runs under.
    pub limits: Limits,
    /// The two fixed roots layout modules are found under: the shipped tree and the user's
    /// config directory. A `gnoblin.` name resolves under the shipped root alone. See
    /// [`ModuleRoots`] for why these are fixed rather than read from the document.
    pub roots: ModuleRoots,
    /// How long a worker may stay silent with work outstanding before it is abandoned.
    /// Registration waits at most this long for its load.
    pub hard_limit: Duration,
    /// How many abandoned workers are tolerated before layouts are disabled.
    pub leak_cap: usize,
    /// Test seam: makes the worker stall on a named layout. `None` in production.
    pub wedge: Option<WedgeHook>,
}

impl HostConfig {
    /// The production settings for layouts found under `roots`.
    pub fn new(roots: ModuleRoots) -> Self {
        Self {
            limits: Limits::default(),
            roots,
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

struct RegisteredLayout {
    module: String,
    options: Data,
    manifest: LayoutManifest,
}

/// The layout host. One per compositor; it owns the worker and the layout registry.
pub struct Host {
    config: HostConfig,
    worker: Option<Worker>,
    registered: BTreeMap<String, RegisteredLayout>,
    /// The last persisted `ctx.state` per layout, with the module hash it belongs to.
    states: BTreeMap<String, (u64, Data)>,
    generation: u64,
    /// Runs sent whose answer has not been received, by generation, with their layout.
    inflight: BTreeMap<u64, String>,
    /// Answers received but not yet taken by their caller.
    ready: BTreeMap<u64, Result<LayoutFinished, LayoutFault>>,
    /// Runs whose worker was abandoned before they answered.
    faulted: BTreeMap<u64, LayoutFault>,
    /// When the oldest run still in flight was sent.
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
            inflight: BTreeMap::new(),
            ready: BTreeMap::new(),
            faulted: BTreeMap::new(),
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

    /// The manifest of layout `name`, read when it loaded. `None` when no layout of that
    /// name is registered.
    pub(crate) fn manifest(&self, name: &str) -> Option<&LayoutManifest> {
        self.registered.get(name).map(|layout| &layout.manifest)
    }

    /// Whether a layout is registered under `name`. A caller reuses that registration
    /// rather than loading the module again, which would replace the options it carries.
    pub fn is_registered(&self, name: &str) -> bool {
        self.registered.contains_key(name)
    }

    /// Loads `module` and registers it as layout `name` with the user's `options`. The
    /// load is waited for at most one hard limit. A failed load restores the previous
    /// registration of `name`.
    pub fn register(&mut self, name: &str, module: &str, options: Data) -> Result<(), HostFailure> {
        if self.disabled {
            return Err(disabled_failure());
        }
        self.ensure_worker();
        if self.disabled {
            return Err(disabled_failure());
        }
        let generation = self.next_generation();
        let job = WorkerJob::Load {
            generation,
            name: name.to_owned(),
            module: module.to_owned(),
            options: options.clone(),
        };
        let deadline = Instant::now() + self.config.hard_limit;
        let outcome = if self.send(generation, name, job) {
            self.poll(generation, deadline)
        } else {
            Some(Err(LayoutFault::new(LayoutReason::VmDown, "the layout worker exited")))
        };
        let failure = match outcome {
            Some(Ok(LayoutFinished::Loaded(manifest))) => {
                let registration = RegisteredLayout {
                    module: module.to_owned(),
                    options,
                    manifest,
                };
                self.registered.insert(name.to_owned(), registration);
                return Ok(());
            }
            Some(Ok(LayoutFinished::Ran(_))) => HostFailure {
                reason: LayoutReason::InvalidLayout,
                message: "the worker answered a load with a layout call".into(),
            },
            Some(Err(fault)) => HostFailure {
                reason: fault.reason,
                message: fault.message,
            },
            None => {
                self.forget(generation);
                HostFailure {
                    reason: LayoutReason::Deadline,
                    message: format!("loading `{module}` did not finish in time"),
                }
            }
        };
        Err(failure)
    }

    /// Loads a layout that a configuration named by reference, after checking that the
    /// file the compositor is about to load is the file the configuration was checked
    /// against. `root_shipped` says which root the reference was written for. It must match
    /// the root `module` resolves under now, so a name that moved to the other root since the
    /// configuration was checked is refused.
    ///
    /// The hash is a change detector, not a security boundary. Anyone who can edit the
    /// user's layout files can already run code in the session, so it only answers
    /// "did this file change since the configuration was accepted?". A change is refused
    /// as `hash-mismatch` rather than loaded silently.
    pub fn register_reference(
        &mut self,
        name: &str,
        root_shipped: bool,
        module: &str,
        expected_hash: &str,
        options: Data,
    ) -> Result<(), HostFailure> {
        let resolved = resolve(&self.config.roots, module).map_err(|message| HostFailure {
            reason: LayoutReason::UnknownModule,
            message,
        })?;
        if root_shipped != (resolved.root == Root::Shipped) {
            return Err(HostFailure {
                reason: LayoutReason::UnknownModule,
                message: format!(
                    "layout `{module}` resolves under the {} root, but the configuration names the {} root",
                    resolved.root.as_str(),
                    if root_shipped { "shipped" } else { "user" },
                ),
            });
        }
        let source = read_source(&resolved.path, module).map_err(|message| HostFailure {
            reason: LayoutReason::MissingModule,
            message,
        })?;
        // The same hash `Modules::fetch` reports, so a reference written from a load
        // compares equal to the file it was written from.
        let actual = format!("{:016x}", fnv1a(source.as_bytes()));
        if actual != expected_hash {
            return Err(HostFailure {
                reason: LayoutReason::HashMismatch,
                message: format!(
                    "layout `{module}` has hash {actual} but the configuration expects {expected_hash}; the file changed after the configuration was checked"
                ),
            });
        }
        self.register(name, module, options)
    }

    /// Starts `call` on layout `name` over `snapshot` and returns its run generation
    /// without waiting. The run is collected with [`Host::poll`] and dropped with
    /// [`Host::forget`].
    pub(crate) fn start(
        &mut self,
        name: &str,
        call: LayoutCall,
        snapshot: &LayoutSnapshot,
    ) -> Result<u64, LayoutFault> {
        if self.disabled {
            return Err(disabled_fault());
        }
        if !self.registered.contains_key(name) {
            return Err(LayoutFault::new(
                LayoutReason::MissingModule,
                format!("layout `{name}` is not registered"),
            ));
        }
        self.ensure_worker();
        if self.disabled {
            return Err(disabled_fault());
        }
        let generation = self.next_generation();
        let job = WorkerJob::Run {
            generation,
            name: name.to_owned(),
            call,
            snapshot: snapshot.clone(),
            state: self.states.get(name).cloned(),
        };
        if self.send(generation, name, job) {
            Ok(generation)
        } else {
            Err(LayoutFault::new(LayoutReason::VmDown, "the layout worker exited"))
        }
    }

    /// Waits until run `generation` answers or `deadline` passes. `None` means it has
    /// not answered yet; the run stays in flight. A run whose worker was abandoned
    /// answers `VmWedged`.
    pub(crate) fn poll(&mut self, generation: u64, deadline: Instant) -> Option<Result<LayoutFinished, LayoutFault>> {
        loop {
            if let Some(result) = self.ready.remove(&generation) {
                return Some(result);
            }
            if let Some(fault) = self.faulted.remove(&generation) {
                return Some(Err(fault));
            }
            if !self.inflight.contains_key(&generation) {
                return Some(Err(LayoutFault::new(
                    LayoutReason::Error,
                    "the layout run was not outstanding",
                )));
            }
            if self.abandon_if_wedged() {
                continue;
            }
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(self.config.hard_limit);
            let received = match self.worker.as_ref() {
                Some(worker) => worker.recv_timeout(slice),
                None => {
                    self.fail_inflight(LayoutReason::VmDown, "the layout worker exited");
                    continue;
                }
            };
            match received {
                Ok(done) => self.absorb(done),
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.lost_worker();
                    self.fail_inflight(LayoutReason::VmDown, "the layout worker exited");
                }
            }
        }
    }

    /// Drops run `generation`: its answer, if it arrives, is discarded.
    pub(crate) fn forget(&mut self, generation: u64) {
        self.inflight.remove(&generation);
        self.ready.remove(&generation);
        self.faulted.remove(&generation);
        self.settle_clock();
    }

    /// Files an answer under its generation. Answers for runs nobody is waiting on are
    /// dropped. A finished layout call's persisted state is kept for the next call.
    fn absorb(&mut self, done: Done) {
        let Some(name) = self.inflight.remove(&done.generation) else {
            return;
        };
        if let Ok(LayoutFinished::Ran(ran)) = &done.result {
            self.states.insert(name, (ran.hash, ran.state.clone()));
        }
        self.ready.insert(done.generation, done.result);
        self.settle_clock();
    }

    /// Queues a job and records it as in flight. Returns false when the worker thread
    /// has exited.
    fn send(&mut self, generation: u64, name: &str, job: WorkerJob) -> bool {
        let sent = self.worker.as_ref().is_some_and(|worker| worker.send(job));
        if sent {
            self.inflight.insert(generation, name.to_owned());
            if self.busy_since.is_none() {
                self.busy_since = Some(Instant::now());
            }
        } else {
            self.lost_worker();
        }
        sent
    }

    /// Fails every run in flight with `reason`.
    fn fail_inflight(&mut self, reason: LayoutReason, message: &str) {
        for (generation, name) in std::mem::take(&mut self.inflight) {
            self.faulted.insert(
                generation,
                LayoutFault::new(reason, format!("layout `{name}`: {message}")),
            );
        }
        self.settle_clock();
    }

    /// Abandons the worker if runs have been in flight for a whole hard limit, and
    /// returns true when it did. After `leak_cap` abandoned workers, layouts are disabled.
    fn abandon_if_wedged(&mut self) -> bool {
        let wedged = match self.busy_since {
            Some(since) => !self.inflight.is_empty() && since.elapsed() >= self.config.hard_limit,
            None => false,
        };
        if !wedged {
            return false;
        }
        // Dropping the worker detaches its thread: the old thread is leaked, not joined.
        self.worker = None;
        self.leaked += 1;
        if self.leaked >= self.config.leak_cap {
            self.disabled = true;
        }
        self.fail_inflight(LayoutReason::VmWedged, "the worker stopped answering");
        true
    }

    /// Starts a worker if there is none, and replays every registration onto it.
    fn ensure_worker(&mut self) {
        if self.worker.is_some() {
            return;
        }
        self.worker = Some(spawn_worker(&self.config));
        let replay: Vec<(String, String, Data)> = self
            .registered
            .iter()
            .map(|(name, layout)| (name.clone(), layout.module.clone(), layout.options.clone()))
            .collect();
        for (name, module, options) in replay {
            // Generation 0 marks a replayed load: it is never in flight, so its answer is
            // dropped on arrival.
            let job = WorkerJob::Load {
                generation: 0,
                name,
                module,
                options,
            };
            if !self.worker.as_ref().is_some_and(|worker| worker.send(job)) {
                self.lost_worker();
                return;
            }
        }
    }

    /// The worker thread is gone. The next call starts a fresh one.
    fn lost_worker(&mut self) {
        self.worker = None;
    }

    /// Clears the clock when nothing is in flight.
    fn settle_clock(&mut self) {
        if self.inflight.is_empty() {
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
        roots: config.roots.clone(),
        wedge: config.wedge.clone(),
    })
}

fn disabled_fault() -> LayoutFault {
    LayoutFault::new(
        LayoutReason::VmWedged,
        "layouts are disabled after repeated wedged workers",
    )
}

fn disabled_failure() -> HostFailure {
    let fault = disabled_fault();
    HostFailure {
        reason: fault.reason,
        message: fault.message,
    }
}
