//! The module's own copy of the object graph.
//!
//! A module reads properties out of this mirror, never out of the compositor: the
//! subscription delivers a snapshot once and every later observation once, so the
//! mirror is exactly the graph's state after the last delivery, and a getter that
//! went back over the connection would both block and see a different revision from
//! the event that made the module look (the same rule the Lua runtime's host
//! follows).
//!
//! The mirror is also what the capability check is made against. `open-window` asks
//! the mirror whether that identity is a window this module's subscription covers,
//! so a handle cannot be invented for an object the module was never told about.

use std::collections::BTreeMap;

use gnoblin_model::{Fields, ObjectId, ObjectView, Observation, Snapshot, TypeKey, Value};

/// One object as the module's mirror holds it.
#[derive(Clone, Debug)]
pub(crate) struct MirrorObject {
    ty: TypeKey,
    properties: Fields,
}

impl MirrorObject {
    /// The object's type, which is what a capability names.
    pub(crate) fn ty(&self) -> &str {
        self.ty.name()
    }

    /// One property, if the module's copy has it.
    pub(crate) fn property(&self, name: &str) -> Option<&Value> {
        self.properties.get(name)
    }
}

/// Every object the module's subscription covers, by identity.
#[derive(Debug, Default)]
pub(crate) struct Mirror {
    objects: BTreeMap<ObjectId, MirrorObject>,
}

impl Mirror {
    /// Start from the snapshot a subscription hands out once.
    pub(crate) fn seed(snapshot: &Snapshot) -> Mirror {
        let mut mirror = Mirror::default();
        for view in snapshot.objects() {
            mirror.opened(view);
        }
        mirror
    }

    /// Apply one observation.
    ///
    /// The graph delivers observations in revision order and exactly once, so this
    /// is a plain fold: no revision bookkeeping, no deduplication, and a closed
    /// object's identity is never reused, so removing it cannot lose a later one.
    pub(crate) fn apply(&mut self, observation: &Observation) {
        match observation {
            Observation::Opened { view, .. } => self.opened(view),
            Observation::Changed {
                object,
                property,
                value,
                ..
            } => {
                if let Some(entry) = self.objects.get_mut(object) {
                    entry.properties.insert(property.clone(), value.clone());
                }
            }
            Observation::Closed { object, .. } => {
                self.objects.remove(object);
            }
            // A signal carries no state and a completion changes none: the graph is
            // unchanged by both, so neither touches the mirror.
            Observation::Signalled { .. } | Observation::Finished { .. } => {}
        }
    }

    /// One object.
    pub(crate) fn get(&self, id: ObjectId) -> Option<&MirrorObject> {
        self.objects.get(&id)
    }

    /// Add one object from a view the graph published.
    fn opened(&mut self, view: &ObjectView) {
        self.objects.insert(
            view.id(),
            MirrorObject {
                ty: view.ty().clone(),
                properties: view.properties().clone(),
            },
        );
    }
}
