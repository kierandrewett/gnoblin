//! Reading the TOML schema format into the model, and checking every rule.
//!
//! ## Why TOML
//!
//! A schema file is a small tree of tables, and TOML writes that tree without a
//! parser of our own: `[[object]]`, then `[[object.property]]` beneath it, and so
//! on. Three reasons picked it over the alternatives:
//!
//! - comments. Every member exists for a reason, and the reason belongs beside
//!   it. JSON cannot carry one, so a JSON schema turns into an undocumented one.
//! - one spelling in the tree. Gnoblin already writes configuration as TOML, so
//!   contributors and documentation do not learn a second file syntax.
//! - a small, audited parser. A bespoke schema language would need its own
//!   parser, error messages and editor support for no gain.
//!
//! ## Rules
//!
//! A file declares one namespace at one version. Every object, member and field
//! records `since`, the version that added it. The parser fails, listing every
//! problem, when:
//!
//! - two objects, members or fields share a name (`since` cannot tell them
//!   apart);
//! - a `since` is missing, is not three numbers, or is newer than the schema
//!   version;
//! - a name does not match its pattern, or a type names nothing known;
//! - a field's `since` is earlier than the member that introduced it.
//!
//! Object references are resolved by [`crate::Catalog`], because a namespace may
//! span several files.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::error::SchemaError;
use crate::model::{
    ErrorDecl, Field, Method, Namespace, ObjectType, Property, Schema, Signal, Type, is_error_code, is_lower_name,
};
use gnoblin_types::Version;

/// Shape of one schema file, exactly as `toml` deserializes it.
///
/// Unknown keys are rejected: a typo like `propertys` would otherwise produce a
/// schema that silently lacks a member, and the generated code would silently
/// lack a field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSchema {
    namespace: String,
    version: String,
    // Schema files name these arrays in the singular, like the other tables.
    #[serde(default, rename = "object")]
    objects: Vec<RawObject>,
    #[serde(default, rename = "error")]
    errors: Vec<RawError>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawObject {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, rename = "property")]
    properties: Vec<RawProperty>,
    #[serde(default, rename = "method")]
    methods: Vec<RawMethod>,
    #[serde(default, rename = "signal")]
    signals: Vec<RawSignal>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProperty {
    name: String,
    #[serde(rename = "type")]
    ty: Option<String>,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMethod {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, rename = "param")]
    params: Vec<RawField>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignal {
    name: String,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, rename = "field")]
    fields: Vec<RawField>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    name: String,
    #[serde(rename = "type")]
    ty: Option<String>,
    since: Option<String>,
    #[serde(default)]
    secret: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawError {
    name: String,
    since: Option<String>,
}

/// Parse and validate one schema file.
///
/// `origin` is the file name or path used in messages and in generated output.
pub fn parse_schema(origin: &str, text: &str) -> Result<Schema, SchemaError> {
    let raw: RawSchema = toml::from_str(text).map_err(|error| SchemaError::Toml {
        source: origin.to_owned(),
        message: error.to_string(),
    })?;

    let mut problems = Vec::new();
    let version = match Version::parse(&raw.version) {
        Ok(version) => Some(version),
        Err(error) => {
            problems.push(format!("schema version: {error}"));
            None
        }
    };
    let namespace = match Namespace::parse(&raw.namespace) {
        Ok(namespace) => Some(namespace),
        Err(reason) => {
            problems.push(format!("namespace: {reason}"));
            None
        }
    };

    let errors = parse_errors(&raw.errors, version, &mut problems);
    let objects = parse_objects(&raw.objects, version, &mut problems);

    if !problems.is_empty() {
        return Err(SchemaError::Invalid {
            source: origin.to_owned(),
            problems,
        });
    }
    match (namespace, version) {
        (Some(namespace), Some(version)) => Ok(Schema {
            origin: origin.to_owned(),
            namespace,
            version,
            objects,
            errors,
        }),
        // Unreachable in practice: a bad namespace or version already pushed a
        // problem, so the branch above returned. Reported rather than panicked so
        // a future edit cannot turn a schema mistake into a crash.
        _ => Err(SchemaError::Invalid {
            source: origin.to_owned(),
            problems: vec!["a schema needs a namespace and a version".to_owned()],
        }),
    }
}

fn parse_errors(raw_errors: &[RawError], version: Option<Version>, problems: &mut Vec<String>) -> Vec<ErrorDecl> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_error in raw_errors {
        let owner = format!("error code `{}`", raw_error.name);
        if !is_error_code(&raw_error.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9-]*"));
        }
        if !seen.insert(raw_error.name.as_str()) {
            problems.push(format!("duplicate {owner}"));
        }
        let Some(since) = since_of(problems, raw_error.since.as_deref(), &owner, version) else {
            continue;
        };
        errors.push(ErrorDecl {
            name: raw_error.name.clone(),
            since,
        });
    }
    errors
}

fn parse_objects(raw_objects: &[RawObject], version: Option<Version>, problems: &mut Vec<String>) -> Vec<ObjectType> {
    let mut objects = Vec::new();
    let mut seen_objects = BTreeSet::new();
    for raw_object in raw_objects {
        let owner = format!("object `{}`", raw_object.name);
        if !is_lower_name(&raw_object.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        if !seen_objects.insert(raw_object.name.as_str()) {
            problems.push(format!("duplicate {owner}"));
        }
        let Some(since) = since_of(problems, raw_object.since.as_deref(), &owner, version) else {
            continue;
        };

        // One namespace of member names inside an object: a dump, a generated
        // binding and a subscriber all address `object.member`, so a property and
        // a method may not share a name.
        let mut members = BTreeSet::new();
        let properties = parse_properties(&raw_object.properties, &owner, since, version, &mut members, problems);
        let methods = parse_methods(&raw_object.methods, &owner, since, version, &mut members, problems);
        let signals = parse_signals(&raw_object.signals, &owner, since, version, &mut members, problems);

        objects.push(ObjectType {
            name: raw_object.name.clone(),
            since,
            secret: raw_object.secret,
            properties,
            methods,
            signals,
        });
    }
    objects
}

fn parse_properties(
    raw_properties: &[RawProperty],
    object: &str,
    object_since: Version,
    version: Option<Version>,
    members: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Vec<Property> {
    let mut properties = Vec::new();
    for raw_property in raw_properties {
        let owner = format!("property `{}` of {object}", raw_property.name);
        if !members.insert(raw_property.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_property.name));
        }
        if !is_lower_name(&raw_property.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(ty) = type_of(problems, raw_property.ty.as_deref(), &owner) else {
            continue;
        };
        let Some(since) = member_since(problems, raw_property.since.as_deref(), &owner, object_since, version) else {
            continue;
        };
        properties.push(Property {
            name: raw_property.name.clone(),
            ty,
            since,
            secret: raw_property.secret,
        });
    }
    properties
}

fn parse_methods(
    raw_methods: &[RawMethod],
    object: &str,
    object_since: Version,
    version: Option<Version>,
    members: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Vec<Method> {
    let mut methods = Vec::new();
    for raw_method in raw_methods {
        let owner = format!("method `{}` of {object}", raw_method.name);
        if !members.insert(raw_method.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_method.name));
        }
        if !is_lower_name(&raw_method.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(since) = member_since(problems, raw_method.since.as_deref(), &owner, object_since, version) else {
            continue;
        };
        let params = parse_fields(&raw_method.params, "parameter", &owner, since, version, problems);
        methods.push(Method {
            name: raw_method.name.clone(),
            since,
            secret: raw_method.secret,
            params,
        });
    }
    methods
}

fn parse_signals(
    raw_signals: &[RawSignal],
    object: &str,
    object_since: Version,
    version: Option<Version>,
    members: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) -> Vec<Signal> {
    let mut signals = Vec::new();
    for raw_signal in raw_signals {
        let owner = format!("signal `{}` of {object}", raw_signal.name);
        if !members.insert(raw_signal.name.clone()) {
            problems.push(format!("{object}: duplicate member name `{}`", raw_signal.name));
        }
        if !is_lower_name(&raw_signal.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(since) = member_since(problems, raw_signal.since.as_deref(), &owner, object_since, version) else {
            continue;
        };
        let fields = parse_fields(&raw_signal.fields, "field", &owner, since, version, problems);
        signals.push(Signal {
            name: raw_signal.name.clone(),
            since,
            secret: raw_signal.secret,
            fields,
        });
    }
    signals
}

fn parse_fields(
    raw_fields: &[RawField],
    role: &str,
    member: &str,
    member_version: Version,
    version: Option<Version>,
    problems: &mut Vec<String>,
) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();
    for raw_field in raw_fields {
        let owner = format!("{role} `{}` of {member}", raw_field.name);
        if !seen.insert(raw_field.name.as_str()) {
            problems.push(format!("{member}: duplicate {role} `{}`", raw_field.name));
        }
        if !is_lower_name(&raw_field.name) {
            problems.push(format!("{owner}: name must match [a-z][a-z0-9_]*"));
        }
        let Some(ty) = type_of(problems, raw_field.ty.as_deref(), &owner) else {
            continue;
        };
        // A field without `since` was added with the member that carries it.
        let since = match raw_field.since.as_deref() {
            None => member_version,
            Some(text) => {
                let Some(since) = member_since(problems, Some(text), &owner, member_version, version) else {
                    continue;
                };
                since
            }
        };
        fields.push(Field {
            name: raw_field.name.clone(),
            ty,
            since,
            secret: raw_field.secret,
        });
    }
    fields
}

fn type_of(problems: &mut Vec<String>, raw: Option<&str>, owner: &str) -> Option<Type> {
    let Some(raw) = raw else {
        problems.push(format!("{owner}: missing `type`"));
        return None;
    };
    match Type::parse(raw) {
        Ok(ty) => Some(ty),
        Err(reason) => {
            problems.push(format!("{owner}: {reason}"));
            None
        }
    }
}

/// Read a `since`, check it is a full version, and check it is not from the
/// future.
fn since_of(problems: &mut Vec<String>, raw: Option<&str>, owner: &str, version: Option<Version>) -> Option<Version> {
    let Some(raw) = raw else {
        problems.push(format!("{owner}: missing `since`"));
        return None;
    };
    let since = match Version::parse(raw) {
        Ok(since) => since,
        Err(error) => {
            problems.push(format!("{owner}: {error}"));
            return None;
        }
    };
    if let Some(version) = version
        && since > version
    {
        problems.push(format!(
            "{owner}: `since` {since} is newer than the schema version {version}"
        ));
        return None;
    }
    Some(since)
}

/// A member's `since`, which may not predate the object that contains it.
fn member_since(
    problems: &mut Vec<String>,
    raw: Option<&str>,
    owner: &str,
    object_since: Version,
    version: Option<Version>,
) -> Option<Version> {
    let since = since_of(problems, raw, owner, version)?;
    if since < object_since {
        problems.push(format!(
            "{owner}: `since` {since} is earlier than its object's `since` {object_since}"
        ));
        return None;
    }
    Some(since)
}

#[cfg(test)]
mod tests {
    use crate::Catalog;
    use crate::model::Type;
    use crate::parse::parse_schema;

    /// A minimal valid file, with one placeholder that a test can replace.
    fn schema_text(members: &str) -> String {
        format!(
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\n{members}"
        )
    }

    fn problems(members: &str) -> Vec<String> {
        match parse_schema("test.toml", &schema_text(members)) {
            Ok(schema) => panic!("expected a rejection, parsed {schema:?}"),
            Err(crate::SchemaError::Invalid { problems, .. }) => problems,
            Err(other) => panic!("expected validation problems, got {other}"),
        }
    }

    #[test]
    fn parses_a_minimal_schema() {
        let schema = parse_schema(
            "test.toml",
            &schema_text(
                "\n[[object.property]]\nname = \"version\"\ntype = \"string\"\nsince = \"0.2.0\"\n\
                 \n[[object.method]]\nname = \"refresh\"\nsince = \"0.2.0\"\n\
                 \n[[object.signal]]\nname = \"ready\"\nsince = \"0.2.0\"\n",
            ),
        )
        .expect("schema");
        assert_eq!(schema.namespace.as_str(), "test");
        assert_eq!(schema.version.to_string(), "0.2.0");
        assert_eq!(schema.objects.len(), 1);
        let object = &schema.objects[0];
        assert_eq!(object.name, "thing");
        assert_eq!(object.properties[0].ty, Type::Text);
        assert_eq!(object.methods[0].name, "refresh");
        assert!(!object.methods[0].is_sensitive());
        assert_eq!(object.signals[0].name, "ready");
    }

    #[test]
    fn a_secret_field_makes_the_member_sensitive() {
        let schema = parse_schema(
            "test.toml",
            &schema_text(
                "\n[[object.method]]\nname = \"auth_response\"\nsince = \"0.2.0\"\n\
                 \n[[object.method.param]]\nname = \"response\"\ntype = \"string\"\nsecret = true\n",
            ),
        )
        .expect("schema");
        let method = &schema.objects[0].methods[0];
        assert!(method.is_sensitive());
        assert!(schema.objects[0].is_sensitive());
        // The parameter inherits the member's `since` when it omits one.
        assert_eq!(method.params[0].since.to_string(), "0.2.0");
    }

    #[test]
    fn a_field_may_not_predate_its_member() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.1.0\"\n");
        assert!(
            found.iter().any(|problem| problem.contains("earlier than its object")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_duplicate_member_names() {
        let found = problems(
            "\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.2.0\"\n\
             \n[[object.method]]\nname = \"title\"\nsince = \"0.2.0\"\n",
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("duplicate member name `title`")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_missing_since() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("property `title` of object `thing`: missing `since`")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_version_that_is_not_three_numbers() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.2\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("`0.2` is not a MAJOR.MINOR.PATCH")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_a_since_from_the_future() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"string\"\nsince = \"0.3.0\"\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("newer than the schema version 0.2.0")),
            "{found:?}"
        );
    }

    #[test]
    fn rejects_an_unknown_type_and_key() {
        let found = problems("\n[[object.property]]\nname = \"title\"\ntype = \"text\"\nsince = \"0.2.0\"\n");
        assert!(
            found.iter().any(|problem| problem.contains("unknown type `text`")),
            "{found:?}"
        );

        let error = parse_schema(
            "test.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"thing\"\nsince = \"0.2.0\"\npropertys = []\n",
        )
        .expect_err("unknown key");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn rejects_a_reference_to_a_missing_object() {
        let schema = parse_schema(
            "test.toml",
            &schema_text("\n[[object.property]]\nname = \"parent\"\ntype = \"object:group\"\nsince = \"0.2.0\"\n"),
        )
        .expect("schema parses; references are a catalog check");
        let error = Catalog::new(vec![schema]).expect_err("unresolved reference");
        assert!(error.to_string().contains("object `group`"), "{error}");
    }

    #[test]
    fn resolves_a_reference_across_files_of_one_namespace() {
        let window = parse_schema(
            "window.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"window\"\nsince = \"0.2.0\"\n",
        )
        .expect("window");
        let group = parse_schema(
            "group.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"group\"\nsince = \"0.2.0\"\n\
             \n[[object.property]]\nname = \"active\"\ntype = \"object:window\"\nsince = \"0.2.0\"\n",
        )
        .expect("group");
        let catalog = Catalog::new(vec![window, group]).expect("catalog");
        let namespaces = catalog.namespaces();
        assert_eq!(namespaces[0].as_str(), "test");
        assert_eq!(catalog.objects(&namespaces[0]).count(), 2);
    }

    #[test]
    fn rejects_two_versions_for_one_namespace() {
        let older = parse_schema(
            "older.toml",
            "namespace = \"test\"\nversion = \"0.2.0\"\n\n[[object]]\nname = \"window\"\nsince = \"0.2.0\"\n",
        )
        .expect("older");
        let newer = parse_schema(
            "newer.toml",
            "namespace = \"test\"\nversion = \"0.3.0\"\n\n[[object]]\nname = \"group\"\nsince = \"0.3.0\"\n",
        )
        .expect("newer");
        let error = Catalog::new(vec![older, newer]).expect_err("version disagreement");
        assert!(error.to_string().contains("is version 0.2.0"), "{error}");
    }

    #[test]
    fn the_checked_in_schemas_parse_and_validate() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let catalog = Catalog::load_dir(&dir).expect("checked-in schema files");
        let namespaces: Vec<String> = catalog.namespaces().iter().map(ToString::to_string).collect();
        assert_eq!(namespaces, vec!["gnoblin".to_owned()]);
        let gnoblin = catalog.namespaces()[0].clone();
        assert_eq!(catalog.version(&gnoblin).expect("version").to_string(), "0.2.0");
        let objects: Vec<&str> = catalog.objects(&gnoblin).map(|object| object.name.as_str()).collect();
        assert_eq!(
            objects,
            vec![
                "capture",
                "effect",
                "eis",
                "idle",
                "lock",
                "output",
                "portal_ui",
                "portal_outcome",
                "request",
                "session",
                "settings",
                "store",
                "window",
                "workspace",
            ]
        );
    }

    #[test]
    fn the_example_extension_fixture_parses_outside_the_stock_catalog() {
        let text = include_str!("../tests/fixtures/extensions/example-ext.toml");
        let schema = super::parse_schema("example-ext.toml", text).expect("fixture parses");
        assert_eq!(schema.namespace.as_str(), "example.ext");
        assert_eq!(schema.objects.len(), 2);
        let catalog = Catalog::new(vec![schema]).expect("fixture validates alone");
        assert_eq!(catalog.namespaces().len(), 1);
    }
}
