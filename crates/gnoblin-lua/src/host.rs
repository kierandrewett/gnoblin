//! The host: the Lua state, the object mirror, and the pump that drives both.
//!
//! The runtime is a privileged IPC client (design/rust-compositor.md, D1). It
//! subscribes once, keeps a mirror of the graph from the snapshot and the property
//! diffs, answers reads from that mirror, turns method calls into operations
//! through the client, and runs handlers when the caller pumps it.
//!
//! Four ordering rules make the behaviour predictable, and each is what the
//! plan's smaller decisions asks for:
//!
//! - **Reads are local.** A property read never touches the connection; it is the
//!   mirror, which the last observation already updated. A handler that reads
//!   after an observation sees that observation.
//! - **One operation handle, one completion.** A method returns one `Operation`
//!   and one `Finished` observation completes it. A second completion for the same
//!   identity is refused by the graph and ignored here, so a callback that can run
//!   once does not run twice.
//! - **Handlers never run inside handlers.** A signal handler runs while the
//!   observation is dispatched, but a completion callback, and an `on_complete`
//!   registered after the operation already finished, run at the end of the pump.
//!   A handler that calls a method therefore cannot re-enter itself.
//! - **One timer per identity, on the caller's clock.** [`Host::pump`] is handed
//!   the tick; a timer fires when the pump reaches its due tick. The host reads no
//!   clock, so a test replays any sequence and gets the same handlers.
//!
//! An erroring handler never ends the host: it is reported once per site
//! ([`crate::ScriptError`]) and the pump continues.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_config::{Config, ConfigPath, Report, SearchPaths, Value, locate};
use gnoblin_model::{Fields, Filter, ObjectId, ObjectView, Observation, OperationId, OperationOutcome, Tick};
use mlua::Value as LuaValue;
use mlua::prelude::LuaChunkMode;
use mlua::{Error, Lua};

use crate::error::{HostError, ScriptError};
use crate::extensions::ExtensionRegistry;
use crate::loader::{self, Resolution};
use gnoblin_lua_sandbox::{Guard, Limits, Profile};

/// How many configuration files one load may nest, the limit the C core applies.
pub const MAX_LOADS: usize = 32;

/// The state of one operation a handler can wait on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Status {
    /// No completion has arrived.
    Pending,
    /// The completion was a success.
    Succeeded,
    /// The completion failed with the schema's error code.
    Failed(String),
}

/// What one registered handler is waiting for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Watching {
    /// A signal on one object, delivered while the observation is dispatched.
    Signal { object: ObjectId, signal: String },
    /// An extension signal, matched by namespace and `Object.Signal` member.
    Extension { namespace: String, member: String },
    /// The single completion of one operation, delivered at the end of the pump.
    Completion { operation: OperationId },
}

/// What the host knows about one operation it called.
#[derive(Debug)]
pub(super) struct Pending {
    /// How it ended, once the single completion arrived.
    pub(super) outcome: Option<OperationOutcome>,
    /// The handlers waiting for that completion, in registration order.
    pub(super) completions: Vec<u64>,
}

/// The host's state, shared with the Lua closures the bindings install.
///
/// It is behind a [`RefCell`] because a Lua callback reaches into it while the
/// host is inside the VM. Every borrow is short and ends before the host calls
/// into Lua: a borrow live across a Lua call would panic the moment a metamethod
/// or a callback reached back in.
///
/// The callbacks themselves are not here. A Lua function is kept in the Lua
/// registry under its handler identity ([`crate::bindings::CALLBACKS`]), so this
/// struct holds no Lua handle and the host and the VM cannot keep each other
/// alive through a cycle.
pub(super) struct Shared<T: Transport> {
    pub(super) client: Client<T>,
    pub(super) mirror: BTreeMap<ObjectId, ObjectView>,
    pub(super) document: Value,
    /// The caller's clock, as of the last pump; timers are due against it.
    pub(super) clock: Tick,
    pub(super) root: Option<PathBuf>,
    pub(super) stack: Vec<PathBuf>,
    pub(super) loaded: Vec<PathBuf>,
    /// One entry per call the host made, kept for the life of the host: the
    /// handle a script holds has to keep answering `status` and to accept an
    /// `on_complete`. The runtime process is restartable, so the entries do not
    /// outlive a session by much.
    pub(super) operations: BTreeMap<OperationId, Pending>,
    pub(super) handlers: BTreeMap<u64, Watching>,
    pub(super) timers: BTreeMap<u64, Tick>,
    /// Handlers that run at the end of the current pump.
    pub(super) deferred: Vec<u64>,
    pub(super) next_id: u64,
    /// Whether a configuration is loading, which is when `gnoblin.configure` may
    /// change the document.
    pub(super) configuring: bool,
    /// Error sites already reported, so a repeat stays quiet.
    pub(super) reported: BTreeSet<String>,
    /// Errors reported since the caller last drained them.
    pub(super) reports: Vec<ScriptError>,
    /// The extension schemas `require` binds from; a signal's declared fields come from here.
    pub(super) extensions: ExtensionRegistry,
    /// The loader that answers `require` for a layout module with a reference. It holds
    /// the one layout VM of the current configuration evaluation.
    pub(super) layouts: crate::layout::LayoutModuleLoader,
    /// The configuration VM's modules for one evaluation, so a plain `require` runs each
    /// module once. `None` outside an evaluation.
    pub(super) modules: Option<Rc<crate::layout::Modules>>,
    /// Set when the compositor asks for a reload, and cleared by [`Host::take_reload_request`].
    /// The request arrives as a signal while the pump observes the graph, so the caller acts on
    /// it once the pump returns.
    pub(super) reload_requested: bool,
}

impl<T: Transport> Shared<T> {
    /// The next handler or timer identity.
    pub(super) fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Schedule a handler to run at the end of the current pump.
    pub(super) fn defer(&mut self, id: u64) {
        self.deferred.push(id);
    }

    /// Remember what `handler` waits for, and schedule it if it is already due.
    pub(super) fn watch(&mut self, handler: u64, watching: Watching) {
        if let Watching::Completion { operation } = &watching {
            match self.operations.get_mut(operation) {
                Some(pending) if pending.outcome.is_none() => pending.completions.push(handler),
                // Already finished, or never called here. Either way the answer
                // exists now, so the callback runs at the end of this pump rather
                // than never.
                _ => self.deferred.push(handler),
            }
        }
        self.handlers.insert(handler, watching);
    }

    /// Forget a handler. Answers whether it was still registered.
    pub(super) fn cancel_handler(&mut self, id: u64) -> bool {
        self.handlers.remove(&id).is_some()
    }

    /// The handlers waiting on one object's signal, in registration order.
    pub(super) fn signal_handlers(&self, object: ObjectId, signal: &str) -> Vec<u64> {
        self.handlers
            .iter()
            .filter(|(_, watching)| {
                matches!(watching, Watching::Signal { object: watched, signal: watched_signal }
                    if *watched == object && watched_signal == signal)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The handlers waiting on one extension signal, in registration order.
    pub(super) fn extension_handlers(&self, namespace: &str, member: &str) -> Vec<u64> {
        self.handlers
            .iter()
            .filter(|(_, watching)| {
                matches!(watching, Watching::Extension { namespace: watched, member: watched_member }
                    if watched == namespace && watched_member == member)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Call a declared method and record the operation its completion arrives on.
    pub(super) fn call(
        &mut self,
        object: ObjectId,
        method: &str,
        arguments: &Fields,
    ) -> Result<OperationId, ClientError> {
        let operation = self.client.call(object, method, arguments)?;
        self.operations.insert(
            operation.id(),
            Pending {
                outcome: None,
                completions: Vec::new(),
            },
        );
        Ok(operation.id())
    }

    /// How one operation stands.
    pub(super) fn status(&self, operation: OperationId) -> Status {
        match self
            .operations
            .get(&operation)
            .and_then(|pending| pending.outcome.as_ref())
        {
            None => Status::Pending,
            Some(OperationOutcome::Done | OperationOutcome::Value(_)) => Status::Succeeded,
            Some(OperationOutcome::Failed(code)) => Status::Failed(code.as_str().to_owned()),
        }
    }

    /// The error code of a failed operation.
    pub(super) fn code(&self, operation: OperationId) -> Option<String> {
        match self.status(operation) {
            Status::Failed(code) => Some(code),
            _ => None,
        }
    }
}

/// One run of the pump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PumpReport {
    /// How many events the connection delivered.
    pub events: usize,
    /// How many handlers ran, including timers and completions.
    pub handlers: usize,
    /// Errors reported during this pump, once per site.
    pub errors: Vec<ScriptError>,
}

/// One configuration run.
#[derive(Debug)]
pub struct RunReport {
    /// The file `locate` chose.
    pub path: ConfigPath,
    /// The files actually evaluated, in load order.
    pub loaded: Vec<PathBuf>,
    /// The merged document the files produced.
    pub document: Value,
    /// The document validated by the configuration layer, or every problem.
    pub config: Result<Config, Report>,
    /// The script errors the run reported, one per site.
    pub errors: Vec<ScriptError>,
}

/// The signal the compositor raises on `session` when `gnoblinctl config reload` asks for a
/// reload. The name is the one `session.toml` declares.
const RELOAD_REQUESTED: &str = "reload_requested";

/// What a reload did. A rejected reload leaves the previous configuration in force.
pub enum ReloadOutcome {
    /// The new document is in force, and this run is the one to commit.
    Accepted(RunReport),
    /// The new configuration failed, with each problem named by file and line.
    Rejected(String),
}

/// The state a reload replaces, kept so a rejected reload can put it back.
struct Previous {
    handlers: BTreeMap<u64, Watching>,
    timers: BTreeMap<u64, Tick>,
    deferred: Vec<u64>,
    document: Value,
    callbacks: mlua::Table,
}

/// Why a reload was refused: every script error and the configuration problems, in one line.
fn refusal(report: &RunReport) -> String {
    let mut problems: Vec<String> = report.errors.iter().map(ToString::to_string).collect();
    if let Err(invalid) = &report.config {
        problems.push(invalid.to_string());
    }
    format!(
        "the reload of {} was refused, and the last accepted configuration stays in force: {}",
        report.path.path().display(),
        problems.join("; ")
    )
}

/// The host's receiver for script `print` and `warn` output: routine output at debug level,
/// deliberate warnings at warn level, both under the `lua` component.
pub(crate) fn script_sink() -> gnoblin_lua_sandbox::Sink {
    Rc::new(|level, text| match level {
        gnoblin_lua_sandbox::LogLevel::Debug => crate::log::debug!(area = "script", "{text}"),
        gnoblin_lua_sandbox::LogLevel::Warn => crate::log::warn!(area = "script", "{text}"),
    })
}

/// The Lua host.
pub struct Host<T: Transport> {
    lua: Lua,
    shared: Rc<RefCell<Shared<T>>>,
    guard: Rc<Guard>,
    limits: Limits,
}

impl<T: Transport + 'static> Host<T> {
    /// Connect the host to a live graph and build its sandbox.
    ///
    /// The subscription happens here, before any script runs, so a property read
    /// is never an empty mirror because of start-up order.
    pub fn new(client: Client<T>, limits: Limits) -> Result<Host<T>, HostError> {
        Host::with_extensions(client, limits, &ExtensionRegistry::new())
    }

    /// Connect the host to a live graph, with the extension namespaces it has loaded.
    ///
    /// `require` answers only for a namespace that is loaded here and granted to this
    /// connection by the compositor. A loaded namespace that was not granted is refused
    /// by name, and a namespace that is not loaded is an unknown module.
    pub fn with_extensions(
        mut client: Client<T>,
        limits: Limits,
        extensions: &ExtensionRegistry,
    ) -> Result<Host<T>, HostError> {
        let snapshot = client.subscribe(Filter::All).map_err(HostError::Subscribe)?;
        let mirror: BTreeMap<ObjectId, ObjectView> = snapshot
            .objects()
            .iter()
            .map(|view| (view.id(), view.clone()))
            .collect();
        let guard = Rc::new(Guard::new());
        let lua = gnoblin_lua_sandbox::create(Profile::Config, &limits, Rc::clone(&guard), script_sink())
            .map_err(HostError::Sandbox)?;
        let shared = Rc::new(RefCell::new(Shared {
            client,
            mirror,
            document: Value::Map(BTreeMap::new()),
            clock: Tick::default(),
            root: None,
            stack: Vec::new(),
            loaded: Vec::new(),
            operations: BTreeMap::new(),
            handlers: BTreeMap::new(),
            timers: BTreeMap::new(),
            deferred: Vec::new(),
            next_id: 1,
            configuring: false,
            reported: BTreeSet::new(),
            reports: Vec::new(),
            extensions: ExtensionRegistry::new(),
            layouts: crate::layout::LayoutModuleLoader::new(limits),
            modules: None,
            reload_requested: false,
        }));
        crate::bindings::install(&lua, Rc::clone(&shared), Rc::clone(&guard), limits).map_err(HostError::Sandbox)?;
        let mut host = Host {
            lua,
            shared,
            guard,
            limits,
        };
        host.bind_extensions(extensions)?;
        Ok(host)
    }

    /// Bind the extension namespaces this connection was granted, from `extensions`.
    ///
    /// A namespace gets a `require` entry only when the host loaded it and the compositor
    /// granted it to this connection. Calling this again replaces the entries, so a host
    /// is bound once from the registry the compositor built.
    pub fn bind_extensions(&mut self, extensions: &ExtensionRegistry) -> Result<(), HostError> {
        let granted: BTreeSet<String> = self
            .shared
            .borrow()
            .client
            .granted_extensions()
            .iter()
            .map(|grant| grant.namespace.clone())
            .collect();
        self.shared.borrow_mut().extensions = extensions.clone();
        crate::extension_bindings::bind(&self.lua, &self.shared, extensions, &granted).map_err(HostError::Sandbox)
    }

    /// The connection, for a role that has a privileged member to send.
    ///
    /// A host owns the connection for the mirror, so the one caller that needs to commit a
    /// document reaches it here rather than constructing a second client on the same
    /// socket, which would interleave two frame streams.
    pub fn client_mut(&self) -> std::cell::RefMut<'_, Client<T>> {
        std::cell::RefMut::map(self.shared.borrow_mut(), |shared| &mut shared.client)
    }

    /// The budgets this host runs under.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The merged configuration document so far.
    pub fn document(&self) -> Value {
        self.shared.borrow().document.clone()
    }

    /// Validate the merged document, reporting every problem at once.
    pub fn validate(&self) -> Result<Config, Report> {
        Config::from_value(self.document())
    }

    /// Locate and evaluate the configuration, then validate what it produced.
    ///
    /// A missing file is not an error: a first login runs the built-in
    /// configuration and this host has nothing to evaluate. A file that fails is
    /// reported and skipped, and the rest of the document stands, which is the
    /// login behaviour the plan keeps for salvage.
    pub fn run_config(&mut self, paths: &SearchPaths) -> Result<RunReport, HostError> {
        let path = locate(paths).map_err(HostError::Locate)?;
        self.reset_runtime()?;
        // The shipped root is the installed prefix; the user root is the directory of the
        // configuration file being evaluated, so a `require` there resolves from one place.
        let shipped = crate::layout::installed_module_dir(&crate::layout::default_prefix(
            std::env::var_os("GNOBLIN_PREFIX").as_deref(),
        ));
        let user = path.path().parent().map(Path::to_path_buf);
        let roots = crate::layout::ModuleRoots { shipped, user };
        {
            let mut shared = self.shared.borrow_mut();
            shared.layouts.reset(roots.clone());
            shared.modules = Some(Rc::new(crate::layout::Modules::new(roots)));
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.document = Value::Map(BTreeMap::new());
            shared.loaded.clear();
            shared.stack.clear();
            shared.root = None;
        }
        if path.exists() {
            let dir = path.path().parent().map(Path::to_path_buf);
            let root = match dir {
                Some(dir) => fs::canonicalize(&dir).map_err(|source| HostError::Read {
                    path: dir.clone(),
                    source,
                })?,
                None => {
                    return Err(HostError::Read {
                        path: path.path().to_path_buf(),
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "the config path has no directory",
                        ),
                    });
                }
            };
            self.shared.borrow_mut().root = Some(root);
            let file = path.path().to_path_buf();
            self.shared.borrow_mut().configuring = true;
            // The root file is the file the directory is derived from, so it is
            // evaluated directly rather than through the confinement check that
            // every later file goes through.
            let outcome = evaluate_file(&self.lua, &self.shared, &self.guard, &self.limits, &file);
            self.shared.borrow_mut().configuring = false;
            if let Err(error) = outcome {
                self.report(error);
            }
        }
        let document = self.document();
        let loaded = self.shared.borrow().loaded.clone();
        let config = Config::from_value(document.clone());
        let errors = std::mem::take(&mut self.shared.borrow_mut().reports);
        Ok(RunReport {
            path,
            loaded,
            document,
            config,
            errors,
        })
    }

    /// Evaluate the configuration again while the host keeps running, so a reload does not
    /// restart the runtime.
    ///
    /// The previous handlers, timers and document are set aside while the new file runs. A
    /// run that reports any failure puts them back, so the last accepted configuration keeps
    /// answering signals. A clean run replaces them.
    pub fn reload_config(&mut self, paths: &SearchPaths) -> Result<ReloadOutcome, HostError> {
        let callbacks: mlua::Table = self
            .lua
            .named_registry_value(crate::bindings::CALLBACKS)
            .map_err(HostError::Sandbox)?;
        let previous = {
            let mut shared = self.shared.borrow_mut();
            // A failure the last run already reported must show again on this run, or a reload
            // that fails the same way looks like one that succeeded.
            shared.reported.clear();
            Previous {
                handlers: std::mem::take(&mut shared.handlers),
                timers: std::mem::take(&mut shared.timers),
                deferred: std::mem::take(&mut shared.deferred),
                document: shared.document.clone(),
                callbacks,
            }
        };
        let run = self.run_config(paths);
        // The drain inside `run_config` may have seen reload requests already queued. This
        // reload answers them, so they must not start a second one.
        self.shared.borrow_mut().reload_requested = false;
        let run = match run {
            Ok(run) => run,
            Err(error) => {
                self.restore(previous)?;
                return Err(error);
            }
        };
        if run.errors.is_empty() && run.config.is_ok() {
            return Ok(ReloadOutcome::Accepted(run));
        }
        let reason = refusal(&run);
        self.restore(previous)?;
        Ok(ReloadOutcome::Rejected(reason))
    }

    /// Put back the state a rejected reload replaced.
    fn restore(&mut self, previous: Previous) -> Result<(), HostError> {
        {
            let mut shared = self.shared.borrow_mut();
            shared.handlers = previous.handlers;
            shared.timers = previous.timers;
            shared.deferred = previous.deferred;
            shared.document = previous.document;
        }
        self.lua
            .set_named_registry_value(crate::bindings::CALLBACKS, previous.callbacks)
            .map_err(HostError::Sandbox)
    }

    /// Whether the compositor has asked for a reload since the last call. Taking the request
    /// clears it, so one request runs one reload.
    pub fn take_reload_request(&self) -> bool {
        std::mem::take(&mut self.shared.borrow_mut().reload_requested)
    }

    /// Whether `object` is the `session` object, the only one whose reload request the runtime acts on.
    fn is_session(&self, object: ObjectId) -> bool {
        self.shared.borrow().mirror.get(&object).is_some_and(|view| {
            let ty = view.ty();
            ty.namespace() == "gnoblin" && ty.name() == "session"
        })
    }

    /// Begin a configuration run.
    ///
    /// The handlers and timers of the previous document are dropped, because they
    /// belong to the document that declared them: keeping them would deliver one
    /// signal once per reload. Everything the connection has queued is applied
    /// first, so the configuration reads the graph as it is now rather than as it
    /// was when the host subscribed.
    fn reset_runtime(&mut self) -> Result<(), HostError> {
        {
            let mut shared = self.shared.borrow_mut();
            shared.handlers.clear();
            shared.timers.clear();
            shared.deferred.clear();
        }
        let callbacks = self.lua.create_table().map_err(HostError::Sandbox)?;
        self.lua
            .set_named_registry_value(crate::bindings::CALLBACKS, callbacks)
            .map_err(HostError::Sandbox)?;
        let mut handlers = 0usize;
        loop {
            let next = self.shared.borrow_mut().client.next_event();
            match next {
                Ok(event) => self.dispatch(event, &mut handlers),
                // Nothing queued, or the connection ended: either way the drain is
                // finished and the document is evaluated against the graph as it is.
                Err(ClientError::Disconnected | ClientError::Idle) => return Ok(()),
                Err(error) => return Err(HostError::Client(error)),
            }
        }
    }

    /// Evaluate one file inside the configuration directory.
    ///
    /// A path that resolves outside the directory [`Host::run_config`] chose is
    /// refused, the same check `gnoblin.load` applies: the root file defines the
    /// directory, and everything after it stays inside.
    pub fn evaluate(&mut self, path: &Path) -> Result<(), ScriptError> {
        let root = self
            .shared
            .borrow()
            .root
            .clone()
            .ok_or_else(|| ScriptError::from_text("no configuration directory: run the configuration first"))?;
        let canonical = fs::canonicalize(path).map_err(|source| {
            ScriptError::from_text(format!("{}: cannot resolve the file: {source}", path.display()))
        })?;
        if !canonical.starts_with(&root) {
            return Err(ScriptError::from_text(format!(
                "{}: outside the configuration directory {}",
                path.display(),
                root.display()
            )));
        }
        evaluate_file(&self.lua, &self.shared, &self.guard, &self.limits, &canonical)
    }

    /// Run every pending event, timer and deferred callback at `now`.
    ///
    /// The tick is the caller's clock: the host never reads one. Draining stops
    /// when the connection has nothing left, which on the in-process link is the
    /// same answer as [`ClientError::Disconnected`]; on a socket the caller
    /// pumps when it is readable.
    pub fn pump(&mut self, now: Tick) -> Result<PumpReport, HostError> {
        self.shared.borrow_mut().clock = now;
        let mut events = 0usize;
        let mut handlers = 0usize;
        loop {
            let next = self.shared.borrow_mut().client.next_event();
            match next {
                Ok(event) => {
                    events += 1;
                    self.dispatch(event, &mut handlers);
                }
                Err(ClientError::Disconnected | ClientError::Idle) => break,
                Err(error) => return Err(HostError::Client(error)),
            }
        }
        let mut due: Vec<(Tick, u64)> = self
            .shared
            .borrow()
            .timers
            .iter()
            .filter(|(_, due)| **due <= now)
            .map(|(id, due)| (*due, *id))
            .collect();
        due.sort();
        for (_, id) in due {
            if self.shared.borrow_mut().timers.remove(&id).is_some() {
                self.run_handler(id, None, &mut handlers);
                // A timer is one-shot, so its callback is finished with.
                self.forget_callback(id);
            }
        }
        // Only what was deferred before this pump: a callback that registers
        // another one while the pump runs waits for the next turn, so a chain
        // cannot spin inside one pump.
        let deferred: Vec<u64> = std::mem::take(&mut self.shared.borrow_mut().deferred);
        for id in deferred {
            self.run_handler(id, None, &mut handlers);
        }
        let errors = std::mem::take(&mut self.shared.borrow_mut().reports);
        Ok(PumpReport {
            events,
            handlers,
            errors,
        })
    }

    /// Report an error, unless its site has been reported already.
    fn report(&mut self, error: ScriptError) {
        let site = error.site();
        let mut shared = self.shared.borrow_mut();
        if shared.reported.insert(site) {
            shared.reports.push(error);
        }
    }

    /// Apply one event and run the handlers it wakes.
    fn dispatch(&mut self, event: Event, handlers: &mut usize) {
        match event {
            Event::Model(observation) => self.observe(observation, handlers),
            Event::Extension {
                namespace,
                member,
                fields,
            } => self.run_extension_signal(&namespace, &member, &fields, handlers),
            // The runtime offers no requests in this slice and answers none: prompts and
            // input policy are later lanes, and a request nobody answers fails closed at
            // its deadline in the broker.
            Event::RequestOffered { .. } | Event::RequestAnswered { .. } | Event::RequestFinished { .. } => {}
        }
    }

    /// Apply one observation to the mirror and then to the handlers that watch it.
    fn observe(&mut self, observation: Observation, handlers: &mut usize) {
        match observation {
            Observation::Opened { view, .. } => {
                self.shared.borrow_mut().mirror.insert(view.id(), view);
            }
            Observation::Changed {
                object,
                property,
                value,
                ..
            } => {
                let mut shared = self.shared.borrow_mut();
                if let Some(view) = shared.mirror.get_mut(&object) {
                    let mut properties = view.properties().clone();
                    properties.insert(property, value);
                    *view = ObjectView::new(view.id(), view.ty().clone(), properties);
                }
            }
            Observation::Signalled {
                object, signal, fields, ..
            } => {
                if signal == RELOAD_REQUESTED && self.is_session(object) {
                    self.shared.borrow_mut().reload_requested = true;
                }
                let declared = crate::bindings::signal_fields(&self.shared, object, &signal);
                let arguments = crate::bindings::signal_table(&self.lua, &self.shared, declared, &fields);
                match arguments {
                    Ok(arguments) => self.run_signal(object, &signal, LuaValue::Table(arguments), handlers),
                    Err(error) => self.report(ScriptError::from_lua(&error)),
                }
            }
            Observation::Closed { object, .. } => {
                // The handler still sees the object: the mirror entry is dropped
                // after it runs, so a `closed` callback can read the last state.
                self.run_signal(object, "closed", LuaValue::Nil, handlers);
                crate::bindings::forget_instance(&self.lua, object);
                let stale: Vec<u64> = {
                    let mut shared = self.shared.borrow_mut();
                    shared.mirror.remove(&object);
                    let stale: Vec<u64> = shared
                        .handlers
                        .iter()
                        .filter(|(_, watching)| {
                            matches!(watching, Watching::Signal { object: watched, .. } if *watched == object)
                        })
                        .map(|(id, _)| *id)
                        .collect();
                    for id in &stale {
                        shared.handlers.remove(id);
                    }
                    stale
                };
                // Outside the borrow: forgetting a callback touches the Lua
                // registry, which must never happen while the state is borrowed.
                for id in stale {
                    self.forget_callback(id);
                }
            }
            Observation::Finished { operation, outcome, .. } => {
                let completions = {
                    let mut shared = self.shared.borrow_mut();
                    match shared.operations.get_mut(&operation) {
                        Some(pending) if pending.outcome.is_none() => {
                            pending.outcome = Some(outcome);
                            std::mem::take(&mut pending.completions)
                        }
                        // A second completion for one operation: the graph refuses
                        // it and a peer that sent one anyway does not get a
                        // second callback.
                        _ => Vec::new(),
                    }
                };
                for id in completions {
                    self.shared.borrow_mut().defer(id);
                }
            }
        }
    }

    /// Run every handler watching one signal on one object.
    fn run_signal(&mut self, object: ObjectId, signal: &str, arguments: LuaValue, handlers: &mut usize) {
        let waiting = self.shared.borrow().signal_handlers(object, signal);
        for id in waiting {
            // A handler that ran may have unsubscribed another one.
            if self.shared.borrow().handlers.contains_key(&id) {
                self.run_handler(id, Some(arguments.clone()), handlers);
            }
        }
    }

    /// Run every handler watching one extension signal, with its fields as a table.
    fn run_extension_signal(&mut self, namespace: &str, member: &str, fields: &Fields, handlers: &mut usize) {
        // A signal the schema does not declare, or one of a secret object, is dropped.
        let declared = {
            let shared = self.shared.borrow();
            crate::extension_bindings::signal_fields(&shared.extensions, namespace, member)
        };
        let Some(declared) = declared else {
            return;
        };
        match crate::extension_bindings::signal_table(&self.lua, &declared, fields) {
            Ok(arguments) => {
                let waiting = self.shared.borrow().extension_handlers(namespace, member);
                for id in waiting {
                    // A handler that ran may have unsubscribed another one.
                    if self.shared.borrow().handlers.contains_key(&id) {
                        self.run_handler(id, Some(LuaValue::Table(arguments.clone())), handlers);
                    }
                }
            }
            Err(error) => self.report(ScriptError::from_lua(&error)),
        }
    }

    /// Drop one callback from the registry table.
    ///
    /// Nothing watches it any more, so it can never run again: forgetting it keeps
    /// a session that closes many objects, or fires many timers, from growing.
    fn forget_callback(&self, id: u64) {
        if let Ok(callbacks) = self.lua.named_registry_value::<mlua::Table>(crate::bindings::CALLBACKS) {
            // `Table::raw_remove` shifts every entry above an integer key down, so
            // it would move all the later callbacks onto other identities;
            // assigning nil removes one entry and leaves the rest where they are.
            let _ = callbacks.raw_set(id, mlua::Value::Nil);
        }
    }

    /// Call one handler, reporting a failure instead of propagating it.
    fn run_handler(&mut self, id: u64, argument: Option<LuaValue>, handlers: &mut usize) {
        let callback: Result<Option<mlua::Function>, Error> = {
            let key = crate::bindings::CALLBACKS;
            match self.lua.named_registry_value::<mlua::Table>(key) {
                Ok(callbacks) => callbacks.raw_get::<Option<mlua::Function>>(id),
                Err(error) => Err(error),
            }
        };
        let callback = match callback {
            Ok(Some(callback)) => callback,
            Ok(None) => return,
            Err(error) => {
                self.report(ScriptError::from_lua(&error));
                return;
            }
        };
        self.guard.begin(&self.limits);
        let result = match argument {
            Some(argument) => callback.call::<()>(argument),
            None => callback.call::<()>(()),
        };
        *handlers += 1;
        if let Err(error) = result {
            self.report(ScriptError::from_lua(&error));
        }
    }
}

/// Evaluate one configuration file, recording success and returning the error.
///
/// Free rather than a method because `gnoblin.load` calls it from inside a Lua
/// callback, where there is no `&mut Host`. The callbacks it needs are all
/// cloneable handles.
pub(super) fn evaluate_file<T: Transport>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    guard: &Guard,
    limits: &Limits,
    path: &Path,
) -> Result<(), ScriptError> {
    let text = fs::read_to_string(path)
        .map_err(|source| ScriptError::from_text(format!("{}: cannot read the file: {source}", path.display())))?;
    shared.borrow_mut().stack.push(path.to_path_buf());
    guard.begin(limits);
    // Text only: a binary chunk can crash the interpreter that runs it, and a
    // configuration file is never bytecode.
    let result = lua
        .load(&text)
        .set_mode(LuaChunkMode::Text)
        .set_name(format!("@{}", path.display()))
        .exec();
    shared.borrow_mut().stack.pop();
    match result {
        Ok(()) => {
            shared.borrow_mut().loaded.push(path.to_path_buf());
            Ok(())
        }
        Err(error) => Err(ScriptError::from_lua(&error)),
    }
}

/// Resolve and evaluate what `gnoblin.load` was asked for.
///
/// Every file is evaluated with its own name, so a failure names the file and
/// line that failed rather than the file that included it.
pub(super) fn load_pattern<T: Transport>(
    lua: &Lua,
    shared: &Rc<RefCell<Shared<T>>>,
    guard: &Guard,
    limits: &Limits,
    pattern: &str,
) -> Result<bool, Error> {
    let (root, current) = {
        let shared = shared.borrow();
        let root = shared
            .root
            .clone()
            .ok_or_else(|| Error::runtime("gnoblin.load is only available while a configuration is loading"))?;
        if shared.stack.len() >= MAX_LOADS {
            return Err(Error::runtime(format!(
                "gnoblin.load: configuration files nest deeper than {MAX_LOADS}"
            )));
        }
        let current = shared
            .stack
            .last()
            .and_then(|file| file.parent())
            .map_or_else(|| root.clone(), Path::to_path_buf);
        (root, current)
    };
    let resolution = loader::resolve(&root, &current, pattern)
        .map_err(|reason| Error::runtime(format!("gnoblin.load: {reason}")))?;
    let files = match resolution {
        Resolution::Missing => {
            return Err(Error::runtime(format!(
                "gnoblin.load: {pattern} needs an existing file; a glob may match nothing, an exact path may not"
            )));
        }
        Resolution::Files(files) => files,
    };
    for file in files {
        evaluate_file(lua, shared, guard, limits, &file).map_err(|error| Error::runtime(error.message().to_owned()))?;
    }
    Ok(true)
}
