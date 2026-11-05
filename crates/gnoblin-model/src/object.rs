//! One live object.

use crate::graph::ObjectView;
use crate::ids::ObjectId;
use crate::revision::Revision;
use crate::types::TypeKey;
use crate::value::{Fields, Value};

/// One live object: its identity, its type, its typed property values and the
/// revision at which it last changed.
///
/// The graph owns objects and only the graph mutates them, so a property can never
/// change without an observation being published for it.
#[derive(Clone, Debug)]
pub struct Object {
    id: ObjectId,
    ty: TypeKey,
    properties: Fields,
    revision: Revision,
}

impl Object {
    /// Build an object whose properties have already been checked against the
    /// schema.
    pub(crate) fn new(id: ObjectId, ty: TypeKey, properties: Fields, revision: Revision) -> Object {
        Object {
            id,
            ty,
            properties,
            revision,
        }
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

    /// The revision of the object's latest change.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// A copy of the object as a subscriber sees it.
    pub fn view(&self) -> ObjectView {
        ObjectView::new(self.id, self.ty.clone(), self.properties.clone())
    }

    /// Change one property; the graph has already validated it.
    pub(crate) fn set(&mut self, property: String, value: Value, revision: Revision) {
        self.properties.insert(property, value);
        self.revision = revision;
    }
}
