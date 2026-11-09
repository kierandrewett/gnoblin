//! Resolving `base`: an object's members are its base's members, then its own.
//!
//! A namespace may span several files, so inheritance is a catalog step, run by
//! [`crate::Catalog::new`] after object references are checked, and not a per-file
//! one. The result replaces each object's member lists in place, so every reader
//! (the generators, the Lua binding, the golden) sees the resolved object and none
//! has to walk the chain itself.
//!
//! The rules, checked here:
//!
//! - a `base` names a class of the same namespace;
//! - the chain of bases has no cycle, and a class is not its own base;
//! - a derived property may redeclare an inherited property only with the same
//!   type, and then it replaces the inherited one where it sits;
//! - any other member may not share a name with an inherited member;
//! - a derived class is secret when its base is.

use std::collections::BTreeMap;

use crate::model::{Method, ObjectType, Property, Schema, Signal};

/// An object's members after inheritance.
#[derive(Clone, Debug)]
struct Members {
    secret: bool,
    properties: Vec<Property>,
    methods: Vec<Method>,
    signals: Vec<Signal>,
}

/// An object is named by its namespace and its name: a base is looked up within
/// the namespace of the class that names it.
type Key = (String, String);

struct Resolver<'a> {
    objects: BTreeMap<Key, &'a ObjectType>,
    /// `None` marks an object whose chain already failed; the problem was reported
    /// when the failure happened, so it is not repeated for each dependant.
    resolved: BTreeMap<Key, Option<Members>>,
    stack: Vec<Key>,
    problems: Vec<String>,
}

/// Resolve every object's members in place.
///
/// On failure nothing is changed and every problem is returned.
pub(crate) fn resolve(schemas: &mut [Schema]) -> Result<(), Vec<String>> {
    let (resolved, problems) = {
        let mut resolver = Resolver::new(&*schemas);
        let mut resolved = Vec::new();
        for (schema_index, schema) in schemas.iter().enumerate() {
            for (object_index, object) in schema.objects.iter().enumerate() {
                let key = (schema.namespace.as_str().to_owned(), object.name.clone());
                if let Some(members) = resolver.resolve(&key) {
                    resolved.push((schema_index, object_index, members));
                }
            }
        }
        (resolved, resolver.problems)
    };
    if !problems.is_empty() {
        return Err(problems);
    }
    for (schema_index, object_index, members) in resolved {
        let object = &mut schemas[schema_index].objects[object_index];
        object.secret = members.secret;
        object.properties = members.properties;
        object.methods = members.methods;
        object.signals = members.signals;
    }
    Ok(())
}

impl<'a> Resolver<'a> {
    fn new(schemas: &'a [Schema]) -> Resolver<'a> {
        let objects = schemas
            .iter()
            .flat_map(|schema| {
                schema
                    .objects
                    .iter()
                    .map(move |object| ((schema.namespace.as_str().to_owned(), object.name.clone()), object))
            })
            .collect();
        Resolver {
            objects,
            resolved: BTreeMap::new(),
            stack: Vec::new(),
            problems: Vec::new(),
        }
    }

    fn resolve(&mut self, key: &Key) -> Option<Members> {
        if let Some(done) = self.resolved.get(key) {
            return done.clone();
        }
        if let Some(start) = self.stack.iter().position(|open| open == key) {
            let chain: Vec<&str> = self.stack[start..]
                .iter()
                .chain(std::iter::once(key))
                .map(|(_, name)| name.as_str())
                .collect();
            self.problems
                .push(format!("base cycle in namespace `{}`: {}", key.0, chain.join(" -> ")));
            return None;
        }
        let object = *self.objects.get(key)?;
        self.stack.push(key.clone());
        let members = self.compose(&key.0, object);
        self.stack.pop();
        self.resolved.insert(key.clone(), members.clone());
        members
    }

    fn compose(&mut self, namespace: &str, object: &ObjectType) -> Option<Members> {
        let mut members = Members {
            secret: object.secret,
            properties: Vec::new(),
            methods: Vec::new(),
            signals: Vec::new(),
        };
        let base = object.base.as_deref().unwrap_or("");
        if let Some(base) = &object.base {
            let base_key = (namespace.to_owned(), base.clone());
            if !self.objects.contains_key(&base_key) {
                self.problems.push(format!(
                    "object `{}` has base `{base}`, which namespace `{namespace}` does not declare",
                    object.name
                ));
                return None;
            }
            let inherited = self.resolve(&base_key)?;
            members = Members {
                secret: inherited.secret || object.secret,
                ..inherited
            };
        }

        // Names are checked against what the base supplied, not against the object's own
        // members: a property and a signal may share a name, as they always could.
        let inherited = members.clone();
        let mut failed = false;
        for property in &object.properties {
            match members
                .properties
                .iter()
                .position(|inherited| inherited.name == property.name)
            {
                Some(index) if members.properties[index].ty == property.ty => {
                    members.properties[index] = property.clone();
                }
                Some(index) => {
                    let inherited = &members.properties[index].ty;
                    self.problems.push(format!(
                        "object `{}` redeclares property `{}` as {} but base `{base}` declares it as {inherited}",
                        object.name, property.name, property.ty
                    ));
                    failed = true;
                }
                None if taken(&inherited, &property.name) => {
                    self.problems.push(format!(
                        "object `{}` declares property `{}`, but base `{base}` already has a member of that name",
                        object.name, property.name
                    ));
                    failed = true;
                }
                None => members.properties.push(property.clone()),
            }
        }
        for method in &object.methods {
            if taken(&inherited, &method.name) {
                self.problems.push(format!(
                    "object `{}` declares method `{}`, but base `{base}` already has a member of that name",
                    object.name, method.name
                ));
                failed = true;
            } else {
                members.methods.push(method.clone());
            }
        }
        for signal in &object.signals {
            if taken(&inherited, &signal.name) {
                self.problems.push(format!(
                    "object `{}` declares signal `{}`, but base `{base}` already has a member of that name",
                    object.name, signal.name
                ));
                failed = true;
            } else {
                members.signals.push(signal.clone());
            }
        }

        if failed { None } else { Some(members) }
    }
}

/// True when a property, method or signal of this name is already on the object.
fn taken(members: &Members, name: &str) -> bool {
    members.properties.iter().any(|property| property.name == name)
        || members.methods.iter().any(|method| method.name == name)
        || members.signals.iter().any(|signal| signal.name == name)
}
