//! The object graph, fed from the session's own snapshot.
//!
//! Design/rust-compositor.md, "One IPC": the compositor keeps the live object
//! graph, publishes property diffs and signals with revisions rather than whole
//! snapshots, and every IPC client reads that one graph. This module is the bridge
//! from the engine to it: [`SessionSnapshot`] in, property diffs out.
//!
//! There is one description of the window state and this module does not add a
//! second: the snapshot is what the Wayland engine publishes for every reader, and
//! the graph is a projection of it. A window the wm lost becomes `closed`, a window
//! it gained is created with all its properties at once, and nothing between two
//! snapshots can be invented.
//!
//! Because the snapshots are whole state and the diff is computed against the last
//! published one, a coalesced wake-up is correct: missing the *event* only means
//! reading the newest state, never losing a change.
//!
//! # Failure modes first
//!
//! 1. **A window object outlives its surface.** A window the snapshot no longer has
//!    is closed once, its identity is never reused and its pending operations finish
//!    with `object-gone`: `a_window_that_disappears_closes_its_object`.
//! 2. **A diff is published for a value that did not change.** Only real changes
//!    become observations, so a subscriber is not woken by a snapshot that repeated
//!    itself: `an_unchanged_snapshot_publishes_nothing`.
//! 3. **A window opens without all of its properties.** Creation is atomic and takes
//!    every declared property, so a subscriber never sees a half-built window, and a
//!    schema that gained a required property fails here rather than silently
//!    publishing nothing: `a_new_window_carries_every_declared_property`.
//! 4. **A publication writes a property the schema does not declare.** The graph refuses
//!    it, so a slot the plan gained without a matching schema property fails here rather
//!    than publishing nothing: see [`SessionSync::publish_session`].
//! 5. **The session object is created without the version it serves.** The version
//!    is the schema's namespace version, which is the IPC version, and a graph that
//!    refused it is a `ModelError` rather than a session that claims nothing:
//!    `the_session_object_names_the_version_this_process_serves`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gnoblin_model::{Fields, Filter, Graph, ModelError, ObjectId, Observation, Subscription, TypeKey, Value};
use gnoblin_schema::Catalog;
use gnoblin_types::{OutputId, Rect, Version, WindowId, WorkspaceId};
use gnoblin_wayland::{EffectSpec, ObservedOutput, ObservedWindow, ObservedWorkspace, SessionSnapshot};

use crate::log;

/// The property and member names this module writes, as the schema spells them.
///
/// They are constants rather than literals because a rename in the schema must be a
/// compile-time change here, not a `set` that fails on every window.
mod member {
    /// The `session` object's version property.
    pub const VERSION: &str = "version";
    /// The `session` object's ready signal.
    pub const READY: &str = "ready";
    /// The `session` object's bell signal.
    pub const BELL: &str = "bell";
    /// The bell signal's field: how many times the bell has rung.
    pub const COUNT: &str = "count";
    /// The `session` object's reload_requested signal.
    pub const RELOAD_REQUESTED: &str = "reload_requested";
    /// The reload signal's field: which request this is, counted by the compositor.
    pub const GENERATION: &str = "generation";
    /// The `window` object's menu_requested signal, and its fields.
    pub const MENU_REQUESTED: &str = "menu_requested";
    /// The menu signal's source field: client, keyboard or extension.
    pub const SOURCE: &str = "source";
    /// The menu signal's seat field.
    pub const SEAT: &str = "seat";
    /// The `window` object's identity.
    pub const ID: &str = "id";
    /// The `window` object's title property.
    pub const TITLE: &str = "title";
    /// The `window` object's app_id property.
    pub const APP_ID: &str = "app_id";
    /// The `window` object's focused property.
    pub const FOCUSED: &str = "focused";
    /// The `window` object's x property.
    pub const X: &str = "x";
    /// The `window` object's y property.
    pub const Y: &str = "y";
    /// The `window` object's width property.
    pub const WIDTH: &str = "width";
    /// The `window` object's height property.
    pub const HEIGHT: &str = "height";
    /// The `window` object's icon property.
    pub const ICON: &str = "icon";
    /// The `window` object's tag property.
    pub const TAG: &str = "tag";
    /// The `window` object's modal property.
    pub const MODAL: &str = "modal";
    /// The `window` object's unresponsive property.
    pub const UNRESPONSIVE: &str = "unresponsive";
    /// The `window` object's effects property: its `effect` objects, first to last.
    pub const EFFECTS: &str = "effects";
    /// The layout name of an `output` or `workspace` object.
    pub const LAYOUT: &str = "layout";
    /// The name of an `output` object, as the compositor reports it.
    pub const NAME: &str = "name";
    /// The reason field of a `layout_failed` signal.
    pub const REASON: &str = "reason";
    /// The `layout_changed` signal, sent when an object's layout changes.
    pub const LAYOUT_CHANGED: &str = "layout_changed";
    /// The `layout_failed` signal, sent when a layout change is refused.
    pub const LAYOUT_FAILED: &str = "layout_failed";
    /// The `lock` object's locked property: whether the lock is holding the desktop.
    pub const LOCKED: &str = "locked";
    /// The `idle` object's idle_ms property: milliseconds since the last input event.
    pub const IDLE_MS: &str = "idle_ms";
    /// The `idle` object's inhibitors property: how many holds keep the session awake.
    pub const INHIBITORS: &str = "inhibitors";
}

/// What the graph last published for one workspace.
#[derive(Clone, Debug, PartialEq)]
struct PublishedWorkspace {
    /// The `workspace` object's identity.
    object: ObjectId,
    /// The layout name last published for this workspace.
    layout: String,
    /// Why the workspace's layout failed to apply, if it did, so a client can show the reason.
    failure: Option<&'static str>,
}

/// What the graph last published for one output.
#[derive(Clone, Debug, PartialEq)]
struct PublishedOutput {
    /// The `output` object's identity.
    object: ObjectId,
    /// Connector name the backend reports for this output.
    name: String,
    /// The property values last published for this output, keyed by schema name.
    fields: Fields,
}

/// What the graph last published for one window.
#[derive(Clone, Debug, PartialEq)]
struct PublishedWindow {
    /// The `window` object's identity.
    object: ObjectId,
    /// Title last published for this window.
    title: String,
    /// App id last published for this window.
    app_id: String,
    /// Whether the window held focus in the last published snapshot.
    focused: bool,
    /// Icon name last published for this window.
    icon: String,
    /// Tag last published for this window.
    tag: String,
    /// Whether the window was modal in the last published snapshot.
    modal: bool,
    /// Whether the window was unresponsive in the last published snapshot.
    unresponsive: bool,
    /// Geometry last published for this window, in the compositor's logical coordinates.
    geometry: Rect,
    /// Effect objects of this window, each with the parameters last published for it.
    effects: Vec<PublishedEffect>,
}

/// What the graph last published for one effect of a window.
#[derive(Clone, Debug, PartialEq)]
struct PublishedEffect {
    /// The `effect` object.
    object: ObjectId,
    /// The parameters the object was last brought to.
    spec: EffectSpec,
}

/// The live object graph of one compositor process.
#[derive(Debug)]
pub struct SessionSync {
    /// The graph the snapshot is applied to. Every write goes through it, so every write
    /// yields a diff that subscribers see.
    graph: Graph,
    /// The `session` object, which carries the slot properties and the schema version.
    session: ObjectId,
    /// The `lock` object, whose `locked` property `apply` writes from the snapshot.
    lock: ObjectId,
    /// The `idle` object, whose idle_ms and inhibitors properties `apply` writes from the snapshot.
    idle: ObjectId,
    /// The `settings` object, which `publish_settings` writes from the configuration.
    settings: ObjectId,
    /// How many bells the last snapshot counted, so `apply` raises the signal once per
    /// ring rather than once per snapshot.
    bells: u64,
    /// The sequence of the last menu request emitted, so each request is raised once.
    menu_seq: u64,
    /// Last published state per window, kept so each new snapshot is diffed against it and
    /// unchanged properties are not rewritten.
    windows: BTreeMap<WindowId, PublishedWindow>,
    /// Last published state per workspace, diffed the same way as `windows`.
    workspaces: BTreeMap<WorkspaceId, PublishedWorkspace>,
    /// Last published state per output, keyed by the engine's output id.
    outputs: BTreeMap<OutputId, PublishedOutput>,
}

impl SessionSync {
    /// Builds the graph and its `session` object.
    ///
    /// # Errors
    ///
    /// [`ModelError`] when the schema does not declare the `session` object or the
    /// version property, which would mean the embedded catalog and this module
    /// disagree.
    pub fn new(catalog: Catalog, version: Version) -> Result<SessionSync, ModelError> {
        let mut graph = Graph::new(catalog);
        // The schema declares one property per slot, so the object cannot be created without
        // them: the plan's default providers are what a session publishes before anyone claims
        // anything, which is also what makes "nobody provides this slot yet" readable.
        let mut properties = Fields::from([(member::VERSION.to_owned(), Value::Text(version.to_string()))]);
        properties.extend(gnoblin_model::Slots::new().fields());
        let session = graph.create(TypeKey::gnoblin("session"), properties)?;
        let lock_properties = Fields::from([(member::LOCKED.to_owned(), Value::Bool(false))]);
        let lock = graph.create(TypeKey::gnoblin("lock"), lock_properties)?;
        let idle_properties = Fields::from([
            (member::IDLE_MS.to_owned(), Value::Uint(0)),
            (member::INHIBITORS.to_owned(), Value::Uint(0)),
        ]);
        let idle = graph.create(TypeKey::gnoblin("idle"), idle_properties)?;
        let settings = graph.create(TypeKey::gnoblin("settings"), super::settings::defaults())?;
        // The `layers` object carries no properties either: its one verb registers a layer
        // under the caller's own provider, which the compositor derives from the connection.
        graph.create(TypeKey::gnoblin("layers"), Fields::new())?;
        Ok(SessionSync {
            graph,
            session,
            lock,
            idle,
            settings,
            bells: 0,
            menu_seq: 0,
            windows: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            outputs: BTreeMap::new(),
        })
    }

    /// How many bells this sync has raised.
    #[must_use]
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// Raises `session.reload_requested` for the runtime to act on.
    ///
    /// The runtime owns the document, so this only says that a reload was asked for. A
    /// failed raise is logged rather than returned: the request then waits for an answer
    /// that never comes, and the caller sees that through its own deadline.
    pub fn request_reload(&mut self, generation: u64) {
        let mut fields = Fields::new();
        fields.insert(member::GENERATION.to_owned(), Value::Uint(generation));
        if let Err(error) = self.graph.signal(self.session, member::RELOAD_REQUESTED, fields) {
            log::error!(area = "ipc", %error, "the session object declares no reload_requested signal");
        }
    }

    /// Raises `session.ready`, once, when the IPC server is about to accept.
    ///
    /// A subscriber that connects afterwards starts from the snapshot instead, which
    /// is why this is a signal and not a property: "the session can accept requests"
    /// is an event, and the state it leaves behind is the version.
    pub fn ready(&mut self) {
        if let Err(error) = self.graph.signal(self.session, member::READY, Fields::new()) {
            log::error!(area = "ipc", %error, "the session object declares no ready signal");
        }
    }
    /// Writes the `settings` object from the configuration's `appearance` and `cursor`
    /// tables. A property the configuration does not name takes its stock value, so a
    /// setting removed from the configuration reverts rather than sticking.
    pub fn publish_settings(&mut self, config: &gnoblin_config::Config) {
        let mut fields = super::settings::defaults();
        fields.extend(super::settings::from_config(config));
        for (property, value) in fields {
            self.set_if_changed(self.settings, &property, value);
        }
    }

    /// Writes the session properties `fields` names.
    ///
    /// The `session` object is the one object this process creates, so its properties are
    /// written here rather than by a snapshot: the slot registry changes when a
    /// connection claims a slot, which no snapshot knows about. A property the schema
    /// does not declare is refused by the graph, so a publication cannot invent one.
    ///
    /// # Errors
    ///
    /// [`ModelError`] when the schema does not declare a property or the value's type
    /// is wrong, which means a publication and the schema disagree.
    pub fn publish_session(&mut self, fields: &Fields) -> Result<(), ModelError> {
        for (property, value) in fields {
            self.graph.set(self.session, property, value.clone())?;
        }
        Ok(())
    }

    /// Raises `window.menu_requested` once for every menu request the snapshot carries
    /// that this session has not yet emitted, in the order the loop made them.
    ///
    /// The cursor is the request's sequence number, so a request is emitted once even
    /// though every snapshot carries the recent ones again. A request whose window is no
    /// longer published is skipped rather than guessed at: its menu has nowhere to appear.
    fn raise_menu_requests(&mut self, snapshot: &SessionSnapshot) {
        for request in snapshot.menu_requests() {
            if request.seq() <= self.menu_seq {
                continue;
            }
            self.menu_seq = request.seq();
            let Some(published) = self.windows.get(&request.window()) else {
                continue;
            };
            let anchor = request.anchor();
            let mut fields = Fields::new();
            fields.insert(member::X.to_owned(), Value::Int(i64::from(anchor.x())));
            fields.insert(member::Y.to_owned(), Value::Int(i64::from(anchor.y())));
            fields.insert(member::WIDTH.to_owned(), Value::Uint(u64::from(anchor.width())));
            fields.insert(member::HEIGHT.to_owned(), Value::Uint(u64::from(anchor.height())));
            fields.insert(
                member::SOURCE.to_owned(),
                Value::Text(request.source().as_str().to_owned()),
            );
            fields.insert(member::SEAT.to_owned(), Value::Text(request.seat().to_owned()));
            if let Err(error) = self.graph.signal(published.object, member::MENU_REQUESTED, fields) {
                log::error!(area = "ipc", %error, "the window object declares no menu_requested signal");
            }
        }
    }

    /// Raises `session.bell` once for every ring the snapshot counted since the last one.
    ///
    /// The snapshot carries a count rather than a queue: a reader that missed one still
    /// learns that it happened, and a compositor that served ten bells between two
    /// snapshots raises the signal for each of them, so a subscriber's count always
    /// reaches the snapshot's.
    fn raise_bell(&mut self, bells: u64) {
        while self.bells < bells {
            self.bells = self.bells.saturating_add(1);
            let mut fields = Fields::new();
            fields.insert(member::COUNT.to_owned(), Value::Uint(self.bells));
            if let Err(error) = self.graph.signal(self.session, member::BELL, fields) {
                log::error!(area = "ipc", %error, "the session object declares no bell signal");
                return;
            }
        }
    }

    /// Applies a snapshot: creates, changes and closes objects to match it.
    ///
    /// # Errors
    ///
    /// [`ModelError`] when the schema refuses a write this module makes. Nothing is
    /// half-applied: the graph validates before it publishes, so a refused property
    /// Applies a snapshot: creates, changes and closes objects to match it.
    ///
    /// Every object is published independently. A refused write is recorded against
    /// the object's kind and id and the rest of the snapshot still publishes, so one
    /// bad object cannot hide the others. The error lists every failure.
    ///
    /// # Errors
    ///
    /// [`PublishError`] when the schema refuses a write this module makes, naming each
    /// object that could not be published.
    pub fn apply(&mut self, snapshot: &SessionSnapshot) -> Result<(), PublishError> {
        self.raise_menu_requests(snapshot);
        self.raise_bell(snapshot.bells());
        let mut failures = Vec::new();
        self.apply_windows(snapshot, &mut failures);
        self.apply_outputs(snapshot, &mut failures);
        self.apply_workspaces(snapshot, &mut failures);
        self.publish_lock(snapshot.lock_active());
        self.publish_idle(snapshot.idle_ms(), snapshot.inhibitors());
        if failures.is_empty() {
            Ok(())
        } else {
            Err(PublishError { failures })
        }
    }

    /// Writes the `lock` object's `locked` property from the snapshot's lock state.
    ///
    /// This is `ScreenSaver.GetActive`, true from the moment a lock is asked for. It is not
    /// the confirmation a `lock` call waits for: that is the operation's own outcome.
    fn publish_lock(&mut self, locked: bool) {
        self.set_if_changed(self.lock, member::LOCKED, Value::Bool(locked));
    }

    /// Writes one property only when its value differs, so a repeat snapshot publishes nothing.
    fn set_if_changed(&mut self, object: ObjectId, property: &str, value: Value) {
        if self.graph.object(object).and_then(|live| live.property(property)) == Some(&value) {
            return;
        }
        if let Err(error) = self.graph.set(object, property, value) {
            log::error!(area = "ipc", %error, property, "an object declares no such property");
        }
    }

    /// Writes the `idle` object's idle_ms and inhibitors properties from the snapshot.
    ///
    /// These are what `IdleMonitor.GetIdletime` and the ScreenSaver inhibit count read, so a
    /// reader that finds no `idle` object fails closed rather than reporting zero idle time.
    fn publish_idle(&mut self, idle_ms: u64, inhibitors: usize) {
        let inhibitors = u64::try_from(inhibitors).unwrap_or(u64::MAX);
        let fields = Fields::from([
            (member::IDLE_MS.to_owned(), Value::Uint(idle_ms)),
            (member::INHIBITORS.to_owned(), Value::Uint(inhibitors)),
        ]);
        for (property, value) in fields {
            self.set_if_changed(self.idle, &property, value);
        }
    }

    /// Creates, changes and closes window objects to match the snapshot.
    fn apply_windows(&mut self, snapshot: &SessionSnapshot, failures: &mut Vec<PublishFailure>) {
        let focus = snapshot.focus();
        let mut seen = BTreeSet::new();
        for window in snapshot.windows() {
            seen.insert(window.id);
            if let Err(error) = self.publish_window(window, focus) {
                failures.push(PublishFailure::new("window", window.id.get(), error));
            }
        }
        let lost: Vec<WindowId> = self
            .windows
            .keys()
            .filter(|window| !seen.contains(window))
            .copied()
            .collect();
        for window in lost {
            if let Some(published) = self.windows.remove(&window) {
                for effect in &published.effects {
                    if let Err(error) = self.graph.close(effect.object) {
                        failures.push(PublishFailure::new("effect", window.get(), error));
                    }
                }
                // The object's own end is the one event. The schema also declares a
                // `closed` signal, and it is deliberately not raised as well: a
                // subscriber would learn the same thing twice.
                if let Err(error) = self.graph.close(published.object) {
                    failures.push(PublishFailure::new("window", window.get(), error));
                }
            }
        }
    }

    /// Creates or updates one window object.
    fn publish_window(&mut self, window: &ObservedWindow, focus: Option<WindowId>) -> Result<(), ModelError> {
        let focused = focus == Some(window.id);
        let icon = window.icon_name.clone().unwrap_or_default();
        let tag = window.tag.clone().unwrap_or_default();
        let Some(previous) = self.windows.get(&window.id).cloned() else {
            let properties = window_properties(window.id, window);
            let object = self.graph.create(TypeKey::gnoblin("window"), properties)?;
            let effects = self.publish_effects(object, Vec::new(), &window.effects)?;
            self.windows.insert(
                window.id,
                PublishedWindow {
                    object,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    focused,
                    icon,
                    tag,
                    modal: window.modal,
                    unresponsive: window.unresponsive,
                    geometry: window.geometry,
                    effects,
                },
            );
            return Ok(());
        };
        let object = previous.object;
        if previous.title != window.title {
            self.graph
                .set(object, member::TITLE, Value::Text(window.title.clone()))?;
        }
        if previous.app_id != window.app_id {
            self.graph
                .set(object, member::APP_ID, Value::Text(window.app_id.clone()))?;
        }
        if previous.focused != focused {
            self.graph.set(object, member::FOCUSED, Value::Bool(focused))?;
        }
        if previous.icon != icon {
            self.graph.set(object, member::ICON, Value::Text(icon.clone()))?;
        }
        if previous.tag != tag {
            self.graph.set(object, member::TAG, Value::Text(tag.clone()))?;
        }
        if previous.modal != window.modal {
            self.graph.set(object, member::MODAL, Value::Bool(window.modal))?;
        }
        if previous.unresponsive != window.unresponsive {
            self.graph
                .set(object, member::UNRESPONSIVE, Value::Bool(window.unresponsive))?;
        }
        if previous.geometry != window.geometry {
            self.set_geometry(object, window.geometry)?;
        }
        let effects = self.publish_effects(object, previous.effects, &window.effects)?;
        self.windows.insert(
            window.id,
            PublishedWindow {
                object,
                title: window.title.clone(),
                app_id: window.app_id.clone(),
                focused,
                icon,
                tag,
                modal: window.modal,
                unresponsive: window.unresponsive,
                geometry: window.geometry,
                effects,
            },
        );
        Ok(())
    }

    /// Brings one window's `effect` objects in line with its effect stack.
    ///
    /// An effect keeps its object while its kind stays the same, so a subscriber sees
    /// changed properties rather than a close and a create. A kind change, an added
    /// effect or a removed one is a new or ended object. The window's `effects` list is
    /// written only when its sequence of objects changed.
    fn publish_effects(
        &mut self,
        window: ObjectId,
        previous: Vec<PublishedEffect>,
        next: &[EffectSpec],
    ) -> Result<Vec<PublishedEffect>, ModelError> {
        let before: Vec<ObjectId> = previous.iter().map(|effect| effect.object).collect();
        let mut kept = previous.into_iter();
        let mut published = Vec::with_capacity(next.len());
        for spec in next {
            match kept.next() {
                Some(old) if old.spec.kind == spec.kind => {
                    // Only the parameters that moved are written; an unchanged one needs no revision.
                    let old_fields = effect_fields(&old.spec);
                    for (name, value) in effect_fields(spec) {
                        if old_fields.get(&name) != Some(&value) {
                            self.graph.set(old.object, &name, value)?;
                        }
                    }
                    published.push(PublishedEffect {
                        object: old.object,
                        spec: spec.clone(),
                    });
                }
                replaced => {
                    if let Some(old) = replaced {
                        self.graph.close(old.object)?;
                    }
                    let object = self.graph.create(TypeKey::gnoblin("effect"), effect_fields(spec))?;
                    published.push(PublishedEffect {
                        object,
                        spec: spec.clone(),
                    });
                }
            }
        }
        for old in kept {
            self.graph.close(old.object)?;
        }
        let after: Vec<ObjectId> = published.iter().map(|effect| effect.object).collect();
        if after != before {
            let list = after.into_iter().map(Value::Object).collect();
            self.graph.set(window, member::EFFECTS, Value::List(list))?;
        }
        Ok(published)
    }

    /// Creates, changes and closes `output` objects to match the snapshot.
    fn apply_outputs(&mut self, snapshot: &SessionSnapshot, failures: &mut Vec<PublishFailure>) {
        let mut seen = BTreeSet::new();
        for output in snapshot.output_layouts() {
            seen.insert(output.id);
            if let Err(error) = self.publish_output(output) {
                failures.push(PublishFailure::new("output", output.id.get(), error));
            }
        }
        let lost: Vec<OutputId> = self.outputs.keys().filter(|id| !seen.contains(id)).copied().collect();
        for id in lost {
            let Some(published) = self.outputs.remove(&id) else {
                continue;
            };
            if let Err(error) = self.graph.close(published.object) {
                failures.push(PublishFailure::new("output", id.get(), error));
            }
        }
    }

    /// Creates or updates one output object.
    ///
    /// A create publishes every property the schema declares, so the graph accepts the
    /// object; an update publishes only the properties whose values moved.
    fn publish_output(&mut self, output: &ObservedOutput) -> Result<(), ModelError> {
        let fields = Self::output_fields(output);
        let Some(previous) = self.outputs.get(&output.id).cloned() else {
            let object = self.graph.create(TypeKey::gnoblin("output"), fields.clone())?;
            self.outputs.insert(
                output.id,
                PublishedOutput {
                    object,
                    name: output.name.clone(),
                    fields,
                },
            );
            return Ok(());
        };
        for (name, value) in &fields {
            if previous.fields.get(name) != Some(value) {
                self.graph.set(previous.object, name, value.clone())?;
            }
        }
        self.outputs.insert(
            output.id,
            PublishedOutput {
                name: output.name.clone(),
                fields,
                ..previous
            },
        );
        Ok(())
    }

    /// The properties the `output` schema declares, from what the backend reports.
    ///
    /// A value the backend does not report arrives as the schema's empty default, so
    /// nothing here is invented hardware data.
    fn output_fields(output: &ObservedOutput) -> Fields {
        Fields::from([
            (member::NAME.to_owned(), Value::Text(output.name.clone())),
            ("make".to_owned(), Value::Text(output.make.clone())),
            ("model".to_owned(), Value::Text(output.model.clone())),
            (
                "modes".to_owned(),
                Value::List(output.modes.iter().cloned().map(Value::Text).collect()),
            ),
            ("mode".to_owned(), Value::Text(output.mode.clone())),
            ("scale".to_owned(), Value::Double(output.scale.as_f64())),
            ("transform".to_owned(), Value::Uint(u64::from(output.transform))),
            ("primary".to_owned(), Value::Bool(output.primary)),
            ("enabled".to_owned(), Value::Bool(output.enabled)),
            ("x".to_owned(), Value::Int(i64::from(output.rect.x))),
            ("y".to_owned(), Value::Int(i64::from(output.rect.y))),
            ("width".to_owned(), Value::Uint(u64::from(output.rect.width))),
            ("height".to_owned(), Value::Uint(u64::from(output.rect.height))),
            ("layout".to_owned(), Value::Text(output.layout.clone())),
        ])
    }

    /// Creates, changes and closes `workspace` objects to match the snapshot, and raises
    /// `layout_changed` and `layout_failed` where the layout or its failure changed.
    ///
    /// A failure is raised once when it first appears; a workspace that stops failing
    /// (its layout was replaced) simply publishes its layout again.
    fn apply_workspaces(&mut self, snapshot: &SessionSnapshot, failures: &mut Vec<PublishFailure>) {
        let layout_failures: BTreeMap<WorkspaceId, &'static str> = snapshot.layout_failures().iter().copied().collect();
        let mut seen = BTreeSet::new();
        for workspace in snapshot.workspaces() {
            seen.insert(workspace.id);
            let failure = layout_failures.get(&workspace.id).copied();
            if let Err(error) = self.publish_workspace(workspace, failure) {
                failures.push(PublishFailure::new("workspace", workspace.id.get(), error));
            }
        }
        let lost: Vec<WorkspaceId> = self
            .workspaces
            .keys()
            .filter(|id| !seen.contains(id))
            .copied()
            .collect();
        for id in lost {
            let Some(published) = self.workspaces.remove(&id) else {
                continue;
            };
            if let Err(error) = self.graph.close(published.object) {
                failures.push(PublishFailure::new("workspace", id.get(), error));
            }
        }
    }

    /// Creates or updates one workspace object and raises its layout signals.
    fn publish_workspace(
        &mut self,
        workspace: &ObservedWorkspace,
        failure: Option<&'static str>,
    ) -> Result<(), ModelError> {
        let Some(previous) = self.workspaces.get(&workspace.id).cloned() else {
            let properties = Fields::from([
                (member::ID.to_owned(), Value::Uint(workspace.id.get())),
                (member::LAYOUT.to_owned(), Value::Text(workspace.layout.clone())),
            ]);
            let object = self.graph.create(TypeKey::gnoblin("workspace"), properties)?;
            if let Some(reason) = failure {
                self.raise_failed(object, &workspace.layout, reason)?;
            }
            self.workspaces.insert(
                workspace.id,
                PublishedWorkspace {
                    object,
                    layout: workspace.layout.clone(),
                    failure,
                },
            );
            return Ok(());
        };
        let object = previous.object;
        if previous.layout != workspace.layout {
            self.graph
                .set(object, member::LAYOUT, Value::Text(workspace.layout.clone()))?;
            self.raise_changed(object, &workspace.layout)?;
        }
        if let Some(reason) = failure.filter(|reason| previous.failure != Some(*reason)) {
            self.raise_failed(object, &workspace.layout, reason)?;
        }
        self.workspaces.insert(
            workspace.id,
            PublishedWorkspace {
                object,
                layout: workspace.layout.clone(),
                failure,
            },
        );
        Ok(())
    }
    /// Raises `layout_changed` on a workspace object.
    fn raise_changed(&mut self, object: ObjectId, layout: &str) -> Result<(), ModelError> {
        let fields = Fields::from([(member::LAYOUT.to_owned(), Value::Text(layout.to_owned()))]);
        self.graph.signal(object, member::LAYOUT_CHANGED, fields).map(|_| ())
    }

    /// Raises `layout_failed` on a workspace object, with the reason the run failed.
    fn raise_failed(&mut self, object: ObjectId, layout: &str, reason: &str) -> Result<(), ModelError> {
        let fields = Fields::from([
            (member::LAYOUT.to_owned(), Value::Text(layout.to_owned())),
            (member::REASON.to_owned(), Value::Text(reason.to_owned())),
        ]);
        self.graph.signal(object, member::LAYOUT_FAILED, fields).map(|_| ())
    }

    /// The graph, for a caller that reads state.
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The graph, for a caller that subscribes, calls or completes.
    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// The `session` object's identity.
    #[must_use]
    pub fn session(&self) -> ObjectId {
        self.session
    }

    /// The workspace an object stands for, if it is one this graph publishes.
    #[must_use]
    pub fn workspace_of(&self, object: ObjectId) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
            .find(|(_, published)| published.object == object)
            .map(|(workspace, _)| *workspace)
    }

    /// The output an object stands for, if it is one this graph publishes.
    #[must_use]
    pub fn output_of(&self, object: ObjectId) -> Option<(OutputId, String)> {
        self.outputs
            .iter()
            .find(|(_, published)| published.object == object)
            .map(|(output, published)| (*output, published.name.clone()))
    }

    /// The window an object stands for, if it is one this graph publishes.
    ///
    /// It is how a method call finds the window it is about: the wire carries an
    /// object identity, and the window manager speaks window identities.
    #[must_use]
    pub fn window_of(&self, object: ObjectId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, published)| published.object == object)
            .map(|(window, _)| *window)
    }

    /// Every window this graph publishes, in window identity order.
    pub fn windows(&self) -> impl Iterator<Item = (WindowId, ObjectId)> + '_ {
        self.windows
            .iter()
            .map(|(window, published)| (*window, published.object))
    }

    /// Subscribes at the graph's current revision.
    pub fn subscribe(&mut self, filter: Filter) -> Subscription {
        self.graph.subscribe(filter)
    }

    /// The next observation for a subscription, in revision order.
    pub fn next(&mut self, subscription: &Subscription) -> Option<Observation> {
        self.graph.next(subscription)
    }

    /// Drops a subscription's queue.
    pub fn unsubscribe(&mut self, subscription: gnoblin_types::SubscriptionId) {
        let _ = self.graph.unsubscribe(subscription);
    }

    /// Publishes one frame rectangle as four property diffs.
    fn set_geometry(&mut self, object: ObjectId, geometry: Rect) -> Result<(), ModelError> {
        // The snapshot can carry a negative extent only for an empty rectangle; the
        // schema's extents are unsigned, so an empty one is published as zero.
        let width = u64::from(geometry.width);
        let height = u64::from(geometry.height);
        self.graph.set(object, member::X, Value::Int(i64::from(geometry.x)))?;
        self.graph.set(object, member::Y, Value::Int(i64::from(geometry.y)))?;
        self.graph.set(object, member::WIDTH, Value::Uint(width))?;
        self.graph.set(object, member::HEIGHT, Value::Uint(height))?;
        Ok(())
    }
}

/// The schema properties of one `effect` object, keyed by their schema names.
///
/// Every parameter is written, not only the ones the kind uses: the schema defaults
/// apply to the rest, and a parameter that is ignored for this kind is harmless.
fn effect_fields(spec: &EffectSpec) -> Fields {
    Fields::from([
        ("kind".to_owned(), Value::Text(spec.kind.clone())),
        ("enabled".to_owned(), Value::Bool(spec.enabled)),
        ("active".to_owned(), Value::Bool(spec.active)),
        ("reason".to_owned(), Value::Text(spec.reason.clone())),
        ("downscale".to_owned(), Value::Uint(u64::from(spec.downscale))),
        ("passes".to_owned(), Value::Uint(u64::from(spec.passes))),
        ("algorithm".to_owned(), Value::Text(spec.algorithm.clone())),
        ("noise".to_owned(), Value::Double(spec.noise)),
        ("brightness".to_owned(), Value::Double(spec.brightness)),
        ("contrast".to_owned(), Value::Double(spec.contrast)),
        ("saturation".to_owned(), Value::Double(spec.saturation)),
        ("tint".to_owned(), Value::Text(spec.tint.clone())),
        ("offset_x".to_owned(), Value::Double(spec.offset_x)),
        ("offset_y".to_owned(), Value::Double(spec.offset_y)),
        ("blur_radius".to_owned(), Value::Double(spec.blur_radius)),
        ("spread".to_owned(), Value::Double(spec.spread)),
        ("colour".to_owned(), Value::Text(spec.colour.clone())),
        ("source".to_owned(), Value::Text(spec.source.clone())),
        ("entry".to_owned(), Value::Text(spec.entry.clone())),
    ])
}

/// Every property a new `window` object is created with.
fn window_properties(window: WindowId, observed: &gnoblin_wayland::ObservedWindow) -> Fields {
    // `mapped` and `visible` are the session's own vocabulary and the schema declares
    // neither: a shell decides what to show from focus and geometry, and the plan's
    // window slice grows a property only when a shell needs it. `visible` is also
    // workspace policy rather than a property of the window.
    Fields::from([
        (member::ID.to_owned(), Value::Uint(window.get())),
        (member::TITLE.to_owned(), Value::Text(observed.title.clone())),
        (member::APP_ID.to_owned(), Value::Text(observed.app_id.clone())),
        (member::FOCUSED.to_owned(), Value::Bool(observed.focused)),
        (member::X.to_owned(), Value::Int(i64::from(observed.geometry.x))),
        (member::Y.to_owned(), Value::Int(i64::from(observed.geometry.y))),
        (
            member::WIDTH.to_owned(),
            Value::Uint(u64::from(observed.geometry.width)),
        ),
        (
            member::HEIGHT.to_owned(),
            Value::Uint(u64::from(observed.geometry.height)),
        ),
        (
            member::ICON.to_owned(),
            Value::Text(observed.icon_name.clone().unwrap_or_default()),
        ),
        (
            member::TAG.to_owned(),
            Value::Text(observed.tag.clone().unwrap_or_default()),
        ),
        (member::MODAL.to_owned(), Value::Bool(observed.modal)),
        // The schema requires `effects` on every window, so a window with no effects is
        // created with an empty list rather than without the property.
        (member::EFFECTS.to_owned(), Value::List(Vec::new())),
        (member::UNRESPONSIVE.to_owned(), Value::Bool(observed.unresponsive)),
    ])
}

#[cfg(test)]
mod tests {
    use gnoblin_model::{Filter, Observation, Subscription, Value};
    use gnoblin_types::{OutputId, Rect, Scale, Version, WindowId};
    use gnoblin_wayland::{ObservedOutput, ObservedWindow, SessionSnapshot};

    use super::SessionSync;

    fn version() -> Version {
        Version::new(0, 2, 0)
    }

    fn store() -> SessionSync {
        SessionSync::new(crate::schema::catalog().expect("the schema"), version()).expect("the session object")
    }

    fn window(id: u64, title: &str, focused: bool, geometry: Rect) -> ObservedWindow {
        ObservedWindow {
            id: WindowId::new(id),
            title: title.to_owned(),
            app_id: "org.gnoblin.Test".to_owned(),
            mapped: true,
            visible: true,
            focused,
            geometry,
            icon_name: None,
            icon_buffer_count: 0,
            tag: None,
            modal: false,
            unresponsive: false,
            transient_for: None,
            effects: Vec::new(),
        }
    }

    fn snapshot(windows: Vec<ObservedWindow>, focus: Option<WindowId>) -> SessionSnapshot {
        SessionSnapshot::from_parts(Vec::new(), focus, windows)
    }

    /// Every observation a subscription has queued, drained.
    fn drained(store: &mut SessionSync, subscription: &Subscription) -> Vec<Observation> {
        let mut observations = Vec::new();
        while let Some(observation) = store.next(subscription) {
            observations.push(observation);
        }
        observations
    }

    fn opened<'a>(observations: &'a [Observation], ty: &str) -> &'a gnoblin_model::ObjectView {
        observations
            .iter()
            .find_map(|observation| match observation {
                Observation::Opened { view, .. } if view.ty().to_string() == ty => Some(view),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no `{ty}` opened in {observations:#?}"))
    }

    fn output(id: u64, name: &str, rect: Rect) -> ObservedOutput {
        ObservedOutput {
            id: OutputId::new(id),
            name: name.to_owned(),
            layout: "floating".to_owned(),
            make: String::new(),
            model: String::new(),
            modes: vec!["1920x1080@60.000".to_owned()],
            mode: "1920x1080@60.000".to_owned(),
            scale: Scale::integral(1),
            transform: 0,
            primary: true,
            enabled: true,
            rect,
        }
    }

    #[test]
    fn an_output_carries_every_declared_property() {
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        let snapshot =
            snapshot(Vec::new(), None).with_output_layouts(vec![output(1, "DP-1", Rect::new(0, 0, 1920, 1080))]);
        store.apply(&snapshot).expect("the graph accepts the output");
        let observations = drained(&mut store, &subscription);
        let view = opened(&observations, "gnoblin.output");
        assert_eq!(view.property("name"), Some(&Value::Text("DP-1".to_owned())));
        assert_eq!(view.property("make"), Some(&Value::Text(String::new())));
        assert_eq!(view.property("model"), Some(&Value::Text(String::new())));
        assert_eq!(
            view.property("modes"),
            Some(&Value::List(vec![Value::Text("1920x1080@60.000".to_owned())]))
        );
        assert_eq!(view.property("mode"), Some(&Value::Text("1920x1080@60.000".to_owned())));
        assert_eq!(view.property("scale"), Some(&Value::Double(1.0)));
        assert_eq!(view.property("transform"), Some(&Value::Uint(0)));
        assert_eq!(view.property("primary"), Some(&Value::Bool(true)));
        assert_eq!(view.property("enabled"), Some(&Value::Bool(true)));
        assert_eq!(view.property("x"), Some(&Value::Int(0)));
        assert_eq!(view.property("y"), Some(&Value::Int(0)));
        assert_eq!(view.property("width"), Some(&Value::Uint(1920)));
        assert_eq!(view.property("height"), Some(&Value::Uint(1080)));
        assert_eq!(view.property("layout"), Some(&Value::Text("floating".to_owned())));
    }

    #[test]
    fn a_changed_output_publishes_only_what_changed() {
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        store
            .apply(&snapshot(Vec::new(), None).with_output_layouts(vec![output(
                1,
                "DP-1",
                Rect::new(0, 0, 1920, 1080),
            )]))
            .expect("applied");
        drained(&mut store, &subscription);
        let moved = ObservedOutput {
            rect: Rect::new(1920, 0, 1920, 1080),
            primary: false,
            ..output(1, "DP-1", Rect::new(0, 0, 1920, 1080))
        };
        store
            .apply(&snapshot(Vec::new(), None).with_output_layouts(vec![moved]))
            .expect("applied");
        let mut changed: Vec<String> = drained(&mut store, &subscription)
            .into_iter()
            .filter_map(|observation| match observation {
                Observation::Changed { property, .. } => Some(property),
                _ => None,
            })
            .collect();
        changed.sort();
        assert_eq!(changed, vec!["primary".to_owned(), "x".to_owned()]);
    }

    #[test]
    fn a_new_window_publishes_an_empty_effects_list() {
        // The schema requires `effects` on every window, so a window with no effects
        // must still be created: without the property the graph refuses the object and
        // the window never appears in a listing.
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        let state = snapshot(
            vec![window(1, "Editor", true, Rect::new(0, 0, 200, 150))],
            Some(WindowId::new(1)),
        );
        store.apply(&state).expect("applied");
        let observations = drained(&mut store, &subscription);
        let view = opened(&observations, "gnoblin.window");
        assert_eq!(view.property("effects"), Some(&Value::List(Vec::new())));
    }

    #[test]
    fn the_session_object_names_the_version_this_process_serves() {
        let mut store = store();
        // It exists before the first subscriber, so it arrives in the snapshot rather
        // than as an `opened`.
        let subscription = store.subscribe(Filter::Object(store.session()));
        let view = subscription
            .snapshot()
            .object(store.session())
            .expect("the session object is in the snapshot");
        assert_eq!(view.ty().to_string(), "gnoblin.session");
        assert_eq!(view.property("version"), Some(&Value::Text("0.2.0".to_owned())));
    }

    #[test]
    fn an_unchanged_snapshot_publishes_nothing() {
        let mut store = store();
        let state = snapshot(
            vec![window(1, "Editor", true, Rect::new(0, 0, 200, 150))],
            Some(WindowId::new(1)),
        );
        store.apply(&state).expect("applied");
        let subscription = store.subscribe(Filter::All);
        let revision = store.graph().revision();
        store.apply(&state).expect("applied again");
        assert_eq!(drained(&mut store, &subscription), Vec::new(), "a repeat is not a diff");
        assert_eq!(store.graph().revision(), revision, "and it does not bump the revision");
    }

    /// A bell is raised once per ring and never for a snapshot that did not ring: the
    /// count the snapshot carries is the only source, so a subscriber's own count
    /// reaches it and a repeated snapshot raises nothing.
    #[test]
    fn a_bell_is_raised_once_per_ring() {
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        let mut state = snapshot(Vec::new(), None);
        state = state.with_bells(1);
        store.apply(&state).expect("applied");
        let bells: Vec<u64> = drained(&mut store, &subscription)
            .into_iter()
            .filter_map(|observation| match observation {
                Observation::Signalled { signal, fields, .. } if signal == "bell" => match fields.get("count") {
                    Some(Value::Uint(count)) => Some(*count),
                    other => panic!("the bell carries a count, not {other:?}"),
                },
                _ => None,
            })
            .collect();
        assert_eq!(bells, vec![1], "one ring, one signal, carrying the count");
        store.apply(&state).expect("applied again");
        assert_eq!(
            drained(&mut store, &subscription)
                .into_iter()
                .filter(|observation| matches!(observation, Observation::Signalled { signal, .. } if signal == "bell"))
                .count(),
            0,
            "an unchanged count raises nothing"
        );
        state = state.with_bells(3);
        store.apply(&state).expect("applied");
        assert_eq!(store.bells(), 3, "a gap is caught up, not dropped");
    }

    #[test]
    fn a_new_window_carries_every_declared_property() {
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        store
            .apply(&snapshot(
                vec![window(1, "Editor", true, Rect::new(10, 20, 300, 200))],
                Some(WindowId::new(1)),
            ))
            .expect("applied");
        let observations = drained(&mut store, &subscription);
        let view = opened(&observations, "gnoblin.window");
        assert_eq!(view.property("id"), Some(&Value::Uint(1)));
        assert_eq!(view.property("title"), Some(&Value::Text("Editor".to_owned())));
        assert_eq!(
            view.property("app_id"),
            Some(&Value::Text("org.gnoblin.Test".to_owned()))
        );
        assert_eq!(view.property("focused"), Some(&Value::Bool(true)));
        assert_eq!(view.property("x"), Some(&Value::Int(10)));
        assert_eq!(view.property("y"), Some(&Value::Int(20)));
        assert_eq!(view.property("width"), Some(&Value::Uint(300)));
        assert_eq!(view.property("height"), Some(&Value::Uint(200)));
    }

    #[test]
    fn geometry_keeps_a_negative_origin_and_unsigned_extents() {
        let mut store = store();
        let subscription = store.subscribe(Filter::All);
        store
            .apply(&snapshot(
                vec![window(1, "Editor", true, Rect::new(-5, 7, 200, 150))],
                Some(WindowId::new(1)),
            ))
            .expect("applied");
        let observations = drained(&mut store, &subscription);
        let view = opened(&observations, "gnoblin.window");
        assert_eq!(view.property("x"), Some(&Value::Int(-5)));
        assert_eq!(view.property("y"), Some(&Value::Int(7)));
        assert_eq!(view.property("width"), Some(&Value::Uint(200)));
        assert_eq!(view.property("height"), Some(&Value::Uint(150)));
    }

    #[test]
    fn a_moved_window_publishes_the_rectangle_and_the_focus_it_lost() {
        let mut store = store();
        store
            .apply(&snapshot(
                vec![window(1, "Editor", true, Rect::new(0, 0, 200, 150))],
                Some(WindowId::new(1)),
            ))
            .expect("applied");
        let subscription = store.subscribe(Filter::All);
        store
            .apply(&snapshot(
                vec![
                    window(1, "Editor", false, Rect::new(40, 0, 200, 150)),
                    window(2, "Terminal", true, Rect::new(400, 0, 200, 150)),
                ],
                Some(WindowId::new(2)),
            ))
            .expect("applied");
        let observations = drained(&mut store, &subscription);
        let changed: Vec<(String, Value)> = observations
            .iter()
            .filter_map(|observation| match observation {
                Observation::Changed { property, value, .. } => Some((property.clone(), value.clone())),
                _ => None,
            })
            .collect();
        assert!(
            changed.contains(&("x".to_owned(), Value::Int(40))),
            "the move is a diff, got {changed:?}"
        );
        assert!(
            changed.contains(&("focused".to_owned(), Value::Bool(false))),
            "the first window lost focus, got {changed:?}"
        );
        // The window that took focus is new, so its `focused` arrives with its
        // `opened` rather than as a separate diff.
        let terminal = opened(&observations, "gnoblin.window");
        assert_eq!(terminal.property("title"), Some(&Value::Text("Terminal".to_owned())));
        assert_eq!(terminal.property("focused"), Some(&Value::Bool(true)));
        assert!(
            observations
                .iter()
                .filter(|observation| matches!(observation, Observation::Opened { .. }))
                .count()
                == 1,
            "one window opened, got {observations:#?}"
        );
    }

    #[test]
    fn a_window_that_disappears_closes_its_object_once() {
        let mut store = store();
        store
            .apply(&snapshot(
                vec![window(1, "Editor", true, Rect::new(0, 0, 200, 150))],
                Some(WindowId::new(1)),
            ))
            .expect("applied");
        let object = store.windows().next().expect("the window object").1;
        let subscription = store.subscribe(Filter::Object(object));
        store.apply(&snapshot(Vec::new(), None)).expect("applied");
        let closed = drained(&mut store, &subscription)
            .into_iter()
            .filter(|observation| matches!(observation, Observation::Closed { .. }))
            .count();
        assert_eq!(closed, 1, "closed arrives exactly once");
        assert_eq!(store.window_of(object), None, "the mapping is gone");
        assert!(
            store.graph().object(object).is_none(),
            "the object is gone from the graph"
        );
    }

    #[test]
    fn a_window_object_is_found_by_its_identity() {
        let mut store = store();
        store
            .apply(&snapshot(
                vec![
                    window(7, "Editor", true, Rect::new(0, 0, 200, 150)),
                    window(9, "Terminal", false, Rect::new(400, 0, 200, 150)),
                ],
                Some(WindowId::new(7)),
            ))
            .expect("applied");
        let (window, object) = store.windows().next().expect("a window object");
        assert_eq!(window, WindowId::new(7));
        assert_eq!(store.window_of(object), Some(WindowId::new(7)));
        assert_eq!(store.window_of(store.session()), None, "the session is not a window");
    }

    #[test]
    fn a_refused_write_names_its_object_and_does_not_hide_the_others() {
        let mut store = store();
        store
            .apply(&snapshot(
                vec![
                    window(7, "one", true, Rect::default()),
                    window(8, "two", false, Rect::default()),
                ],
                Some(WindowId::new(7)),
            ))
            .expect("applied");
        // Close window 7's object behind the store's back, so its next write is refused.
        let broken = store.windows.get(&WindowId::new(7)).expect("published").object;
        store.graph_mut().close(broken).expect("closed");

        let error = store
            .apply(&snapshot(
                vec![
                    window(7, "renamed", true, Rect::default()),
                    window(8, "renamed too", false, Rect::default()),
                ],
                Some(WindowId::new(7)),
            ))
            .expect_err("the closed object refuses its write");
        let failures = error.failures();
        assert_eq!(failures.len(), 1, "only the broken object fails: {error}");
        assert_eq!(failures[0].kind(), "window");
        assert_eq!(failures[0].id(), 7, "the failure names the object that refused");
        assert!(
            error.to_string().starts_with("window 7:"),
            "the message names it: {error}"
        );

        // The healthy window still published its change.
        let healthy = store.windows.get(&WindowId::new(8)).expect("still published");
        assert_eq!(healthy.title, "renamed too", "one refusal must not hide the rest");
    }
}

/// Every object the last `apply` could not publish, with the object's kind and id.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishError {
    failures: Vec<PublishFailure>,
}

impl PublishError {
    /// The failures, in the order the snapshot reached them.
    #[must_use]
    pub fn failures(&self) -> &[PublishFailure] {
        &self.failures
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, failure) in self.failures.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{failure}")?;
        }
        Ok(())
    }
}

impl std::error::Error for PublishError {}

/// One object that failed to publish.
#[derive(Clone, Debug, PartialEq)]
pub struct PublishFailure {
    kind: &'static str,
    id: u64,
    error: ModelError,
}

impl PublishFailure {
    fn new(kind: &'static str, id: u64, error: ModelError) -> Self {
        Self { kind, id, error }
    }

    /// The kind of object that failed: `window`, `output` or `workspace`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// The id of the object that failed, as the snapshot names it.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The model's reason for refusing the write.
    #[must_use]
    pub fn error(&self) -> &ModelError {
        &self.error
    }
}

impl fmt::Display for PublishFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.kind, self.id, self.error)
    }
}
