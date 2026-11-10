//! The property sets of schema objects, built from the schema.
//!
//! Every fixture that creates an object has to supply every property the schema
//! requires, and a hand-written list goes stale the moment the schema grows: a new
//! property then fails every fixture at once. These helpers read an object's
//! properties from the same schema the compositor is generated from and fill each one
//! with its type's default, so a fixture names only the values it cares about.

use std::path::Path;
use std::sync::LazyLock;

use gnoblin_model::{Fields, Value};
use gnoblin_schema::{Catalog, Namespace, Type};

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../gnoblin-schema/schema"));
    Catalog::load_dir_except(dir, &["tree"]).expect("the schema directory loads")
});

/// The default for a property of type `ty`, or [`None`] for an object reference, which
/// has no meaningful default and must be supplied by the fixture.
fn default_for(ty: &Type) -> Option<Value> {
    Some(match ty {
        Type::Bool => Value::Bool(false),
        Type::Int => Value::Int(0),
        Type::Uint => Value::Uint(0),
        Type::Double => Value::Double(0.0),
        Type::Text => Value::Text(String::new()),
        Type::Bytes => Value::Bytes(Vec::new()),
        Type::List(_) => Value::List(Vec::new()),
        Type::ObjectRef(_) => return None,
    })
}

/// The full property set of the `gnoblin` object `object`: every property the schema
/// declares, each at its type's default, with `overrides` applied on top.
///
/// # Panics
///
/// Panics when the schema does not declare `object`, when an override names a property
/// the object does not declare, when an override does not fit its declared type, or
/// when the schema declares an object reference the caller did not supply. A fixture
/// therefore cannot drift from the schema silently.
#[must_use]
pub fn object_fields(object: &str, overrides: &[(&str, Value)]) -> Fields {
    let namespace = Namespace::parse("gnoblin").expect("the namespace parses");
    let declared = CATALOG
        .objects(&namespace)
        .find(|candidate| candidate.name == object)
        .unwrap_or_else(|| panic!("the schema declares gnoblin::{object}"));

    for (name, _) in overrides {
        assert!(
            declared.properties.iter().any(|property| property.name == *name),
            "gnoblin::{object} has no property `{name}`"
        );
    }

    let mut fields = Fields::new();
    for property in &declared.properties {
        let value = overrides
            .iter()
            .find(|(name, _)| *name == property.name)
            .map(|(_, value)| value.clone())
            .or_else(|| default_for(&property.ty))
            .unwrap_or_else(|| {
                panic!(
                    "gnoblin::{object}.{} is an object reference: the fixture must supply it",
                    property.name
                )
            });
        assert!(
            value.fits(&property.ty),
            "gnoblin::{object}.{} does not take the value supplied for it",
            property.name
        );
        fields.insert(property.name.clone(), value);
    }
    fields
}

/// The full property set of a `gnoblin::window`; see [`object_fields`].
#[must_use]
pub fn window_fields(overrides: &[(&str, Value)]) -> Fields {
    object_fields("window", overrides)
}
