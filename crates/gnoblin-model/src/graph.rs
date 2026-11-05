//! The object graph: objects, revisions, observations and subscriptions.
//!
//! One rule holds the whole file together. Every mutation validates against the
//! schema first, then bumps the single revision counter, then pushes the
//! resulting [`Observation`] into every matching subscription queue in that same
//! step. Because the bump and the push happen together and the queue is a FIFO
//! drained in order, a subscription's stream is strictly increasing in revision
//! and neither a late subscriber nor a fast one can miss or repeat a diff.

use std::collections::{BTreeMap, VecDeque};

use gnoblin_schema::{Catalog, Type};

use gnoblin_types::{ErrorCode, ObjectId, OperationId, Revision, SubscriptionId};

use crate::error::ModelError;
use crate::object::Object;
use crate::operation::{Operation, OperationOutcome};
use crate::types::{TypeKey, Types};
use crate::value::{Fields, Value};

/// An object as a subscriber sees it: identity, type and every property value.
///
/// This is what a snapshot contains and what [`Observation::Opened`] carries, so a
/// subscriber can build its view of an object from one value in both cases.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectView {
    id: ObjectId,
    ty: TypeKey,
    properties: Fields,
}

impl ObjectView {
    /// Assemble a view, for a snapshot the graph built or one a client decoded
    /// from the wire.
    pub fn new(id: ObjectId, ty: TypeKey, properties: Fields) -> ObjectView {
        ObjectView { id, ty, properties }
    }

    /// The object's identity.
    pub fn id(&self) -> ObjectId {
        self.id
    }

    /// The object's type.
    pub fn ty(&self) -> &TypeKey {
        &self.ty
    }

    /// Every property value, ordered by name.
    pub fn properties(&self) -> &Fields {
        &self.properties
    }

    /// One property value.
    pub fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }
}

/// Everything the graph held at one revision: the half of the rule a subscriber
/// receives once, before any diff.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    revision: Revision,
    objects: Vec<ObjectView>,
}

impl Snapshot {
    /// Assemble a snapshot, for the graph or for a client that decoded one.
    pub fn new(revision: Revision, objects: Vec<ObjectView>) -> Snapshot {
        Snapshot { revision, objects }
    }

    /// The revision the snapshot describes. Every observation a subscriber
    /// receives afterwards is later than this.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Every object the subscription covers, in identity order.
    pub fn objects(&self) -> &[ObjectView] {
        &self.objects
    }

    /// One covered object.
    pub fn object(&self, id: ObjectId) -> Option<&ObjectView> {
        self.objects.iter().find(|object| object.id() == id)
    }
}

/// Something the graph published, and the only way state leaves it.
#[derive(Clone, Debug, PartialEq)]
pub enum Observation {
    /// An object came into existence with its complete initial state.
    Opened {
        /// The new object and all its property values.
        view: ObjectView,
        /// The revision the object was created at.
        revision: Revision,
    },
    /// One property changed.
    Changed {
        /// The object that changed.
        object: ObjectId,
        /// The property's schema name.
        property: String,
        /// The new value.
        value: Value,
        /// The revision of this change.
        revision: Revision,
    },
    /// The compositor raised a declarative signal on an object.
    Signalled {
        /// The object that raised it.
        object: ObjectId,
        /// The signal's schema name.
        signal: String,
        /// The signal's declared fields.
        fields: Fields,
        /// The revision of this signal.
        revision: Revision,
    },
    /// The object is gone. It arrives exactly once, and its identity is never
    /// reused.
    Closed {
        /// The object that ended.
        object: ObjectId,
        /// The revision of the object's end.
        revision: Revision,
    },
    /// A method call finished, once.
    Finished {
        /// The operation the call returned.
        operation: OperationId,
        /// The object the method was called on. It may already have ended, which
        /// is why the outcome can be [`ErrorCode::object_gone`].
        object: ObjectId,
        /// How the call ended.
        outcome: OperationOutcome,
        /// The revision of the completion.
        revision: Revision,
    },
}

impl Observation {
    /// The revision this observation was published at.
    pub fn revision(&self) -> Revision {
        match self {
            Observation::Opened { revision, .. }
            | Observation::Changed { revision, .. }
            | Observation::Signalled { revision, .. }
            | Observation::Closed { revision, .. }
            | Observation::Finished { revision, .. } => *revision,
        }
    }

    /// The object this observation is about.
    pub fn object(&self) -> ObjectId {
        match self {
            Observation::Opened { view, .. } => view.id(),
            Observation::Changed { object, .. }
            | Observation::Signalled { object, .. }
            | Observation::Closed { object, .. }
            | Observation::Finished { object, .. } => *object,
        }
    }
}

/// Which objects a subscription covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Every object in the graph.
    All,
    /// One object, from its snapshot entry until its [`Observation::Closed`].
    Object(ObjectId),
}

impl Filter {
    /// Whether an object belongs in the snapshot.
    fn covers(&self, object: &Object) -> bool {
        match self {
            Filter::All => true,
            Filter::Object(id) => object.id() == *id,
        }
    }

    /// Whether a later observation belongs in the queue.
    fn matches(&self, observation: &Observation) -> bool {
        match self {
            Filter::All => true,
            Filter::Object(id) => observation.object() == *id,
        }
    }
}

/// One subscription: the snapshot taken when it was created, and the identity its
/// later observations are drained by.
///
/// Held by the subscriber, not by the graph, so a subscriber cannot lose its
/// snapshot behind the graph's back and cannot subscribe twice by accident.
#[derive(Clone, Debug)]
pub struct Subscription {
    id: SubscriptionId,
    snapshot: Snapshot,
}

impl Subscription {
    /// The identity the graph queues observations under.
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    /// The state at the moment of subscribing.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

/// What the graph holds for one subscription.
#[derive(Debug)]
struct SubscriptionState {
    filter: Filter,
    queue: VecDeque<Observation>,
}

/// The live object graph.
#[derive(Debug)]
pub struct Graph {
    types: Types,
    revision: Revision,
    next_object: u64,
    next_operation: u64,
    next_subscription: u64,
    objects: BTreeMap<ObjectId, Object>,
    pending: BTreeMap<OperationId, Operation>,
    subscriptions: BTreeMap<SubscriptionId, SubscriptionState>,
}

impl Graph {
    /// Build a graph whose vocabulary is a validated schema catalog.
    pub fn new(catalog: Catalog) -> Graph {
        Graph {
            types: Types::index(&catalog),
            revision: Revision::ZERO,
            // Identities start at one so zero stays an "absent" sentinel on the
            // wire and in a frame dump.
            next_object: 1,
            next_operation: 1,
            next_subscription: 1,
            objects: BTreeMap::new(),
            pending: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
        }
    }

    /// The revision of the latest published observation.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// One live object.
    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(&id)
    }

    /// Every live object, in identity order.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }

    /// How many objects are live.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether no object is live.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Every operation still waiting for a completion.
    pub fn pending(&self) -> impl Iterator<Item = Operation> + '_ {
        self.pending.values().copied()
    }

    /// Create an object with its complete initial state.
    ///
    /// Every declared property must be supplied, exactly once: creation is atomic,
    /// so a subscriber either sees an object with all of its values or never sees
    /// it at all. A refused creation publishes nothing.
    pub fn create(&mut self, ty: TypeKey, properties: Fields) -> Result<ObjectId, ModelError> {
        let declared: Vec<(String, Type)> = {
            let object_type = self
                .types
                .get(&ty)
                .ok_or_else(|| ModelError::UnknownType { ty: ty.clone() })?;
            object_type
                .properties
                .iter()
                .map(|property| (property.name.clone(), property.ty.clone()))
                .collect()
        };
        for (property, expected) in &declared {
            match properties.get(property) {
                None => {
                    return Err(ModelError::MissingProperty {
                        ty,
                        property: property.clone(),
                    });
                }
                Some(value) if !value.fits(expected) => {
                    return Err(ModelError::WrongType {
                        member: format!("property `{property}` of `{ty}`"),
                        expected: expected.clone(),
                        got: value.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        if let Some(undeclared) = properties
            .keys()
            .find(|name| !declared.iter().any(|(known, _)| known == *name))
        {
            return Err(ModelError::UnexpectedProperty {
                ty,
                property: undeclared.clone(),
            });
        }
        let id = ObjectId::new(self.next_object);
        self.next_object += 1;
        let revision = self.bump();
        let object = Object::new(id, ty, properties, revision);
        let view = object.view();
        self.objects.insert(id, object);
        self.publish(Observation::Opened { view, revision });
        Ok(id)
    }

    /// Set one property, publishing a diff.
    pub fn set(&mut self, object: ObjectId, property: &str, value: Value) -> Result<Revision, ModelError> {
        let (ty, expected) = {
            let live = self.objects.get(&object).ok_or(ModelError::UnknownObject { object })?;
            let ty = live.ty().clone();
            let object_type = self
                .types
                .get(&ty)
                .ok_or_else(|| ModelError::UnknownType { ty: ty.clone() })?;
            let expected = object_type
                .properties
                .iter()
                .find(|declared| declared.name == property)
                .map(|declared| declared.ty.clone());
            (ty, expected)
        };
        let expected = expected.ok_or_else(|| ModelError::UnknownProperty {
            ty: ty.clone(),
            property: property.to_owned(),
        })?;
        if !value.fits(&expected) {
            return Err(ModelError::WrongType {
                member: format!("property `{property}` of `{ty}`"),
                expected,
                got: value,
            });
        }
        let revision = self.bump();
        {
            let live = self
                .objects
                .get_mut(&object)
                .ok_or(ModelError::UnknownObject { object })?;
            live.set(property.to_owned(), value.clone(), revision);
        }
        self.publish(Observation::Changed {
            object,
            property: property.to_owned(),
            value,
            revision,
        });
        Ok(revision)
    }

    /// Raise a declared signal on an object.
    ///
    /// Signals are declarative: the compositor says the thing happened and this
    /// crate validates the shape. Nothing in the graph reacts to a signal.
    pub fn signal(&mut self, object: ObjectId, signal: &str, fields: Fields) -> Result<Revision, ModelError> {
        let (ty, declared) = {
            let live = self.objects.get(&object).ok_or(ModelError::UnknownObject { object })?;
            let ty = live.ty().clone();
            let object_type = self
                .types
                .get(&ty)
                .ok_or_else(|| ModelError::UnknownType { ty: ty.clone() })?;
            let declared: Option<Vec<(String, Type)>> = object_type
                .signals
                .iter()
                .find(|declared| declared.name == signal)
                .map(|declared| {
                    declared
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone()))
                        .collect()
                });
            (ty, declared)
        };
        let declared = declared.ok_or_else(|| ModelError::UnknownSignal {
            ty: ty.clone(),
            signal: signal.to_owned(),
        })?;
        for (field, expected) in &declared {
            match fields.get(field) {
                None => {
                    return Err(ModelError::MissingField {
                        signal: signal.to_owned(),
                        field: field.clone(),
                    });
                }
                Some(value) if !value.fits(expected) => {
                    return Err(ModelError::WrongType {
                        member: format!("field `{field}` of signal `{signal}`"),
                        expected: expected.clone(),
                        got: value.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        if let Some(undeclared) = fields
            .keys()
            .find(|name| !declared.iter().any(|(known, _)| known == *name))
        {
            return Err(ModelError::UnexpectedField {
                signal: signal.to_owned(),
                field: undeclared.clone(),
            });
        }
        let revision = self.bump();
        self.publish(Observation::Signalled {
            object,
            signal: signal.to_owned(),
            fields,
            revision,
        });
        Ok(revision)
    }

    /// End an object's life.
    ///
    /// Pending operations for the object are completed with
    /// [`ErrorCode::object_gone`] first, so a client waiting for one learns the
    /// reason instead of waiting for an answer that cannot come. The
    /// [`Observation::Closed`] observation follows, exactly once.
    pub fn close(&mut self, object: ObjectId) -> Result<Revision, ModelError> {
        if !self.objects.contains_key(&object) {
            return Err(ModelError::UnknownObject { object });
        }
        let pending: Vec<OperationId> = self
            .pending
            .values()
            .filter(|operation| operation.object() == object)
            .map(|operation| operation.id())
            .collect();
        for operation in pending {
            self.complete(operation, OperationOutcome::Failed(object_gone()?))?;
        }
        self.objects.remove(&object);
        let revision = self.bump();
        self.publish(Observation::Closed { object, revision });
        Ok(revision)
    }

    /// Call a declared method, returning the operation the call will complete.
    ///
    /// Nothing is published here: the call is a request to the compositor, and its
    /// effect appears as property diffs and as the single completion. Arguments are
    /// checked against the schema before the operation is issued, so an operation
    /// always names a call that was valid when it started.
    pub fn call(&mut self, object: ObjectId, method: &str, arguments: Fields) -> Result<Operation, ModelError> {
        let (ty, declared) = {
            let live = self.objects.get(&object).ok_or(ModelError::UnknownObject { object })?;
            let ty = live.ty().clone();
            let object_type = self
                .types
                .get(&ty)
                .ok_or_else(|| ModelError::UnknownType { ty: ty.clone() })?;
            let declared: Option<Vec<(String, Type)>> = object_type
                .methods
                .iter()
                .find(|declared| declared.name == method)
                .map(|declared| {
                    declared
                        .params
                        .iter()
                        .map(|parameter| (parameter.name.clone(), parameter.ty.clone()))
                        .collect()
                });
            (ty, declared)
        };
        let declared = declared.ok_or_else(|| ModelError::UnknownMethod {
            ty,
            method: method.to_owned(),
        })?;
        for (parameter, expected) in &declared {
            match arguments.get(parameter) {
                None => {
                    return Err(ModelError::MissingArgument {
                        method: method.to_owned(),
                        parameter: parameter.clone(),
                    });
                }
                Some(value) if !value.fits(expected) => {
                    return Err(ModelError::WrongType {
                        member: format!("parameter `{parameter}` of method `{method}`"),
                        expected: expected.clone(),
                        got: value.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        if let Some(undeclared) = arguments
            .keys()
            .find(|name| !declared.iter().any(|(known, _)| known == *name))
        {
            return Err(ModelError::UnexpectedArgument {
                method: method.to_owned(),
                parameter: undeclared.clone(),
            });
        }
        let operation = Operation::new(OperationId::new(self.next_operation), object);
        self.next_operation += 1;
        self.pending.insert(operation.id(), operation);
        Ok(operation)
    }

    /// Complete an operation with its one outcome.
    ///
    /// A second completion of the same operation is refused with
    /// [`ModelError::UnknownOperation`], because the pending entry is consumed here
    /// rather than marked: there is no window in which an operation can be
    /// completed twice.
    pub fn complete(&mut self, operation: OperationId, outcome: OperationOutcome) -> Result<Revision, ModelError> {
        let pending = self
            .pending
            .remove(&operation)
            .ok_or(ModelError::UnknownOperation { operation })?;
        let revision = self.bump();
        self.publish(Observation::Finished {
            operation,
            object: pending.object(),
            outcome,
            revision,
        });
        Ok(revision)
    }

    /// Subscribe at the graph's current revision.
    ///
    /// The snapshot and the queue are taken in one step, which is the whole of the
    /// late-subscriber rule: the snapshot carries everything up to now, and the
    /// queue can only receive later observations.
    pub fn subscribe(&mut self, filter: Filter) -> Subscription {
        let id = SubscriptionId::new(self.next_subscription);
        self.next_subscription += 1;
        let snapshot = Snapshot::new(
            self.revision,
            self.objects
                .values()
                .filter(|object| filter.covers(object))
                .map(Object::view)
                .collect(),
        );
        self.subscriptions.insert(
            id,
            SubscriptionState {
                filter,
                queue: VecDeque::new(),
            },
        );
        Subscription { id, snapshot }
    }

    /// The next observation for a subscription, in revision order.
    ///
    /// `None` means the subscription has nothing queued: either nothing happened or
    /// it has caught up. It never means the subscription ended.
    pub fn next(&mut self, subscription: &Subscription) -> Option<Observation> {
        self.subscriptions
            .get_mut(&subscription.id)
            .and_then(|state| state.queue.pop_front())
    }

    /// Drop a subscription's queue. Whether it existed is reported so a caller that
    /// double-unsubscribes notices.
    pub fn unsubscribe(&mut self, subscription: SubscriptionId) -> bool {
        self.subscriptions.remove(&subscription).is_some()
    }

    /// Advance the revision and stamp the observation with it.
    ///
    /// The bump and the publish are separate statements but not separable in
    /// effect: nothing can observe the gap, because no caller gets control between
    /// them.
    fn bump(&mut self) -> Revision {
        let revision = self.revision.next();
        self.revision = revision;
        revision
    }

    /// Push an observation into every matching queue.
    fn publish(&mut self, observation: Observation) {
        for state in self.subscriptions.values_mut() {
            if state.filter.matches(&observation) {
                state.queue.push_back(observation.clone());
            }
        }
    }
}

/// The code the graph reports for an operation whose object ended first.
///
/// `object-gone` is a literal that satisfies the kebab rule, so validation cannot
/// fail; the failure is reported as a [`ModelError`] rather than a panic because
/// the graph has no panic path, and a test pins the literal.
fn object_gone() -> Result<ErrorCode, ModelError> {
    ErrorCode::parse("object-gone").ok_or_else(|| ModelError::InvalidErrorCode {
        text: "object-gone".to_owned(),
    })
}
