//! The generated host API, and the store data every host call reaches.
//!
//! `bindgen!` compiles the generated WIT package into Rust traits: one trait per
//! namespace interface for its lookups, and one per object resource for its
//! getters, methods and `watch`. That is why a generated WIT is not documentation:
//! a schema member the host does not implement is a compile error, and a signature
//! that drifted from the schema is a compile error too.
//!
//! Three decisions in the macro invocation are load-bearing:
//!
//! - `with` maps every object resource to [`ObjectRep`], which holds the object's
//!   identity and nothing else. Without it the generated resource type is
//!   *uninhabited* (a resource the host owns has whatever representation the host
//!   chooses, and the default choice is "none"), so the host could not create a
//!   handle for a window it knows about. The keys are `gnoblin:ipc/<namespace>.<object>`
//!   without a version, which is the one form that does not have to be rewritten
//!   when the workspace version moves; the resource table then keeps identities, not
//!   data, because the data lives in one place (the mirror).
//! - `imports: { default: trappable }` lets a host call fail with a trap. The host
//!   pushes handles into a table, and a table that is out of room is a failure the
//!   module must see as a refusal rather than a panic in the compositor.
//! - There is no `async` and no `async-lift`: a module's call into the host runs on
//!   the thread that entered it, so the budget in [`crate::Budget`] is the only
//!   time bound and no store is moved across threads behind the caller's back.
//!
//! The hand-written part is the mapping from a WIT member to the object model:
//! property getters read the module's mirror, methods become IPC calls, `watch`
//! subscribes the module to one object, and lookups hand out a handle only for an
//! object the module's mirror has and its manifest lets it read.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use gnoblin_client::{Client, ClientError, Loopback};
use gnoblin_model::{Fields, ObjectId, Observation, OperationId, OperationOutcome, Value};
use wasmtime::StoreLimits;
use wasmtime::component::{Resource, ResourceTable, ResourceTableError};

use crate::log::{self, Level, LogLine};
use crate::manifest::Grants;
use crate::mirror::Mirror;

wasmtime::component::bindgen!({
    path: "generated/gnoblin-ipc.wit",
    world: "module",
    with: {
        "gnoblin:ipc/gnoblin.operation": ObjectRep,
        "gnoblin:ipc/gnoblin.request": ObjectRep,
        "gnoblin:ipc/gnoblin.session": ObjectRep,
        "gnoblin:ipc/gnoblin.window": ObjectRep,
        "gnoblin:ipc/gnoblin.effect": ObjectRep,
        "gnoblin:ipc/gnoblin.capture": ObjectRep,
        "gnoblin:ipc/gnoblin.eis": ObjectRep,
        "gnoblin:ipc/gnoblin.idle": ObjectRep,
        "gnoblin:ipc/gnoblin.lock": ObjectRep,
        "gnoblin:ipc/gnoblin.output": ObjectRep,
        "gnoblin:ipc/gnoblin.settings": ObjectRep,
        "gnoblin:ipc/gnoblin.workspace": ObjectRep,
        "gnoblin:ipc/gnoblin.layers": ObjectRep,
    },
    imports: { default: trappable },
});

/// The one representation every object resource has: which object it is.
///
/// A resource is a handle, and a handle's job here is to name an object. What the
/// module may do with it is the manifest's business, and what the object holds is
/// the mirror's, so the rep carries no data and the same type serves every object
/// type in every namespace.
pub struct ObjectRep {
    object: ObjectId,
}

/// Why the host refused something a module asked for.
///
/// The four cases are the host's words in the generated `error-code` enum; each
/// namespace's interface impl turns one into its own enum, which is the only place
/// the per-namespace types differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefusalKind {
    /// The manifest does not grant what the call needs.
    Denied,
    /// The identity is not an object of that type in the module's mirror.
    UnknownObject,
    /// The object type declares no such member.
    UnknownMember,
    /// The compositor refused the call, or the connection did not exist.
    Refused,
}

impl From<ResourceTableError> for RefusalKind {
    fn from(_: ResourceTableError) -> RefusalKind {
        RefusalKind::Refused
    }
}

/// One module's store: everything its host calls may reach.
///
/// It is the store's data and not the host's, so a module's failures cannot reach
/// another module's state and dropping the store drops the module's whole world.
pub(crate) struct StoreData {
    /// The module's name, for its log lines.
    pub(crate) module: String,
    /// What the manifest granted.
    pub(crate) capabilities: Grants,
    /// The limits the store's resource limiter enforces.
    pub(crate) limits: StoreLimits,
    /// The handles this module holds.
    pub(crate) table: ResourceTable,
    /// The module's copy of the graph.
    pub(crate) mirror: Mirror,
    /// The connection, once the host has subscribed it.
    pub(crate) client: Option<Client<Loopback>>,
    /// The observations the host owes the module before it reads its connection:
    /// the snapshot it inherited, as `opened` events.
    pub(crate) pending: VecDeque<Observation>,
    /// The objects whose events the module asked for.
    pub(crate) watched: BTreeSet<ObjectId>,
    /// The capabilities the host has already refused, so a denial is logged once.
    pub(crate) denied: BTreeSet<(String, String)>,
    /// How the operations the module was handed ended.
    pub(crate) outcomes: BTreeMap<OperationId, OperationOutcome>,
    /// Where lines go.
    pub(crate) sink: log::Shared,
}

impl StoreData {
    /// A store for a module that is being loaded and has not connected yet.
    pub(crate) fn new(module: String, capabilities: Grants, limits: StoreLimits, sink: log::Shared) -> StoreData {
        StoreData {
            module,
            capabilities,
            limits,
            table: ResourceTable::new(),
            mirror: Mirror::default(),
            client: None,
            pending: VecDeque::new(),
            watched: BTreeSet::new(),
            denied: BTreeSet::new(),
            outcomes: BTreeMap::new(),
            sink,
        }
    }

    /// Write one line through the shared sink.
    pub(crate) fn write(&self, level: Level, message: &str) {
        log::write(
            &self.sink,
            &LogLine {
                module: &self.module,
                level,
                message,
            },
        );
    }

    /// True when the manifest grants the capability, logging the first refusal.
    ///
    /// Once per (object, member) and not once per call: a module that retries a
    /// denied call in a loop must not be able to fill the log, and a reviewer needs
    /// the fact once, not the count.
    fn allowed(&mut self, object: &str, member: &str, granted: bool) -> bool {
        if granted {
            return true;
        }
        let key = (object.to_owned(), member.to_owned());
        if self.denied.insert(key) {
            self.write(
                Level::Warn,
                &format!("refused `{object}.{member}`: the manifest does not grant it"),
            );
        }
        false
    }

    /// The identity behind a handle.
    fn object(&self, handle: &Resource<ObjectRep>) -> Result<ObjectId, RefusalKind> {
        Ok(self.table.get(handle)?.object)
    }

    /// A handle for `object`, if the manifest grants reading it and the mirror has
    /// it with the expected type.
    fn open(&mut self, id: u64, object: &str) -> Result<Resource<ObjectRep>, RefusalKind> {
        if !self.allowed(object, "read", self.capabilities.may_read(object)) {
            return Err(RefusalKind::Denied);
        }
        let identity = ObjectId::new(id);
        match self.mirror.get(identity) {
            Some(entry) if entry.ty() == object => {}
            _ => return Err(RefusalKind::UnknownObject),
        }
        Ok(self.table.push(ObjectRep { object: identity })?)
    }

    /// One property value from the mirror.
    fn property(&self, object: ObjectId, property: &str) -> Option<&Value> {
        self.mirror.get(object)?.property(property)
    }

    /// A property as text. A property the mirror does not have (yet) reads empty:
    /// the getter never blocks and never fails, and the event that follows carries
    /// the real value.
    fn text(&self, object: ObjectId, property: &str) -> String {
        match self.property(object, property) {
            Some(Value::Text(text)) => text.clone(),
            _ => String::new(),
        }
    }

    /// A property as a boolean, reading `false` for anything else.
    fn boolean(&self, object: ObjectId, property: &str) -> bool {
        matches!(self.property(object, property), Some(Value::Bool(true)))
    }

    /// A property as a signed integer, reading `0` for anything else.
    fn int(&self, object: ObjectId, property: &str) -> i64 {
        match self.property(object, property) {
            Some(Value::Int(value)) => *value,
            _ => 0,
        }
    }

    /// A property as an unsigned integer, reading `0` for anything else.
    fn uint(&self, object: ObjectId, property: &str) -> u64 {
        match self.property(object, property) {
            Some(Value::Uint(value)) => *value,
            _ => 0,
        }
    }

    /// A property as a float, reading `0.0` for anything else.
    fn float(&self, object: ObjectId, property: &str) -> f64 {
        match self.property(object, property) {
            Some(Value::Double(value)) => *value,
            _ => 0.0,
        }
    }

    /// A property as a list of text. Entries of another shape are skipped, and a
    /// property the mirror does not have reads empty, like the other getters.
    fn texts(&self, object: ObjectId, property: &str) -> Vec<String> {
        let Some(Value::List(items)) = self.property(object, property) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| match item {
                Value::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// A property as a list of object identities. Entries of another shape are
    /// skipped, and a property the mirror does not have reads empty, like `texts`.
    fn objects(&self, object: ObjectId, property: &str) -> Vec<u64> {
        let Some(Value::List(items)) = self.property(object, property) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| match item {
                Value::Object(identity) => Some(identity.get()),
                _ => None,
            })
            .collect()
    }

    /// Call a method, returning the operation the completion is matched on.
    fn invoke(
        &mut self,
        object: ObjectId,
        object_type: &str,
        method: &str,
        arguments: Fields,
    ) -> Result<OperationId, RefusalKind> {
        if !self.allowed(object_type, method, self.capabilities.may_call(object_type, method)) {
            return Err(RefusalKind::Denied);
        }
        let Some(client) = self.client.as_mut() else {
            return Err(RefusalKind::Refused);
        };
        match client.call(object, method, &arguments) {
            Ok(operation) => Ok(operation.id()),
            Err(ClientError::Refused { code, .. }) => Err(if code == "unknown-member" {
                RefusalKind::UnknownMember
            } else {
                RefusalKind::Refused
            }),
            Err(_) => Err(RefusalKind::Refused),
        }
    }

    /// Record a handle for the operation a method returned.
    fn operation(&mut self, id: OperationId) -> Result<Resource<ObjectRep>, RefusalKind> {
        Ok(self.table.push(ObjectRep {
            object: ObjectId::new(id.get()),
        })?)
    }

    /// Subscribe the module to one object's events.
    fn watch(&mut self, handle: &Resource<ObjectRep>) -> Result<(), RefusalKind> {
        if !self.allowed("event", "watch", self.capabilities.may_subscribe()) {
            return Err(RefusalKind::Denied);
        }
        let object = self.object(handle)?;
        self.watched.insert(object);
        Ok(())
    }

    /// Where an operation the module holds is.
    fn operation_state(&self, handle: &Resource<ObjectRep>) -> Result<bool, RefusalKind> {
        let id = ObjectId::new(self.object(handle)?.get());
        match self.outcomes.get(&OperationId::new(id.get())) {
            Some(outcome) => Ok(outcome.is_done()),
            None => Ok(false),
        }
    }
}

/// The `error-code` case for one refusal, per namespace.
///
/// One function per namespace because the generated enums are distinct types that
/// happen to have the same cases. Every case a [`RefusalKind`] can be is declared in
/// every generated enum, so the mapping is total.
fn gnoblin_error(refusal: RefusalKind) -> gnoblin::ipc::gnoblin::ErrorCode {
    match refusal {
        RefusalKind::Denied => gnoblin::ipc::gnoblin::ErrorCode::Denied,
        RefusalKind::UnknownObject => gnoblin::ipc::gnoblin::ErrorCode::UnknownObject,
        RefusalKind::UnknownMember => gnoblin::ipc::gnoblin::ErrorCode::UnknownMember,
        RefusalKind::Refused => gnoblin::ipc::gnoblin::ErrorCode::Refused,
    }
}

/// Lookups, getters, methods and subscriptions for the `gnoblin` namespace.
mod gnoblin_objects {
    use super::gnoblin;
    use super::{ObjectRep, Resource, StoreData, Value, gnoblin_error};

    type ErrorCode = gnoblin::ipc::gnoblin::ErrorCode;

    impl gnoblin::ipc::gnoblin::Host for StoreData {
        fn window_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "window") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn effect_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "effect") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn session_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "session") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn request_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "request") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn capture_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "capture") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn eis_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "eis") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn idle_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "idle") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn lock_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "lock") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn output_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "output") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn settings_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "settings") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn workspace_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "workspace") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn layers_by_id(&mut self, id: u64) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.open(id, "layers") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }
    }

    impl gnoblin::ipc::gnoblin::HostOperation for StoreData {
        fn id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn state(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<gnoblin::ipc::gnoblin::OperationState> {
            match self.operation_state(&handle) {
                Ok(true) => Ok(gnoblin::ipc::gnoblin::OperationState::Succeeded),
                Ok(false) => Ok(gnoblin::ipc::gnoblin::OperationState::Pending),
                Err(_) => Ok(gnoblin::ipc::gnoblin::OperationState::Failed),
            }
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostWorkspace for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "id"))
        }

        fn layout(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "layout"))
        }

        fn set_layout(
            &mut self,
            handle: Resource<ObjectRep>,
            root: String,
            module: String,
            hash: String,
            options: Vec<u8>,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("root".to_owned(), Value::Text(root));
            arguments.insert("module".to_owned(), Value::Text(module));
            arguments.insert("hash".to_owned(), Value::Text(hash));
            arguments.insert("options".to_owned(), Value::Bytes(options));
            self.method(object, "workspace", "set_layout", arguments)
        }

        fn layout_command(
            &mut self,
            handle: Resource<ObjectRep>,
            name: String,
            params: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("name".to_owned(), Value::Text(name));
            arguments.insert("params".to_owned(), Value::Text(params));
            self.method(object, "workspace", "layout_command", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostWindow for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "id"))
        }

        fn title(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "title"))
        }

        fn app_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "app_id"))
        }

        /// The client process id; 0 when the compositor cannot name the process.
        fn pid(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "pid"))
        }

        fn focused(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "focused"))
        }

        fn unresponsive(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "unresponsive"))
        }

        fn kill(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            self.method(object, "window", "kill", gnoblin_model::Fields::new())
        }

        /// Ask the window's client to close. The client may refuse or ignore it.
        fn close(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            self.method(object, "window", "close", gnoblin_model::Fields::new())
        }

        fn set_minimized(
            &mut self,
            handle: Resource<ObjectRep>,
            value: bool,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("value".to_owned(), Value::Bool(value));
            self.method(object, "window", "set_minimized", arguments)
        }

        fn set_maximized(
            &mut self,
            handle: Resource<ObjectRep>,
            value: bool,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("value".to_owned(), Value::Bool(value));
            self.method(object, "window", "set_maximized", arguments)
        }

        fn set_fullscreen(
            &mut self,
            handle: Resource<ObjectRep>,
            value: bool,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("value".to_owned(), Value::Bool(value));
            self.method(object, "window", "set_fullscreen", arguments)
        }

        /// Move the top-left corner to `x`, `y` in global logical coordinates.
        fn move_(
            &mut self,
            handle: Resource<ObjectRep>,
            x: i64,
            y: i64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("x".to_owned(), Value::Int(x));
            arguments.insert("y".to_owned(), Value::Int(y));
            self.method(object, "window", "move", arguments)
        }

        /// Resize to `width` by `height` logical pixels, keeping the position.
        fn resize(
            &mut self,
            handle: Resource<ObjectRep>,
            width: u64,
            height: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("width".to_owned(), Value::Uint(width));
            arguments.insert("height".to_owned(), Value::Uint(height));
            self.method(object, "window", "resize", arguments)
        }

        /// Move to the workspace named by the `workspace` object identity.
        fn set_workspace(
            &mut self,
            handle: Resource<ObjectRep>,
            workspace: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert(
                "workspace".to_owned(),
                Value::Object(gnoblin_model::ObjectId::new(workspace)),
            );
            self.method(object, "window", "set_workspace", arguments)
        }

        /// Move to the output named by the `output` object identity.
        fn set_output(
            &mut self,
            handle: Resource<ObjectRep>,
            output: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("output".to_owned(), Value::Object(gnoblin_model::ObjectId::new(output)));
            self.method(object, "window", "set_output", arguments)
        }

        fn request_menu(
            &mut self,
            handle: Resource<ObjectRep>,
            x: i64,
            y: i64,
            width: u64,
            height: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("x".to_owned(), Value::Int(x));
            arguments.insert("y".to_owned(), Value::Int(y));
            arguments.insert("width".to_owned(), Value::Uint(width));
            arguments.insert("height".to_owned(), Value::Uint(height));
            self.method(object, "window", "request_menu", arguments)
        }

        /// The window's effects in paint order, as `effect` object identities.
        fn effects(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Vec<u64>> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.objects(object, "effects"))
        }

        /// Add an effect to the top of the window's effects. The parameters are the
        /// effect object's properties for `kind`; the compositor ignores those a kind
        /// does not use. The reply is an operation; a rejected kind fails it with
        /// `unknown-effect-kind`.
        #[allow(clippy::too_many_arguments)]
        fn add_effect(
            &mut self,
            handle: Resource<ObjectRep>,
            kind: String,
            downscale: u64,
            passes: u64,
            algorithm: String,
            noise: f64,
            brightness: f64,
            contrast: f64,
            saturation: f64,
            tint: String,
            offset_x: f64,
            offset_y: f64,
            blur_radius: f64,
            spread: f64,
            colour: String,
            source: String,
            entry: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("kind".to_owned(), Value::Text(kind));
            arguments.insert("downscale".to_owned(), Value::Uint(downscale));
            arguments.insert("passes".to_owned(), Value::Uint(passes));
            arguments.insert("algorithm".to_owned(), Value::Text(algorithm));
            arguments.insert("noise".to_owned(), Value::Double(noise));
            arguments.insert("brightness".to_owned(), Value::Double(brightness));
            arguments.insert("contrast".to_owned(), Value::Double(contrast));
            arguments.insert("saturation".to_owned(), Value::Double(saturation));
            arguments.insert("tint".to_owned(), Value::Text(tint));
            arguments.insert("offset_x".to_owned(), Value::Double(offset_x));
            arguments.insert("offset_y".to_owned(), Value::Double(offset_y));
            arguments.insert("blur_radius".to_owned(), Value::Double(blur_radius));
            arguments.insert("spread".to_owned(), Value::Double(spread));
            arguments.insert("colour".to_owned(), Value::Text(colour));
            arguments.insert("source".to_owned(), Value::Text(source));
            arguments.insert("entry".to_owned(), Value::Text(entry));
            self.method(object, "window", "add_effect", arguments)
        }

        fn icon(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "icon"))
        }

        fn tag(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "tag"))
        }

        fn modal(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "modal"))
        }

        fn x(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<i64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.int(object, "x"))
        }

        fn y(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<i64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.int(object, "y"))
        }

        fn width(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "width"))
        }

        fn height(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "height"))
        }

        fn focus(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(
                match self.invoke(object, "window", "focus", gnoblin_model::Fields::new()) {
                    Ok(operation) => match self.operation(operation) {
                        Ok(handle) => Ok(handle),
                        Err(refusal) => Err(gnoblin_error(refusal)),
                    },
                    Err(refusal) => Err(gnoblin_error(refusal)),
                },
            )
        }

        fn present(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(
                match self.invoke(object, "window", "present", gnoblin_model::Fields::new()) {
                    Ok(operation) => match self.operation(operation) {
                        Ok(handle) => Ok(handle),
                        Err(refusal) => Err(gnoblin_error(refusal)),
                    },
                    Err(refusal) => Err(gnoblin_error(refusal)),
                },
            )
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostEffect for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn kind(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "kind"))
        }

        fn enabled(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "enabled"))
        }

        fn active(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "active"))
        }

        fn reason(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "reason"))
        }

        fn downscale(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "downscale"))
        }

        fn passes(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "passes"))
        }

        fn algorithm(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "algorithm"))
        }

        fn noise(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "noise"))
        }

        fn brightness(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "brightness"))
        }

        fn contrast(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "contrast"))
        }

        fn saturation(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "saturation"))
        }

        fn tint(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "tint"))
        }

        fn offset_x(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "offset_x"))
        }

        fn offset_y(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "offset_y"))
        }

        fn blur_radius(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "blur_radius"))
        }

        fn spread(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "spread"))
        }

        fn colour(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "colour"))
        }

        fn source(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "source"))
        }

        fn entry(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "entry"))
        }

        /// Remove this effect from its window. The reply is an operation.
        fn remove(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            self.method(object, "effect", "remove", gnoblin_model::Fields::new())
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostSession for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn version(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "version"))
        }

        fn slot_auth_polkit(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "slot_auth_polkit"))
        }

        fn slot_prompt_secret(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "slot_prompt_secret"))
        }

        fn slot_location_agent(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "slot_location_agent"))
        }

        fn slot_idle_policy(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "slot_idle_policy"))
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostRequest for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn claim(
            &mut self,
            handle: Resource<ObjectRep>,
            request: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("request".to_owned(), Value::Uint(request));
            self.method(object, "request", "claim", arguments)
        }

        fn answer(
            &mut self,
            handle: Resource<ObjectRep>,
            request: u64,
            answer: Vec<u8>,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("request".to_owned(), Value::Uint(request));
            arguments.insert("answer".to_owned(), Value::Bytes(answer));
            // An answer may be a password, so it never reaches the log: the host
            // logs the refusal, never the arguments.
            self.method(object, "request", "answer", arguments)
        }

        fn decline(
            &mut self,
            handle: Resource<ObjectRep>,
            request: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("request".to_owned(), Value::Uint(request));
            self.method(object, "request", "decline", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostCapture for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn open(
            &mut self,
            handle: Resource<ObjectRep>,
            session: u64,
            owner: String,
            remote_session: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("session".to_owned(), Value::Uint(session));
            arguments.insert("owner".to_owned(), Value::Text(owner));
            arguments.insert("remote_session".to_owned(), Value::Text(remote_session));
            self.method(object, "capture", "open", arguments)
        }

        fn close(
            &mut self,
            handle: Resource<ObjectRep>,
            session: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("session".to_owned(), Value::Uint(session));
            self.method(object, "capture", "close", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostEis for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn open(
            &mut self,
            handle: Resource<ObjectRep>,
            session: u64,
            owner: String,
            capabilities: u64,
            path: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("session".to_owned(), Value::Uint(session));
            arguments.insert("owner".to_owned(), Value::Text(owner));
            arguments.insert("capabilities".to_owned(), Value::Uint(capabilities));
            arguments.insert("path".to_owned(), Value::Text(path));
            self.method(object, "eis", "open", arguments)
        }

        fn close(
            &mut self,
            handle: Resource<ObjectRep>,
            session: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("session".to_owned(), Value::Uint(session));
            self.method(object, "eis", "close", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostIdle for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn idle_ms(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "idle_ms"))
        }

        fn inhibitors(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "inhibitors"))
        }

        fn inhibit(
            &mut self,
            handle: Resource<ObjectRep>,
            holder: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("holder".to_owned(), Value::Text(holder));
            self.method(object, "idle", "inhibit", arguments)
        }

        fn uninhibit(
            &mut self,
            handle: Resource<ObjectRep>,
            holder: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("holder".to_owned(), Value::Text(holder));
            self.method(object, "idle", "uninhibit", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostLayers for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn register(
            &mut self,
            handle: Resource<ObjectRep>,
            local: String,
            target: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("local".to_owned(), Value::Text(local));
            arguments.insert("target".to_owned(), Value::Uint(target));
            self.method(object, "layers", "register", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostLock for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn locked(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "locked"))
        }

        fn lock_session(
            &mut self,
            handle: Resource<ObjectRep>,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            self.method(object, "lock", "lock_session", gnoblin_model::Fields::new())
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostOutput for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn name(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "name"))
        }

        fn make(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "make"))
        }

        fn model(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "model"))
        }

        fn modes(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Vec<String>> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.texts(object, "modes"))
        }

        fn mode(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "mode"))
        }

        fn layout(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "layout"))
        }

        fn set_layout(
            &mut self,
            handle: Resource<ObjectRep>,
            root: String,
            module: String,
            hash: String,
            options: Vec<u8>,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("root".to_owned(), Value::Text(root));
            arguments.insert("module".to_owned(), Value::Text(module));
            arguments.insert("hash".to_owned(), Value::Text(hash));
            arguments.insert("options".to_owned(), Value::Bytes(options));
            self.method(object, "output", "set_layout", arguments)
        }

        fn scale(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "scale"))
        }

        fn transform(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "transform"))
        }

        fn primary(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "primary"))
        }

        fn enabled(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "enabled"))
        }

        fn x(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<i64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.int(object, "x"))
        }

        fn y(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<i64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.int(object, "y"))
        }

        fn width(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "width"))
        }

        fn height(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "height"))
        }

        #[allow(clippy::too_many_arguments)]
        fn apply(
            &mut self,
            handle: Resource<ObjectRep>,
            x: i64,
            y: i64,
            scale: f64,
            transform: u64,
            primary: bool,
            enabled: bool,
            mode: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("x".to_owned(), Value::Int(x));
            arguments.insert("y".to_owned(), Value::Int(y));
            arguments.insert("scale".to_owned(), Value::Double(scale));
            arguments.insert("transform".to_owned(), Value::Uint(transform));
            arguments.insert("primary".to_owned(), Value::Bool(primary));
            arguments.insert("enabled".to_owned(), Value::Bool(enabled));
            arguments.insert("mode".to_owned(), Value::Text(mode));
            self.method(object, "output", "apply", arguments)
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::gnoblin::HostSettings for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn color_scheme(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "color_scheme"))
        }

        fn accent_red(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "accent_red"))
        }

        fn accent_green(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "accent_green"))
        }

        fn accent_blue(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "accent_blue"))
        }

        fn contrast(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "contrast"))
        }

        fn reduced_motion(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "reduced_motion"))
        }

        fn gtk_theme(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "gtk_theme"))
        }

        fn icon_theme(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "icon_theme"))
        }

        fn cursor_theme(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "cursor_theme"))
        }

        fn cursor_size(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.uint(object, "cursor_size"))
        }

        fn font_name(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "font_name"))
        }

        fn document_font_name(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "document_font_name"))
        }

        fn monospace_font_name(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "monospace_font_name"))
        }

        fn text_scaling_factor(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<f64> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.float(object, "text_scaling_factor"))
        }

        fn button_layout(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<String> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.text(object, "button_layout"))
        }

        fn enable_animations(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "enable_animations"))
        }

        fn watch(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<Result<bool, ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl StoreData {
        /// A method call that returns a handle, shared by the methods with
        /// parameters.
        fn method(
            &mut self,
            object: gnoblin_types::ObjectId,
            object_type: &str,
            method: &str,
            arguments: gnoblin_model::Fields,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, ErrorCode>> {
            Ok(match self.invoke(object, object_type, method, arguments) {
                Ok(operation) => match self.operation(operation) {
                    Ok(handle) => Ok(handle),
                    Err(refusal) => Err(gnoblin_error(refusal)),
                },
                Err(refusal) => Err(gnoblin_error(refusal)),
            })
        }
    }
}
