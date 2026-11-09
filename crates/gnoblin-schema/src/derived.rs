//! Events the host raises from a property change, not from a signal the compositor sends.
//!
//! A derived event is declared in `crates/gnoblin-lua/derived_events.toml`: the host
//! raises `signal` on `object` when `property` becomes `when`, with the fields the
//! entry lists. [`Catalog::with_derived`] adds each one to the catalog as an ordinary
//! signal, so the generated bindings, the stub and the docs-parity gate treat it like
//! any schema signal, and the host reads the same list to know when to raise it.

use std::path::Path;

use serde::Deserialize;

use crate::Version;
use crate::error::SchemaError;
use crate::model::{Catalog, Field, Signal, Type};

/// The namespace derived events belong to: the compositor's own.
pub const NAMESPACE: &str = "gnoblin";

/// One event the host raises when a property of an object changes to a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedEvent {
    /// The object type that raises the event.
    pub object: String,
    /// The signal name, as scripts subscribe to it.
    pub signal: String,
    /// The boolean property whose change raises the event.
    pub property: String,
    /// The value the property must change to for the event to fire.
    pub when: bool,
    /// The version that added the event.
    pub since: Version,
    /// The payload, in declaration order.
    pub fields: Vec<DerivedField>,
}

/// One field of a derived event's payload and where its value comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedField {
    /// The field name.
    pub name: String,
    /// The field type.
    pub ty: Type,
    /// The value the host copies into the field.
    pub from: DerivedSource,
}

/// Where a derived field's value is read from when the host raises the event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivedSource<S = String> {
    /// The object that raised the event, as an object reference.
    Object,
    /// The value of one of the object's properties.
    Property(S),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    derived: Vec<RawDerivedEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDerivedEvent {
    object: String,
    signal: String,
    property: String,
    when: bool,
    since: String,
    #[serde(default)]
    field: Vec<RawField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    from: String,
}

/// Read the derived events declared in `path`.
///
/// # Errors
///
/// Fails when the file cannot be read, is not in the expected shape, or names a type,
/// version or source that does not parse. Every problem in the file is reported.
pub fn load(path: &Path) -> Result<Vec<DerivedEvent>, SchemaError> {
    let source = path.display().to_string();
    let text = std::fs::read_to_string(path).map_err(|source| SchemaError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file: RawFile = toml::from_str(&text).map_err(|error| SchemaError::Toml {
        source: source.clone(),
        message: error.to_string(),
    })?;
    let mut problems = Vec::new();
    let mut events = Vec::new();
    for raw in file.derived {
        let label = format!("derived {}.{}", raw.object, raw.signal);
        let since = match raw.since.parse::<Version>() {
            Ok(since) => Some(since),
            Err(error) => {
                problems.push(format!("{label}: `since`: {error}"));
                None
            }
        };
        let mut fields = Vec::new();
        for field in raw.field {
            match Type::parse(&field.ty) {
                Ok(ty) => {
                    let from = if field.from == "object" {
                        DerivedSource::Object
                    } else {
                        DerivedSource::Property(field.from)
                    };
                    fields.push(DerivedField {
                        name: field.name,
                        ty,
                        from,
                    });
                }
                Err(reason) => problems.push(format!("{label}: field `{}`: {reason}", field.name)),
            }
        }
        if let Some(since) = since {
            events.push(DerivedEvent {
                object: raw.object,
                signal: raw.signal,
                property: raw.property,
                when: raw.when,
                since,
                fields,
            });
        }
    }
    if problems.is_empty() {
        Ok(events)
    } else {
        Err(SchemaError::Invalid { source, problems })
    }
}

impl Catalog {
    /// A copy of this catalog with each derived event added as a signal of its object.
    ///
    /// # Errors
    ///
    /// Fails when an event names an object or property the catalog does not declare,
    /// when the property is not a boolean, or when the object already declares the signal.
    pub fn with_derived(&self, events: &[DerivedEvent]) -> Result<Catalog, SchemaError> {
        let mut schemas = self.declared().to_vec();
        let mut problems = Vec::new();
        for event in events {
            let label = format!("derived {}.{}", event.object, event.signal);
            let Some(object) = schemas
                .iter_mut()
                .filter(|schema| schema.namespace.as_str() == NAMESPACE)
                .flat_map(|schema| schema.objects.iter_mut())
                .find(|object| object.name == event.object)
            else {
                problems.push(format!(
                    "{label}: no object `{}` in the `{NAMESPACE}` namespace",
                    event.object
                ));
                continue;
            };
            match object
                .properties
                .iter()
                .find(|property| property.name == event.property)
            {
                Some(property) if property.ty == Type::Bool => {}
                Some(_) => {
                    problems.push(format!("{label}: property `{}` is not a bool", event.property));
                    continue;
                }
                None => {
                    problems.push(format!("{label}: no property `{}`", event.property));
                    continue;
                }
            }
            if object.signals.iter().any(|signal| signal.name == event.signal) {
                problems.push(format!("{label}: the object already declares this signal"));
                continue;
            }
            object.signals.push(Signal {
                name: event.signal.clone(),
                since: event.since,
                secret: false,
                fields: event
                    .fields
                    .iter()
                    .map(|field| Field {
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                        since: event.since,
                        secret: false,
                    })
                    .collect(),
            });
        }
        if !problems.is_empty() {
            return Err(SchemaError::Invalid {
                source: "derived events".to_owned(),
                problems,
            });
        }
        Catalog::new(schemas)
    }
}
