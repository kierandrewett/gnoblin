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

use gnoblin_model::{Fields, Filter, Graph, ModelError, ObjectId, Observation, Subscription, TypeKey, Value};
use gnoblin_schema::Catalog;
use gnoblin_types::{Rect, Version, WindowId};
use gnoblin_wayland::SessionSnapshot;
use tracing::error;

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
    /// The `window` object's identity.
    pub const ID: &str = "id";
    pub const TITLE: &str = "title";
    pub const APP_ID: &str = "app_id";
    pub const FOCUSED: &str = "focused";
    pub const X: &str = "x";
    pub const Y: &str = "y";
    pub const WIDTH: &str = "width";
    pub const HEIGHT: &str = "height";
}

/// What the graph last published for one window.
#[derive(Clone, Debug, PartialEq)]
struct PublishedWindow {
    object: ObjectId,
    title: String,
    app_id: String,
    focused: bool,
    geometry: Rect,
}

/// The live object graph of one compositor process.
#[derive(Debug)]
pub struct SessionSync {
    graph: Graph,
    session: ObjectId,
    /// How many bells the last snapshot counted, so `apply` raises the signal once per
    /// ring rather than once per snapshot.
    bells: u64,
    windows: BTreeMap<WindowId, PublishedWindow>,
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
        Ok(SessionSync {
            graph,
            session,
            bells: 0,
            windows: BTreeMap::new(),
        })
    }

    /// How many bells this sync has raised.
    #[must_use]
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// Raises `session.ready`, once, when the IPC server is about to accept.
    ///
    /// A subscriber that connects afterwards starts from the snapshot instead, which
    /// is why this is a signal and not a property: "the session can accept requests"
    /// is an event, and the state it leaves behind is the version.
    pub fn ready(&mut self) {
        if let Err(error) = self.graph.signal(self.session, member::READY, Fields::new()) {
            error!(%error, "[ipc] the session object declares no ready signal");
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
                error!(%error, "[ipc] the session object declares no bell signal");
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
    /// leaves every subscriber's view whole.
    pub fn apply(&mut self, snapshot: &SessionSnapshot) -> Result<(), ModelError> {
        self.raise_bell(snapshot.bells());
        let focus = snapshot.focus();
        let mut seen = BTreeSet::new();
        for window in snapshot.windows() {
            seen.insert(window.id);
            let focused = focus == Some(window.id);
            let Some(previous) = self.windows.get(&window.id).cloned() else {
                let properties = window_properties(window.id, window);
                let object = self.graph.create(TypeKey::gnoblin("window"), properties)?;
                self.windows.insert(
                    window.id,
                    PublishedWindow {
                        object,
                        title: window.title.clone(),
                        app_id: window.app_id.clone(),
                        focused,
                        geometry: window.geometry,
                    },
                );
                continue;
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
            if previous.geometry != window.geometry {
                self.set_geometry(object, window.geometry)?;
            }
            self.windows.insert(
                window.id,
                PublishedWindow {
                    object,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    focused,
                    geometry: window.geometry,
                },
            );
        }
        let lost: Vec<WindowId> = self
            .windows
            .keys()
            .filter(|window| !seen.contains(window))
            .copied()
            .collect();
        for window in lost {
            if let Some(published) = self.windows.remove(&window) {
                // The object's own end is the one event. The schema also declares a
                // `closed` signal, and it is deliberately not raised as well: a
                // subscriber would learn the same thing twice.
                self.graph.close(published.object)?;
            }
        }
        Ok(())
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
    ])
}

#[cfg(test)]
mod tests {
    use gnoblin_model::{Filter, Observation, Subscription, Value};
    use gnoblin_types::{Rect, Version, WindowId};
    use gnoblin_wayland::{ObservedWindow, SessionSnapshot};

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
}
