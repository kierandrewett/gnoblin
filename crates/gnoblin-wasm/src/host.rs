//! The host: load, start, deliver, stop and reload modules, and survive them.
//!
//! Every module gets its own [`wasmtime::Store`], its own resource table, its own
//! mirror of the graph and its own IPC connection. That is what makes the failure
//! rule in D6 true rather than aspirational: "a module that traps or misses its
//! deadline is unloaded; the compositor keeps running with the module's last
//! accepted state removed". A trap leaves one store unusable, and the host drops
//! that store, records why, and carries on with the other modules; nothing a module
//! did is visible to another module, and nothing it did is replayed afterwards.
//!
//! # Lifecycle
//!
//! - [`ModuleHost::load`] compiles and instantiates a component and stops there: no
//!   module code runs until [`ModuleHost::start`], so a module that is loaded is
//!   inert.
//! - [`ModuleHost::connect`] subscribes each loaded module to the graph through the
//!   in-process loopback (or, in the compositor, the socket: the client does not
//!   know the difference) and seeds its mirror with the snapshot. The snapshot's
//!   objects arrive as `opened` events on the module's first delivery, so a module
//!   sees the graph it inherited exactly like the graph it watched appear.
//! - [`ModuleHost::deliver`] drains the module's connection and calls its
//!   `on-event` once per observation. One budget covers the whole delivery: a
//!   module that spends it is killed, which is the only way to bound an event storm
//!   without silently dropping events.
//! - [`ModuleHost::stop`] calls the module's `stop` hook and drops its store.
//! - [`ModuleHost::reload`] builds the replacement first and swaps only when it is
//!   whole, so a module that fails to reload leaves the running one in place. The
//!   replacement starts unconnected: its store is new, so it must be connected and
//!   started like any other.
//!
//! # Deadlines
//!
//! A deadline is fuel and the budget is the caller's. Before every entry into a
//! module the store is refilled to `budget`; running out is a trap with
//! [`wasmtime::Trap::OutOfFuel`] as its root cause, which the host reports as
//! [`CallError::Deadline`] rather than as a module bug. Epochs would need a ticker
//! thread and a clock, and would make the same module succeed or fail depending on
//! what else the machine was doing.
//!
//! Compilation and instantiation also enter wasm (a component's core modules may
//! have start functions), so both happen under the same budget: loading is bounded
//! by the caller's budget too, and a component that does not finish instantiating
//! is refused rather than left half-built.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

use gnoblin_client::Server;
use gnoblin_model::{Filter, Observation, Snapshot};
use gnoblin_types::Version;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, StoreLimitsBuilder, Trap};

use crate::abi;
use crate::bindings::{Module, StoreData};
use crate::error::{CallError, ModuleError};
use crate::log::{self, Level, Sink};
use crate::manifest::{Budget, Capabilities, Manifest, ModuleLimits};
use crate::mirror::Mirror;

/// Where a module is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleState {
    /// Loaded and instantiated, with no module code run yet.
    Loaded,
    /// Started and accepting events.
    Running,
    /// Stopped by the host, or never started and unloaded.
    Stopped,
    /// Killed by a trap, a deadline overrun or an import the host could not
    /// satisfy. Nothing of it is running, and the reason is kept for the report.
    Failed {
        /// What the host saw, in one line.
        reason: String,
    },
}

impl ModuleState {
    /// The state's name, for a report line.
    pub fn as_str(&self) -> &'static str {
        match self {
            ModuleState::Loaded => "loaded",
            ModuleState::Running => "running",
            ModuleState::Stopped => "stopped",
            ModuleState::Failed { .. } => "failed",
        }
    }

    /// Why it failed, if it did.
    pub fn reason(&self) -> Option<&str> {
        match self {
            ModuleState::Failed { reason } => Some(reason),
            _ => None,
        }
    }
}

/// One loaded module: the store it runs in, and the generated bindings that call it.
struct Runtime {
    store: Store<StoreData>,
    bindings: Module,
}

/// A module's slot: what it was allowed, where it is, and the runtime if any.
struct ModuleSlot {
    capabilities: Capabilities,
    limits: ModuleLimits,
    state: ModuleState,
    runtime: Option<Runtime>,
}

/// The modules this host runs.
pub struct ModuleHost {
    engine: Engine,
    linker: Linker<StoreData>,
    sink: log::Shared,
    modules: BTreeMap<String, ModuleSlot>,
}

impl std::fmt::Debug for ModuleHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ModuleHost")
            .field("modules", &self.modules.keys().collect::<Vec<&String>>())
            .finish()
    }
}

impl ModuleHost {
    /// A host with no modules, reporting through standard error.
    pub fn new() -> Result<ModuleHost, ModuleError> {
        ModuleHost::with_sink(Box::new(log::StderrSink))
    }

    /// A host reporting through `sink`.
    pub fn with_sink(sink: Box<dyn Sink>) -> Result<ModuleHost, ModuleError> {
        // The engine is configured once and never again: fuel is how a deadline is
        // measured, and the component model is the shape of the host API. Nothing
        // here enables a proposal the ABI does not use, and no WASI implementation
        // is linked anywhere, so a module that imports `wasi:*` fails to link.
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|error| ModuleError::Link {
            module: "engine".to_owned(),
            reason: error.to_string(),
        })?;
        let mut linker = Linker::new(&engine);
        Module::add_to_linker::<_, HasSelf<_>>(&mut linker, |data: &mut StoreData| data).map_err(|error| {
            ModuleError::Link {
                module: "linker".to_owned(),
                reason: error.to_string(),
            }
        })?;
        Ok(ModuleHost {
            engine,
            linker,
            sink: std::sync::Arc::new(std::sync::Mutex::new(sink)),
            modules: BTreeMap::new(),
        })
    }

    /// Load and instantiate a component under `manifest`.
    ///
    /// Nothing of the module runs until [`ModuleHost::start`].
    pub fn load(&mut self, manifest: Manifest, bytes: &[u8], budget: Budget) -> Result<(), ModuleError> {
        let name = manifest.name().to_owned();
        if self.modules.contains_key(&name) {
            return Err(ModuleError::Duplicate { module: name });
        }
        manifest.granted().check().map_err(|reason| ModuleError::Unsupported {
            module: name.clone(),
            reason,
        })?;
        let runtime = self.build(&name, manifest.granted().clone(), manifest.bounds(), bytes, budget)?;
        self.modules.insert(
            name,
            ModuleSlot {
                capabilities: manifest.granted().clone(),
                limits: manifest.bounds(),
                state: ModuleState::Loaded,
                runtime: Some(runtime),
            },
        );
        Ok(())
    }

    /// Load a component from a file, with the same rules as [`ModuleHost::load`].
    pub fn load_file(&mut self, manifest: Manifest, path: &Path, budget: Budget) -> Result<(), ModuleError> {
        let bytes = std::fs::read(path).map_err(|error| ModuleError::Read {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        self.load(manifest, &bytes, budget)
    }

    /// Replace a module's component, keeping the old one if the new one fails.
    ///
    /// The replacement is compiled, linked and instantiated before the running one
    /// is touched, and it is installed only when all of that succeeded: a broken
    /// reload leaves the module exactly as it was, still running. A successful one
    /// starts unconnected and unstarted, because its store is new.
    pub fn reload(&mut self, name: &str, bytes: &[u8], budget: Budget) -> Result<(), ModuleError> {
        let Some(slot) = self.modules.get(name) else {
            return Err(ModuleError::Missing {
                module: name.to_owned(),
            });
        };
        let capabilities = slot.capabilities.clone();
        let limits = slot.limits;
        let runtime = self.build(name, capabilities, limits, bytes, budget)?;
        let slot = self.modules.get_mut(name).ok_or_else(|| ModuleError::Missing {
            module: name.to_owned(),
        })?;
        slot.runtime = Some(runtime);
        slot.state = ModuleState::Loaded;
        Ok(())
    }

    /// Unload a module: stop its runtime without a hook and forget it.
    ///
    /// Returns whether a module with that name was loaded.
    pub fn unload(&mut self, name: &str) -> bool {
        self.modules.remove(name).is_some()
    }

    /// Connect every loaded module to the graph and seed its mirror.
    ///
    /// One connection and one subscription per module, both `Filter::All`: the
    /// capability check decides what reaches a module, and a filtered subscription
    /// would have to know the identities before the module has seen them. A module
    /// that is already connected is left alone, so this is safe to call again after
    /// a reload.
    pub fn connect(&mut self, server: &mut Server, version: Version) -> Result<(), ModuleError> {
        for (name, slot) in &mut self.modules {
            let Some(runtime) = slot.runtime.as_mut() else {
                continue;
            };
            if runtime.store.data().client.is_some() {
                continue;
            }
            let mut client = server.connect(version).map_err(|source| ModuleError::Connect {
                module: name.clone(),
                source,
            })?;
            let snapshot = client.subscribe(Filter::All).map_err(|source| ModuleError::Subscribe {
                module: name.clone(),
                source,
            })?;
            let data = runtime.store.data_mut();
            data.mirror = Mirror::seed(&snapshot);
            data.pending = opening(&snapshot, &data.capabilities);
            data.client = Some(client);
        }
        Ok(())
    }

    /// Run a module's `start` hook.
    pub fn start(&mut self, name: &str, budget: Budget) -> Result<(), CallError> {
        let run = {
            let runtime = self.runtime(name, &[ModuleState::Loaded])?;
            runtime
                .store
                .set_fuel(budget.as_fuel())
                .and_then(|()| Module::gnoblin_ipc_lifecycle(&runtime.bindings).call_start(&mut runtime.store))
        };
        settle(&mut self.modules, &self.sink, name, run, budget)?;
        if let Some(slot) = self.modules.get_mut(name) {
            slot.state = ModuleState::Running;
        }
        Ok(())
    }

    /// Deliver every observation the module is owed, returning how many were
    /// delivered.
    ///
    /// The module's whole budget covers the delivery, so an event storm cannot make
    /// a single `deliver` call unbounded. Observations for objects the module did
    /// not ask about still update its mirror (a module that starts watching an
    /// object later reads current values) but are not delivered.
    pub fn deliver(&mut self, name: &str, budget: Budget) -> Result<usize, CallError> {
        let mut delivered = 0usize;
        let mut killed = None;
        {
            let runtime = self.runtime(name, &[ModuleState::Running])?;
            runtime
                .store
                .set_fuel(budget.as_fuel())
                .map_err(|error| CallError::Trapped {
                    module: name.to_owned(),
                    reason: error.to_string(),
                })?;
            loop {
                let next = {
                    let data = runtime.store.data_mut();
                    match data.pending.pop_front() {
                        Some(observation) => Some(Ok(Some(observation))),
                        None => data.client.as_mut().map(|client| client.next_event().map(observation)),
                    }
                };
                let Some(next) = next else {
                    break;
                };
                let observation = match next {
                    Ok(Some(observation)) => observation,
                    // A request the module could claim or a request that ended. The
                    // request layer is not part of the module ABI yet: a module reaches
                    // a request through the `request` object's methods, so these two
                    // carry nothing it has not already been told.
                    Ok(None) => continue,
                    Err(gnoblin_client::ClientError::Disconnected) => break,
                    Err(source) => {
                        return Err(CallError::Client {
                            module: name.to_owned(),
                            source,
                        });
                    }
                };
                let data = runtime.store.data_mut();
                data.mirror.apply(&observation);
                match &observation {
                    Observation::Opened { view, .. } => {
                        // An object a module may read and subscribe to is watched from the
                        // moment it opens: this is how a module learns that anything
                        // exists, and requiring a `watch` call first would need the
                        // identity it has not been told yet.
                        if data.capabilities.may_subscribe() && data.capabilities.may_read(view.ty().name()) {
                            data.watched.insert(view.id());
                        }
                    }
                    Observation::Finished { operation, outcome, .. } => {
                        data.outcomes.insert(*operation, outcome.clone());
                    }
                    _ => {}
                }
                let Some((kind, object, member, argument)) = words(data, &observation) else {
                    continue;
                };
                let call = Module::gnoblin_ipc_callbacks(&runtime.bindings).call_on_event(
                    &mut runtime.store,
                    kind,
                    object,
                    member,
                    argument,
                );
                delivered += 1;
                if let Err(error) = call {
                    killed = Some(failure(name, error, budget));
                    break;
                }
            }
        }
        match killed {
            Some(error) => {
                kill(&mut self.modules, &self.sink, name, &error);
                Err(error)
            }
            None => Ok(delivered),
        }
    }

    /// Run a module's `stop` hook and drop its store.
    ///
    /// A module that traps while stopping is still stopped: its store is dropped
    /// either way, and the trap is reported as the failure that ended it.
    pub fn stop(&mut self, name: &str, budget: Budget) -> Result<(), CallError> {
        let state = self
            .modules
            .get(name)
            .map(|slot| slot.state.clone())
            .ok_or_else(|| CallError::Missing {
                module: name.to_owned(),
            })?;
        if state == ModuleState::Stopped {
            return Ok(());
        }
        if state == ModuleState::Running {
            let run = {
                let runtime = self.runtime(name, &[ModuleState::Running])?;
                runtime
                    .store
                    .set_fuel(budget.as_fuel())
                    .and_then(|()| Module::gnoblin_ipc_lifecycle(&runtime.bindings).call_stop(&mut runtime.store))
            };
            settle(&mut self.modules, &self.sink, name, run, budget)?;
        }
        if let Some(slot) = self.modules.get_mut(name) {
            slot.runtime = None;
            slot.state = ModuleState::Stopped;
        }
        Ok(())
    }

    /// Where a module is, or `None` when nothing has that name.
    pub fn state(&self, name: &str) -> Option<&ModuleState> {
        self.modules.get(name).map(|slot| &slot.state)
    }

    /// The names of the loaded modules and their states, in name order.
    pub fn modules(&self) -> impl Iterator<Item = (&str, &ModuleState)> {
        self.modules.iter().map(|(name, slot)| (name.as_str(), &slot.state))
    }

    /// The runtime of a module that is in one of `states`.
    fn runtime(&mut self, name: &str, states: &[ModuleState]) -> Result<&mut Runtime, CallError> {
        let slot = self.modules.get_mut(name).ok_or_else(|| CallError::Missing {
            module: name.to_owned(),
        })?;
        if !states.contains(&slot.state) {
            return Err(CallError::NotRunning {
                module: name.to_owned(),
                state: slot.state.clone(),
            });
        }
        slot.runtime.as_mut().ok_or_else(|| CallError::NotRunning {
            module: name.to_owned(),
            state: slot.state.clone(),
        })
    }

    /// Compile, link and instantiate a component without installing it.
    fn build(
        &self,
        name: &str,
        capabilities: Capabilities,
        limits: ModuleLimits,
        bytes: &[u8],
        budget: Budget,
    ) -> Result<Runtime, ModuleError> {
        let component = Component::new(&self.engine, bytes).map_err(|error| ModuleError::Compile {
            module: name.to_owned(),
            reason: error.to_string(),
        })?;
        let bounds = StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes)
            .table_elements(limits.table_elements as usize)
            .memories(limits.memories)
            .tables(limits.tables)
            // A component is not one core instance: the wrapper the canonical ABI
            // builds around a module, the module itself and a shared memory are each
            // an instance. The count is a fixed, small ceiling rather than a knob,
            // because a module that instantiates dozens of modules is not doing what
            // this host offers.
            .instances(16)
            .build();
        let mut store = Store::new(
            &self.engine,
            StoreData::new(name.to_owned(), capabilities, bounds, std::sync::Arc::clone(&self.sink)),
        );
        store.limiter(|data| &mut data.limits as &mut dyn ResourceLimiter);
        store.set_fuel(budget.as_fuel()).map_err(|error| ModuleError::Link {
            module: name.to_owned(),
            reason: error.to_string(),
        })?;
        let bindings = Module::instantiate(&mut store, &component, &self.linker).map_err(|error| {
            // A module that imports something the host does not provide fails here,
            // and the message names the import: this is where "no WASI filesystem,
            // no network" is enforced, by there being nothing to link against.
            ModuleError::Link {
                module: name.to_owned(),
                reason: error.to_string(),
            }
        })?;
        Ok(Runtime { store, bindings })
    }
}

/// Turn one entry's result into the host's decision.
///
/// A trap or a deadline overrun kills the module: its store is dropped so nothing
/// half-run can be called again, the state records why, and the error carries the
/// same reason to the caller. A plain success marks nothing, because the caller
/// knows which state it asked for.
fn settle(
    modules: &mut BTreeMap<String, ModuleSlot>,
    sink: &log::Shared,
    name: &str,
    run: wasmtime::Result<()>,
    budget: Budget,
) -> Result<(), CallError> {
    match run {
        Ok(()) => Ok(()),
        Err(error) => {
            let failure = failure(name, error, budget);
            kill(modules, sink, name, &failure);
            Err(failure)
        }
    }
}

/// What a failed entry into a module was.
///
/// Running out of fuel is a deadline and not a module bug: the module was killed by
/// the budget the caller granted, which is the whole point of granting one.
fn failure(name: &str, error: wasmtime::Error, budget: Budget) -> CallError {
    match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => CallError::Deadline {
            module: name.to_owned(),
            fuel: budget.as_fuel(),
        },
        _ => CallError::Trapped {
            module: name.to_owned(),
            // The rendered error is wasmtime's context ("error while executing at
            // wasm backtrace ...") and the trap itself is its root cause, so the
            // reason names both: one says where, the other says what.
            reason: match error.downcast_ref::<Trap>() {
                Some(trap) => format!("{error}: {trap}"),
                None => error.to_string(),
            },
        },
    }
}

/// Remove a dead module's runtime and record why, writing one line.
fn kill(modules: &mut BTreeMap<String, ModuleSlot>, sink: &log::Shared, name: &str, failure: &CallError) {
    let reason = failure.to_string();
    if let Some(slot) = modules.get_mut(name) {
        slot.runtime = None;
        slot.state = ModuleState::Failed { reason: reason.clone() };
    }
    log::write(
        sink,
        &log::Line {
            module: name,
            level: Level::Error,
            message: &reason,
        },
    );
}

/// The `opened` events a snapshot becomes, for the types the module may read and
/// subscribe to.
///
/// A module that is connected after the graph already has objects still has to see
/// them, and delivering them as the same `opened` event a later object produces
/// keeps one rule in the ABI instead of a special "initial state" call.
fn opening(snapshot: &Snapshot, capabilities: &Capabilities) -> VecDeque<Observation> {
    let mut opening = VecDeque::new();
    if !capabilities.may_subscribe() {
        return opening;
    }
    for view in snapshot.objects() {
        if !capabilities.may_read(view.ty().name()) {
            continue;
        }
        opening.push_back(Observation::Opened {
            view: view.clone(),
            revision: snapshot.revision(),
        });
    }
    opening
}

/// The words one observation becomes, or `None` when the module did not ask for it.
fn words(data: &StoreData, observation: &Observation) -> Option<(u32, u64, u32, u64)> {
    if !data.capabilities.may_subscribe() {
        return None;
    }
    let (kind, object) = match observation {
        Observation::Opened { view, .. } => {
            if !data.capabilities.may_read(view.ty().name()) {
                return None;
            }
            (abi::kind::OPENED, view.id())
        }
        Observation::Changed { object, .. } => (abi::kind::CHANGED, *object),
        Observation::Signalled { object, .. } => (abi::kind::SIGNALLED, *object),
        Observation::Closed { object, .. } => (abi::kind::CLOSED, *object),
        Observation::Finished { object, .. } => (abi::kind::OPERATION_FINISHED, *object),
    };
    if !data.watched.contains(&object) {
        return None;
    }
    let member = match observation {
        Observation::Changed { property, .. } => data.member(object, property)?,
        Observation::Signalled { signal, .. } => data.member(object, signal)?,
        _ => 0,
    };
    Some((kind, object.get(), member, argument_of(observation)))
}

/// The operation identity a completion carries, and `0` for everything else.
fn argument_of(observation: &Observation) -> u64 {
    match observation {
        Observation::Finished { operation, .. } => operation.get(),
        _ => 0,
    }
}

impl StoreData {
    /// The event index of one member of one object in the mirror.
    fn member(&self, object: gnoblin_model::ObjectId, name: &str) -> Option<u32> {
        let entry = self.mirror.get(object)?;
        abi::member_index(entry.ty(), name)
    }
}

/// The observation a client event is, or `None` for the request layer.
fn observation(event: gnoblin_client::Event) -> Option<Observation> {
    match event {
        gnoblin_client::Event::Model(observation) => Some(observation),
        // A request offer or finish. The request layer is not part of the module
        // ABI yet: a module reaches a request through the `request` object's
        // methods, so these two carry nothing it has not already been told.
        gnoblin_client::Event::RequestOffered { .. } | gnoblin_client::Event::RequestFinished { .. } => None,
    }
}
