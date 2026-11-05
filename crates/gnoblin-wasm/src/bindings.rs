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

use crate::log::{self, Level, Line};
use crate::manifest::Capabilities;
use crate::mirror::Mirror;

wasmtime::component::bindgen!({
    path: "generated/gnoblin-ipc.wit",
    world: "module",
    with: {
        "gnoblin:ipc/gnoblin.operation": ObjectRep,
        "gnoblin:ipc/gnoblin.request": ObjectRep,
        "gnoblin:ipc/gnoblin.session": ObjectRep,
        "gnoblin:ipc/gnoblin.window": ObjectRep,
        "gnoblin:ipc/bsm-greeter.operation": ObjectRep,
        "gnoblin:ipc/bsm-greeter.greeter": ObjectRep,
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
    pub(crate) capabilities: Capabilities,
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
    pub(crate) fn new(module: String, capabilities: Capabilities, limits: StoreLimits, sink: log::Shared) -> StoreData {
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
            &Line {
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

fn greeter_error(refusal: RefusalKind) -> gnoblin::ipc::bsm_greeter::ErrorCode {
    match refusal {
        RefusalKind::Denied => gnoblin::ipc::bsm_greeter::ErrorCode::Denied,
        RefusalKind::UnknownObject => gnoblin::ipc::bsm_greeter::ErrorCode::UnknownObject,
        RefusalKind::UnknownMember => gnoblin::ipc::bsm_greeter::ErrorCode::UnknownMember,
        RefusalKind::Refused => gnoblin::ipc::bsm_greeter::ErrorCode::Refused,
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

        fn focused(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<bool> {
            let object = self.object(&handle).expect("a handle this store issued");
            Ok(self.boolean(object, "focused"))
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

/// The same, for the `bsm.greeter` extension namespace.
mod greeter_objects {
    use super::gnoblin;
    use super::{ObjectRep, Resource, StoreData, greeter_error};

    impl gnoblin::ipc::bsm_greeter::Host for StoreData {
        fn greeter_by_id(
            &mut self,
            id: u64,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, gnoblin::ipc::bsm_greeter::ErrorCode>> {
            Ok(match self.open(id, "greeter") {
                Ok(handle) => Ok(handle),
                Err(refusal) => Err(greeter_error(refusal)),
            })
        }
    }

    impl gnoblin::ipc::bsm_greeter::HostOperation for StoreData {
        fn id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn state(
            &mut self,
            handle: Resource<ObjectRep>,
        ) -> wasmtime::Result<gnoblin::ipc::bsm_greeter::OperationState> {
            match self.operation_state(&handle) {
                Ok(true) => Ok(gnoblin::ipc::bsm_greeter::OperationState::Succeeded),
                Ok(false) => Ok(gnoblin::ipc::bsm_greeter::OperationState::Pending),
                Err(_) => Ok(gnoblin::ipc::bsm_greeter::OperationState::Failed),
            }
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }

    impl gnoblin::ipc::bsm_greeter::HostGreeter for StoreData {
        fn object_id(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<u64> {
            Ok(self.object(&handle).map(|object| object.get()).unwrap_or(0))
        }

        fn begin_auth(
            &mut self,
            handle: Resource<ObjectRep>,
            user: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, gnoblin::ipc::bsm_greeter::ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("user".to_owned(), gnoblin_model::Value::Text(user));
            Ok(match self.invoke(object, "greeter", "begin_auth", arguments) {
                Ok(operation) => match self.operation(operation) {
                    Ok(handle) => Ok(handle),
                    Err(refusal) => Err(greeter_error(refusal)),
                },
                Err(refusal) => Err(greeter_error(refusal)),
            })
        }

        fn auth_response(
            &mut self,
            handle: Resource<ObjectRep>,
            response: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, gnoblin::ipc::bsm_greeter::ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("response".to_owned(), gnoblin_model::Value::Text(response));
            Ok(match self.invoke(object, "greeter", "auth_response", arguments) {
                Ok(operation) => match self.operation(operation) {
                    Ok(handle) => Ok(handle),
                    Err(refusal) => Err(greeter_error(refusal)),
                },
                Err(refusal) => Err(greeter_error(refusal)),
            })
        }

        fn power(
            &mut self,
            handle: Resource<ObjectRep>,
            action: String,
        ) -> wasmtime::Result<Result<Resource<ObjectRep>, gnoblin::ipc::bsm_greeter::ErrorCode>> {
            let object = self.object(&handle).expect("a handle this store issued");
            let mut arguments = gnoblin_model::Fields::new();
            arguments.insert("action".to_owned(), gnoblin_model::Value::Text(action));
            Ok(match self.invoke(object, "greeter", "power", arguments) {
                Ok(operation) => match self.operation(operation) {
                    Ok(handle) => Ok(handle),
                    Err(refusal) => Err(greeter_error(refusal)),
                },
                Err(refusal) => Err(greeter_error(refusal)),
            })
        }

        fn watch(
            &mut self,
            handle: Resource<ObjectRep>,
        ) -> wasmtime::Result<Result<bool, gnoblin::ipc::bsm_greeter::ErrorCode>> {
            Ok(match self.watch(&handle) {
                Ok(()) => Ok(true),
                Err(refusal) => Err(greeter_error(refusal)),
            })
        }

        fn drop(&mut self, handle: Resource<ObjectRep>) -> wasmtime::Result<()> {
            let _ = self.table.delete(handle);
            Ok(())
        }
    }
}
